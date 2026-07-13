//! In-memory aggregation for the per-spec access counter (#944).
//!
//! The counter (#549) is aggregate analytics — nobody needs each hit
//! durably persisted the instant it happens. The first cut awaited an
//! UPSERT inline (a DB round-trip on every API response, serializing on
//! SQLite's single writer); #744 moved it to a detached `tokio::spawn`
//! per request, which only relocated the cost: task rate and write rate
//! still equaled the request rate, now without backpressure or
//! supervision.
//!
//! This module completes the fix. The hot path is a plain in-memory
//! increment on a `(spec_id, day)` bucket — no await, no task, no DB.
//! One supervised task per process drains the buffer every
//! [`FLUSH_INTERVAL`], writing a single `count = count + delta` UPSERT
//! per touched bucket. Ten thousand API calls in a window become one
//! write, and a DB hiccup merges the deltas back into the buffer to
//! retry with exponential backoff — nothing is silently lost while the
//! process lives.
//!
//! Memory stays bounded structurally: keys are `(spec, day)` pairs, so
//! the buffer grows with the catalog and the outage length in days, not
//! with traffic. [`MAX_PENDING_KEYS`] is a backstop far above any real
//! catalog; increments past it on *new* buckets are counted in
//! [`AccessCounter::dropped`] rather than growing the map.

use crate::db::{self, ConfigDb};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// How often the drain task writes pending deltas to the DB. Counts
/// surface in the admin at most this much later — fine for analytics.
pub const FLUSH_INTERVAL: Duration = Duration::from_secs(2);

/// Ceiling for the failure backoff: 2 s → 4 s → … → capped here.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Backstop on distinct `(spec, day)` buckets held in memory. Real
/// deployments count dozens of specs, so hitting this means something
/// is very wrong; we prefer dropping *new* buckets (counted in
/// `dropped`) over unbounded growth during a long DB outage.
pub const MAX_PENDING_KEYS: usize = 4096;

/// Shared in-memory buffer of not-yet-persisted access counts.
/// Cheap to clone via `Arc` in [`crate::AppState`].
#[derive(Debug, Default)]
pub struct AccessCounter {
    /// Pending deltas keyed by `(spec_id, day)`. A std `Mutex` (not
    /// tokio): the critical sections are a map insert or a `mem::take`,
    /// never an await.
    pending: Mutex<HashMap<(String, String), i64>>,
    /// Increments discarded because the buffer was at
    /// [`MAX_PENDING_KEYS`] — observability for the backstop.
    dropped: AtomicU64,
    /// Failed flush attempts since boot — observability for retries.
    flush_failures: AtomicU64,
}

impl AccessCounter {
    /// Record one access for `spec_id` in today's bucket. This is the
    /// hot path: synchronous, allocation-light, no DB. A poisoned lock
    /// is recovered rather than propagated — losing one increment to a
    /// panicking peer thread beats poisoning the counter forever.
    pub fn bump(&self, spec_id: &str) {
        let key = (spec_id.to_string(), db::spec_access::today());
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if pending.len() >= MAX_PENDING_KEYS && !pending.contains_key(&key) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        *pending.entry(key).or_insert(0) += 1;
    }

    /// Persist every pending delta with one UPSERT per bucket. Returns
    /// how many buckets were written. On a write error the failed
    /// bucket *and* everything not yet attempted are merged back into
    /// the buffer (increments that arrived meanwhile are preserved),
    /// so a retry later re-flushes the full amount.
    pub async fn flush(&self, db: &ConfigDb) -> anyhow::Result<usize> {
        let pending = std::mem::take(
            &mut *self
                .pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        if pending.is_empty() {
            return Ok(0);
        }
        let total = pending.len();
        let mut entries = pending.into_iter();
        while let Some(((spec_id, day), delta)) = entries.next() {
            if let Err(e) = db::spec_access::record_delta(db, &spec_id, &day, delta).await {
                let mut rest: HashMap<_, _> = entries.collect();
                rest.insert((spec_id, day), delta);
                self.merge_back(rest);
                self.flush_failures.fetch_add(1, Ordering::Relaxed);
                return Err(e);
            }
        }
        Ok(total)
    }

    /// Re-add deltas that failed to persist, folding them into any
    /// increments that arrived while the flush was in flight.
    fn merge_back(&self, deltas: HashMap<(String, String), i64>) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for (key, delta) in deltas {
            *pending.entry(key).or_insert(0) += delta;
        }
    }

    /// Distinct `(spec, day)` buckets waiting to be flushed.
    pub fn backlog(&self) -> usize {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Increments lost to the [`MAX_PENDING_KEYS`] backstop since boot.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Failed flush attempts since boot.
    pub fn flush_failures(&self) -> u64 {
        self.flush_failures.load(Ordering::Relaxed)
    }
}

/// Start the single per-process drain task. Flushes every
/// [`FLUSH_INTERVAL`]; on a DB error the deltas are already back in the
/// buffer (see [`AccessCounter::flush`]) and the next attempt waits
/// with exponential backoff up to [`MAX_BACKOFF`], logging the backlog
/// so an outage is visible instead of silent.
pub fn spawn(counter: Arc<AccessCounter>, db: ConfigDb) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut wait = FLUSH_INTERVAL;
        loop {
            tokio::time::sleep(wait).await;
            match counter.flush(&db).await {
                Ok(n) => {
                    if n > 0 {
                        tracing::debug!(buckets = n, "access counters flushed");
                    }
                    wait = FLUSH_INTERVAL;
                }
                Err(e) => {
                    wait = (wait * 2).min(MAX_BACKOFF);
                    tracing::warn!(
                        error = ?e,
                        backlog = counter.backlog(),
                        dropped = counter.dropped(),
                        retry_in_s = wait.as_secs(),
                        "access-counter flush failed; deltas kept for retry"
                    );
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_db() -> ConfigDb {
        ConfigDb::Sqlite(crate::db::open_memory().await.expect("open in-memory"))
    }

    #[tokio::test]
    async fn concurrent_bumps_flush_to_the_exact_total() {
        let counter = Arc::new(AccessCounter::default());
        let db = mem_db().await;

        // 10 000 increments across 20 concurrent tasks, two specs.
        let mut handles = Vec::new();
        for t in 0..20 {
            let c = counter.clone();
            handles.push(tokio::spawn(async move {
                let spec = if t % 2 == 0 { "a" } else { "b" };
                for _ in 0..500 {
                    c.bump(spec);
                }
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        // Whatever the interleaving, the buffer holds 2 buckets and the
        // flush lands the exact totals.
        assert_eq!(counter.backlog(), 2);
        assert_eq!(counter.flush(&db).await.unwrap(), 2);
        assert_eq!(counter.backlog(), 0, "flush empties the buffer");
        let totals = db::spec_access::totals(&db).await.unwrap();
        assert_eq!(totals.get("a"), Some(&5000));
        assert_eq!(totals.get("b"), Some(&5000));
        assert_eq!(counter.dropped(), 0);
    }

    #[tokio::test]
    async fn flush_failure_keeps_deltas_for_retry() {
        let counter = AccessCounter::default();
        counter.bump("a");
        counter.bump("a");
        counter.bump("b");

        // A closed pool makes every write fail — the outage case.
        let db = mem_db().await;
        if let ConfigDb::Sqlite(pool) = &db {
            pool.close().await;
        }
        assert!(counter.flush(&db).await.is_err());
        assert_eq!(
            counter.backlog(),
            2,
            "failed flush merges both buckets back"
        );
        assert_eq!(counter.flush_failures(), 1);

        // Increments arriving after the failure fold into the same
        // buckets — a later successful flush writes the full amount.
        counter.bump("a");
        let db = mem_db().await;
        assert_eq!(counter.flush(&db).await.unwrap(), 2);
        let totals = db::spec_access::totals(&db).await.unwrap();
        assert_eq!(totals.get("a"), Some(&3));
        assert_eq!(totals.get("b"), Some(&1));
    }

    #[tokio::test]
    async fn backstop_drops_new_buckets_but_keeps_counting_existing() {
        let counter = AccessCounter::default();
        // Fill the buffer to the cap with distinct synthetic specs.
        for i in 0..MAX_PENDING_KEYS {
            counter.bump(&format!("spec-{i}"));
        }
        assert_eq!(counter.backlog(), MAX_PENDING_KEYS);

        // A new bucket is dropped (and counted)…
        counter.bump("one-too-many");
        assert_eq!(counter.backlog(), MAX_PENDING_KEYS);
        assert_eq!(counter.dropped(), 1);

        // …but an existing bucket still increments.
        counter.bump("spec-0");
        assert_eq!(counter.backlog(), MAX_PENDING_KEYS);

        let db = mem_db().await;
        counter.flush(&db).await.unwrap();
        let totals = db::spec_access::totals(&db).await.unwrap();
        assert_eq!(totals.get("spec-0"), Some(&2));
        assert_eq!(totals.get("one-too-many"), None);
    }
}
