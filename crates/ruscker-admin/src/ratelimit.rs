//! Per-client, per-spec request rate limiting for API specs.
//!
//! This is the proxy-side enforcement of a spec's `api.rate-limit`
//! field (e.g. `100/min`). It complements the *global* login
//! limiter in [`crate::auth`] — that one guards a single secret and
//! is deliberately keyed by nothing; this one is about per-caller
//! fairness on a public API, so it keys by client identity.
//!
//! ## Algorithm
//!
//! A sliding window of request timestamps per `(spec, client)` key,
//! same shape as [`crate::auth::LoginRateLimiter`]. On each request
//! we prune timestamps older than the window, then allow if fewer
//! than `max` remain. Denials report a `Retry-After` computed from
//! the oldest in-window timestamp.
//!
//! ## Client identity & spoofing
//!
//! The caller passes a `client` key (see `proxy::client_key`). When
//! Ruscker runs behind a reverse proxy that the operator has opted
//! into trusting (`server.useForwardHeaders`), that key is the
//! right-most parseable `X-Forwarded-For` address (the entry the
//! trusted proxy appended — the spoof-safe choice); otherwise it's the
//! TCP peer.
//! We never trust `X-Forwarded-For` unless the operator opted in,
//! because otherwise a client could rotate the header to evade the
//! limit — the same reasoning the login limiter documents.
//!
//! ## Memory
//!
//! One [`VecDeque`] per distinct `(spec, client)` seen. Entries are
//! never emptied in place (an allowed request always re-records), so
//! an amortized sweep runs every [`SWEEP_EVERY`] checks and drops
//! every entry whose **newest** timestamp has aged past the largest
//! policy window observed — memory then tracks the set of *recently
//! active* clients rather than every client ever seen (#737: before
//! the sweep existed, a client rotating source addresses — trivial
//! within an IPv6 /64 — grew the map one permanent allocation per
//! distinct address). A spec with no `rate-limit` configured is never
//! consulted and costs nothing.

use dashmap::DashMap;
use ruscker_config::RatePolicy;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// How many `check` calls between idle-entry sweeps. Amortizes the
/// full-map `retain` to ~0.1% of requests; between sweeps the map can
/// grow by at most this many new entries beyond the active set.
const SWEEP_EVERY: u32 = 1024;

/// Outcome of a rate-limit check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateDecision {
    /// Under the limit — the request was counted and may proceed.
    Allow,
    /// Over the limit — reject with `429`. `retry_after_secs` is a
    /// conservative estimate of when a slot frees up, suitable for
    /// the `Retry-After` header (always ≥ 1).
    Deny { retry_after_secs: u64 },
}

/// Shared, lock-free-ish per-`(spec, client)` sliding-window limiter.
///
/// Cloneable cheaply via the inner `DashMap` being behind the same
/// allocation when wrapped in an `Arc` (which `AppState` does).
#[derive(Debug, Default)]
pub struct ApiRateLimiter {
    windows: DashMap<(String, String), VecDeque<Instant>>,
    /// Largest policy window observed (ms) — the sweep's idle horizon.
    /// Per-spec policies differ; sweeping with the largest one is the
    /// conservative choice (never evicts timestamps a stricter spec
    /// might still count).
    max_window_ms: AtomicU64,
    /// `check` calls since the last sweep.
    since_sweep: AtomicU32,
}

impl ApiRateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check (and, if allowed, record) one request from `client`
    /// against `spec_id` under `policy`.
    ///
    /// This both reads and mutates the window, so an allowed request
    /// is counted immediately — there's no separate "commit" step.
    pub fn check(&self, spec_id: &str, client: &str, policy: &RatePolicy) -> RateDecision {
        let now = Instant::now();
        // Bookkeeping for the idle sweep (#737): remember the widest
        // window in play and run the sweep every SWEEP_EVERY calls.
        self.max_window_ms
            .fetch_max(policy.window.as_millis() as u64, Ordering::Relaxed);
        if self.since_sweep.fetch_add(1, Ordering::Relaxed) + 1 >= SWEEP_EVERY {
            self.since_sweep.store(0, Ordering::Relaxed);
            self.sweep_idle(now);
        }
        // `entry` holds a shard lock for the duration; we never await
        // while holding it, so this stays a plain synchronous section.
        let mut entry = self
            .windows
            .entry((spec_id.to_owned(), client.to_owned()))
            .or_default();
        let q = entry.value_mut();

        // Drop timestamps that have aged out of the window.
        while q
            .front()
            .is_some_and(|t| now.duration_since(*t) > policy.window)
        {
            q.pop_front();
        }

        if (q.len() as u32) < policy.max {
            q.push_back(now);
            RateDecision::Allow
        } else {
            // The oldest in-window hit is the one that must expire
            // before a slot opens. `+1` rounds up so we never tell a
            // client to retry "in 0 seconds".
            let retry = match q.front() {
                Some(oldest) => {
                    let elapsed = now.duration_since(*oldest);
                    policy.window.saturating_sub(elapsed).as_secs() + 1
                }
                None => 1,
            };
            RateDecision::Deny {
                retry_after_secs: retry,
            }
        }
    }

    /// Drop every `(spec, client)` entry whose newest timestamp has
    /// aged past the largest observed window — its deque would prune
    /// to empty on the next touch anyway, so the entry only pins
    /// memory for a client that may never return (#737).
    fn sweep_idle(&self, now: Instant) {
        let horizon = Duration::from_millis(self.max_window_ms.load(Ordering::Relaxed));
        self.windows
            .retain(|_, q| q.back().is_some_and(|t| now.duration_since(*t) <= horizon));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn policy(max: u32, window: Duration) -> RatePolicy {
        RatePolicy { max, window }
    }

    #[test]
    fn allows_up_to_the_limit_then_denies() {
        let rl = ApiRateLimiter::new();
        let p = policy(3, Duration::from_secs(60));
        assert_eq!(rl.check("api", "1.1.1.1", &p), RateDecision::Allow);
        assert_eq!(rl.check("api", "1.1.1.1", &p), RateDecision::Allow);
        assert_eq!(rl.check("api", "1.1.1.1", &p), RateDecision::Allow);
        match rl.check("api", "1.1.1.1", &p) {
            RateDecision::Deny { retry_after_secs } => assert!(retry_after_secs >= 1),
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[test]
    fn separate_clients_have_separate_budgets() {
        let rl = ApiRateLimiter::new();
        let p = policy(1, Duration::from_secs(60));
        assert_eq!(rl.check("api", "a", &p), RateDecision::Allow);
        // Different client — own budget, still allowed.
        assert_eq!(rl.check("api", "b", &p), RateDecision::Allow);
        // First client is now over its budget of 1.
        assert!(matches!(
            rl.check("api", "a", &p),
            RateDecision::Deny { .. }
        ));
    }

    #[test]
    fn separate_specs_have_separate_budgets() {
        let rl = ApiRateLimiter::new();
        let p = policy(1, Duration::from_secs(60));
        assert_eq!(rl.check("api-a", "client", &p), RateDecision::Allow);
        // Same client, different spec — independent budget.
        assert_eq!(rl.check("api-b", "client", &p), RateDecision::Allow);
        assert!(matches!(
            rl.check("api-a", "client", &p),
            RateDecision::Deny { .. }
        ));
    }

    // #737: the sweep drops entries whose newest hit aged out of the
    // widest window, and keeps the active ones — so a client rotating
    // addresses can't grow the map without bound.
    #[test]
    fn sweep_drops_idle_clients_and_keeps_active_ones() {
        let rl = ApiRateLimiter::new();
        let p = policy(5, Duration::from_millis(10));
        rl.check("api", "idle-client", &p);
        std::thread::sleep(Duration::from_millis(25));
        rl.check("api", "active-client", &p);
        assert_eq!(rl.windows.len(), 2, "both entries present before the sweep");

        rl.sweep_idle(Instant::now());
        assert_eq!(rl.windows.len(), 1, "idle entry dropped");
        assert!(
            rl.windows.contains_key(&("api".into(), "active-client".into())),
            "the in-window client survives the sweep"
        );
    }

    // The sweep horizon is the LARGEST window seen — a stricter spec's
    // hits must not be evicted early just because a looser spec exists.
    #[test]
    fn sweep_horizon_is_the_widest_observed_window() {
        let rl = ApiRateLimiter::new();
        rl.check("strict", "c", &policy(5, Duration::from_millis(10)));
        rl.check("loose", "c", &policy(5, Duration::from_secs(60)));
        std::thread::sleep(Duration::from_millis(25));
        // Both entries' newest hits are older than the strict window but
        // inside the loose one → neither is swept.
        rl.sweep_idle(Instant::now());
        assert_eq!(rl.windows.len(), 2, "60s horizon keeps both entries");
    }

    #[test]
    fn window_slides_so_old_hits_stop_counting() {
        let rl = ApiRateLimiter::new();
        // A zero-length window means every prior hit is already
        // "older than the window" on the next call, so the budget
        // refreshes every time.
        let p = policy(1, Duration::from_millis(0));
        assert_eq!(rl.check("api", "c", &p), RateDecision::Allow);
        std::thread::sleep(Duration::from_millis(1));
        assert_eq!(
            rl.check("api", "c", &p),
            RateDecision::Allow,
            "the earlier hit should have aged out of the window"
        );
    }
}
