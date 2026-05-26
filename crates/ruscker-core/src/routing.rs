//! Routing decisions: which replica gets the next session?
//!
//! All routing strategies are pure functions of `(strategy, replicas)`
//! → `Option<ReplicaId>`. This makes them trivially testable.

use crate::replica::{Replica, ReplicaId};
use ruscker_config::RoutingStrategy;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingDecision {
    /// Use this replica.
    Use(ReplicaId),
    /// All replicas at capacity. Caller should request a scale-up.
    Saturated,
    /// No replicas exist yet. Caller should request initial spawn.
    NoReplicas,
}

pub struct Router {
    strategy: RoutingStrategy,
    round_robin_cursor: AtomicUsize,
}

impl Router {
    pub fn new(strategy: RoutingStrategy) -> Self {
        Self {
            strategy,
            round_robin_cursor: AtomicUsize::new(0),
        }
    }

    /// Pick a replica for a new session.
    pub fn pick(&self, replicas: &[Replica]) -> RoutingDecision {
        let accepting: Vec<&Replica> = replicas.iter().filter(|r| r.is_accepting()).collect();
        if accepting.is_empty() {
            return if replicas.is_empty() {
                RoutingDecision::NoReplicas
            } else {
                RoutingDecision::Saturated
            };
        }

        let chosen = match self.strategy {
            RoutingStrategy::RoundRobin => {
                let idx = self.round_robin_cursor.fetch_add(1, Ordering::Relaxed) % accepting.len();
                accepting[idx]
            }
            RoutingStrategy::LeastConnections => accepting
                .iter()
                .max_by_key(|r| r.available_seats())
                .copied()
                .unwrap(),
            RoutingStrategy::WeightedRandom => {
                // Naive: pick first for now. Real implementation uses
                // available_seats as weights. Deferred to Phase 3.
                accepting[0]
            }
            RoutingStrategy::ResourceAware => {
                // Requires live CPU/mem metrics. Deferred to Phase 5.
                accepting
                    .iter()
                    .max_by_key(|r| r.available_seats())
                    .copied()
                    .unwrap()
            }
        };

        RoutingDecision::Use(chosen.id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replica::{Replica, ReplicaId, ReplicaState};
    use chrono::Utc;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn make_replica(active: u32, max: u32) -> Replica {
        Replica {
            id: ReplicaId::new(),
            spec_id: "test".to_string(),
            container_id: "abc".to_string(),
            upstream: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080),
            state: ReplicaState::Ready,
            started_at: Utc::now(),
            sessions_active: active,
            sessions_max: max,
        }
    }

    #[test]
    fn least_connections_picks_most_free() {
        let r1 = make_replica(8, 10);
        let r2 = make_replica(3, 10);
        let r3 = make_replica(5, 10);
        let expected = r2.id.clone();
        let router = Router::new(RoutingStrategy::LeastConnections);

        let decision = router.pick(&[r1, r2, r3]);
        assert_eq!(decision, RoutingDecision::Use(expected));
    }

    #[test]
    fn returns_saturated_when_all_full() {
        let r1 = make_replica(10, 10);
        let r2 = make_replica(10, 10);
        let router = Router::new(RoutingStrategy::LeastConnections);

        assert_eq!(router.pick(&[r1, r2]), RoutingDecision::Saturated);
    }

    #[test]
    fn returns_no_replicas_when_empty() {
        let router = Router::new(RoutingStrategy::LeastConnections);
        assert_eq!(router.pick(&[]), RoutingDecision::NoReplicas);
    }

    #[test]
    fn round_robin_cycles() {
        let r1 = make_replica(0, 10);
        let r2 = make_replica(0, 10);
        let id1 = r1.id.clone();
        let id2 = r2.id.clone();
        let router = Router::new(RoutingStrategy::RoundRobin);
        let replicas = [r1, r2];

        let first = router.pick(&replicas);
        let second = router.pick(&replicas);
        let third = router.pick(&replicas);

        assert_eq!(first, RoutingDecision::Use(id1.clone()));
        assert_eq!(second, RoutingDecision::Use(id2));
        assert_eq!(third, RoutingDecision::Use(id1));
    }
}
