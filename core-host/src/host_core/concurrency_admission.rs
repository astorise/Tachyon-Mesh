use super::*;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

/// Per-process registry of routes whose admission slot is currently held.
/// Used by `NodeSingleton` mode (intra-node serialization).
#[derive(Default)]
pub(crate) struct NodeSingletonRegistry {
    held: Mutex<HashSet<String>>,
    notify: Notify,
}

impl NodeSingletonRegistry {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn try_acquire(&self, route_path: &str) -> bool {
        let mut held = self
            .held
            .lock()
            .expect("node singleton registry should not be poisoned");
        held.insert(route_path.to_owned())
    }

    fn release(&self, route_path: &str) {
        let mut held = self
            .held
            .lock()
            .expect("node singleton registry should not be poisoned");
        held.remove(route_path);
        drop(held);
        self.notify.notify_waiters();
    }

    async fn wait_then_acquire(&self, route_path: &str) {
        loop {
            if self.try_acquire(route_path) {
                return;
            }
            self.notify.notified().await;
        }
    }
}

/// Per-route admission state used by `Drain` backup mode: when an admission is
/// paused, all new invocations are rejected with 503 until resume.
#[derive(Default)]
pub(crate) struct AdmissionPauseRegistry {
    paused: Mutex<HashMap<String, ()>>,
}

impl AdmissionPauseRegistry {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn pause(&self, route_path: &str) {
        self.paused
            .lock()
            .expect("admission pause registry should not be poisoned")
            .insert(route_path.to_owned(), ());
    }

    pub(crate) fn resume(&self, route_path: &str) {
        self.paused
            .lock()
            .expect("admission pause registry should not be poisoned")
            .remove(route_path);
    }

    fn is_paused(&self, route_path: &str) -> bool {
        self.paused
            .lock()
            .expect("admission pause registry should not be poisoned")
            .contains_key(route_path)
    }
}

/// Outcome of an admission check. The guard MUST stay alive for the duration
/// of the guest invocation and be dropped to release the slot.
pub(crate) enum AdmissionOutcome {
    Pass(AdmissionGuard),
    Rejected(AdmissionRejection),
}

pub(crate) struct AdmissionGuard {
    inner: GuardKind,
}

enum GuardKind {
    NoOp,
    NodeSlot {
        registry: Arc<NodeSingletonRegistry>,
        route_path: String,
    },
    MeshLock(distributed_lock::DistributedLockGuard),
}

impl Drop for AdmissionGuard {
    fn drop(&mut self) {
        if let GuardKind::NodeSlot {
            registry,
            route_path,
        } = &self.inner
        {
            registry.release(route_path);
        }
        // MeshLock variant drops the inner DistributedLockGuard automatically,
        // which releases the lock + stops the heartbeat task.
    }
}

pub(crate) enum AdmissionRejection {
    /// HTTP 503; client SHOULD retry against the named leader node.
    NotLeader { leader_node: Option<String> },
    /// HTTP 409; another invocation holds the singleton lock.
    Conflict { reason: String },
    /// HTTP 503; route admission is paused by an in-progress drain backup.
    AdmissionPaused,
    /// HTTP 204; invocation was silently dropped per `on_conflict = "drop"`.
    Dropped,
}

/// Evaluate the route's concurrency policy and return either a guard that
/// MUST be held for the duration of the invocation, or a rejection.
///
/// - `Unrestricted` passes through.
/// - `NodeSingleton + Queue` blocks asynchronously until the slot is free.
/// - `NodeSingleton + Reject` returns Conflict immediately.
/// - `MeshSingleton` acquires a redb-backed [`distributed_lock`] keyed on
///   `route:<path>`. `Queue` polls with backoff; `Reject` returns Conflict
///   immediately; `Drop` silently discards.
/// - `MeshLeader` uses deterministic hash election against the node registry.
pub(crate) async fn check(state: &AppState, route: &IntegrityRoute) -> AdmissionOutcome {
    let policy = &route.concurrency;

    // Drain backup pauses admission for the route regardless of concurrency mode.
    if state.admission_pause.is_paused(&route.path) {
        return AdmissionOutcome::Rejected(AdmissionRejection::AdmissionPaused);
    }

    match policy.mode {
        ConcurrencyMode::Unrestricted => AdmissionOutcome::Pass(AdmissionGuard {
            inner: GuardKind::NoOp,
        }),
        ConcurrencyMode::NodeSingleton => match policy.on_conflict {
            ConflictPolicy::Queue => {
                state.node_singletons.wait_then_acquire(&route.path).await;
                AdmissionOutcome::Pass(AdmissionGuard {
                    inner: GuardKind::NodeSlot {
                        registry: Arc::clone(&state.node_singletons),
                        route_path: route.path.clone(),
                    },
                })
            }
            ConflictPolicy::Reject => {
                if state.node_singletons.try_acquire(&route.path) {
                    AdmissionOutcome::Pass(AdmissionGuard {
                        inner: GuardKind::NodeSlot {
                            registry: Arc::clone(&state.node_singletons),
                            route_path: route.path.clone(),
                        },
                    })
                } else {
                    AdmissionOutcome::Rejected(AdmissionRejection::Conflict {
                        reason: format!(
                            "route `{}` is a node-singleton and another invocation is in progress on this node",
                            route.path
                        ),
                    })
                }
            }
            ConflictPolicy::Drop => {
                if state.node_singletons.try_acquire(&route.path) {
                    AdmissionOutcome::Pass(AdmissionGuard {
                        inner: GuardKind::NodeSlot {
                            registry: Arc::clone(&state.node_singletons),
                            route_path: route.path.clone(),
                        },
                    })
                } else {
                    AdmissionOutcome::Rejected(AdmissionRejection::Dropped)
                }
            }
        },
        ConcurrencyMode::MeshSingleton => {
            let key = format!("route:{}", route.path);
            let holder = leader_election::local_node_id();
            let lease_ttl = std::time::Duration::from_millis(policy.effective_lock_ttl_ms());
            match policy.on_conflict {
                ConflictPolicy::Queue => {
                    // Cap the queue wait at lease_ttl × 6 so a stuck holder
                    // (whose lease should eventually expire) can't pin the
                    // caller indefinitely.
                    let wait_timeout = lease_ttl.saturating_mul(6);
                    match distributed_lock::acquire_with_wait(
                        &state.core_store,
                        &key,
                        &holder,
                        lease_ttl,
                        wait_timeout,
                    )
                    .await
                    {
                        Ok(Some(guard)) => AdmissionOutcome::Pass(AdmissionGuard {
                            inner: GuardKind::MeshLock(guard),
                        }),
                        Ok(None) => AdmissionOutcome::Rejected(AdmissionRejection::Conflict {
                            reason: format!(
                                "route `{}` is mesh-singleton and the lock wait exceeded {}ms",
                                route.path,
                                wait_timeout.as_millis()
                            ),
                        }),
                        Err(error) => AdmissionOutcome::Rejected(AdmissionRejection::Conflict {
                            reason: format!(
                                "route `{}` distributed lock acquire failed: {error:#}",
                                route.path
                            ),
                        }),
                    }
                }
                ConflictPolicy::Reject => {
                    match distributed_lock::try_acquire(&state.core_store, &key, &holder, lease_ttl)
                    {
                        Ok((distributed_lock::AcquireOutcome::Acquired, Some(guard))) => {
                            AdmissionOutcome::Pass(AdmissionGuard {
                                inner: GuardKind::MeshLock(guard),
                            })
                        }
                        Ok(_) => AdmissionOutcome::Rejected(AdmissionRejection::Conflict {
                            reason: format!(
                                "route `{}` mesh-singleton lock is held by another invocation",
                                route.path
                            ),
                        }),
                        Err(error) => AdmissionOutcome::Rejected(AdmissionRejection::Conflict {
                            reason: format!(
                                "route `{}` distributed lock acquire failed: {error:#}",
                                route.path
                            ),
                        }),
                    }
                }
                ConflictPolicy::Drop => {
                    match distributed_lock::try_acquire(&state.core_store, &key, &holder, lease_ttl)
                    {
                        Ok((distributed_lock::AcquireOutcome::Acquired, Some(guard))) => {
                            AdmissionOutcome::Pass(AdmissionGuard {
                                inner: GuardKind::MeshLock(guard),
                            })
                        }
                        _ => AdmissionOutcome::Rejected(AdmissionRejection::Dropped),
                    }
                }
            }
        }
        ConcurrencyMode::MeshLeader => {
            let key = format!("route:{}", route.path);
            let nodes = leader_election_nodes(state);
            if leader_election::am_i_leader_in(&nodes, &key) {
                AdmissionOutcome::Pass(AdmissionGuard {
                    inner: GuardKind::NoOp,
                })
            } else {
                AdmissionOutcome::Rejected(AdmissionRejection::NotLeader {
                    leader_node: leader_election::leader_for_in(&nodes, &key),
                })
            }
        }
        ConcurrencyMode::MeshLeaderStrict => {
            let key = format!("route:{}", route.path);
            let nodes = leader_election_nodes(state);
            // Phase 1: hash election — reject immediately if not the elected node.
            if !leader_election::am_i_leader_in(&nodes, &key) {
                return AdmissionOutcome::Rejected(AdmissionRejection::NotLeader {
                    leader_node: leader_election::leader_for_in(&nodes, &key),
                });
            }
            // Phase 2: distributed lock — prevents double-execution during the brief
            // window where two nodes may both consider themselves elected after a
            // registry change.
            let holder = leader_election::local_node_id();
            let lease_ttl =
                std::time::Duration::from_millis(policy.effective_lock_ttl_ms());
            match policy.on_conflict {
                ConflictPolicy::Queue => {
                    let wait_timeout = lease_ttl.saturating_mul(6);
                    match distributed_lock::acquire_with_wait(
                        &state.core_store,
                        &key,
                        &holder,
                        lease_ttl,
                        wait_timeout,
                    )
                    .await
                    {
                        Ok(Some(guard)) => AdmissionOutcome::Pass(AdmissionGuard {
                            inner: GuardKind::MeshLock(guard),
                        }),
                        Ok(None) => AdmissionOutcome::Rejected(AdmissionRejection::Conflict {
                            reason: format!(
                                "route `{}` mesh-leader-strict lock wait exceeded {}ms",
                                route.path,
                                wait_timeout.as_millis()
                            ),
                        }),
                        Err(error) => AdmissionOutcome::Rejected(AdmissionRejection::Conflict {
                            reason: format!(
                                "route `{}` mesh-leader-strict lock acquire failed: {error:#}",
                                route.path
                            ),
                        }),
                    }
                }
                ConflictPolicy::Reject => {
                    match distributed_lock::try_acquire(
                        &state.core_store,
                        &key,
                        &holder,
                        lease_ttl,
                    ) {
                        Ok((distributed_lock::AcquireOutcome::Acquired, Some(guard))) => {
                            AdmissionOutcome::Pass(AdmissionGuard {
                                inner: GuardKind::MeshLock(guard),
                            })
                        }
                        Ok(_) => AdmissionOutcome::Rejected(AdmissionRejection::Conflict {
                            reason: format!(
                                "route `{}` mesh-leader-strict lock is held by another invocation",
                                route.path
                            ),
                        }),
                        Err(error) => AdmissionOutcome::Rejected(AdmissionRejection::Conflict {
                            reason: format!(
                                "route `{}` mesh-leader-strict lock acquire failed: {error:#}",
                                route.path
                            ),
                        }),
                    }
                }
                ConflictPolicy::Drop => {
                    match distributed_lock::try_acquire(
                        &state.core_store,
                        &key,
                        &holder,
                        lease_ttl,
                    ) {
                        Ok((distributed_lock::AcquireOutcome::Acquired, Some(guard))) => {
                            AdmissionOutcome::Pass(AdmissionGuard {
                                inner: GuardKind::MeshLock(guard),
                            })
                        }
                        _ => AdmissionOutcome::Rejected(AdmissionRejection::Dropped),
                    }
                }
            }
        }
    }
}

fn leader_election_nodes(state: &AppState) -> Vec<String> {
    match state
        .core_store
        .kv_partition_get_range("node-registry", "", "\u{10ffff}", 10_000, 0)
    {
        Ok(rows) => rows
            .into_iter()
            .filter_map(|(_, value)| {
                serde_json::from_slice::<RegistryEnrolledNode>(&value)
                    .ok()
                    .map(|n| n.node_id)
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Helper module used by the backup scheduler's drain write_isolation. Kept
/// inside `concurrency_admission` because it manipulates the same
/// `AdmissionPauseRegistry` consulted by [`check`].
pub(crate) mod backup_drain {
    use super::*;

    pub(crate) async fn pause_admission(state: &AppState, route_path: &str) {
        state.admission_pause.pause(route_path);
        tracing::info!(route = route_path, "backup drain: admission paused");
    }

    pub(crate) fn resume_admission(state: &AppState, route_path: &str) {
        state.admission_pause.resume(route_path);
        tracing::info!(route = route_path, "backup drain: admission resumed");
    }

    /// Wait until the route has no active invocations, up to a fixed timeout.
    /// Phase 1: best-effort tracking — checks the routing runtime's
    /// `concurrency_limits` map for the per-route execution control and polls
    /// `active_count()`. If the route isn't tracked there yet, returns
    /// immediately (the route was never invoked since the runtime loaded).
    pub(crate) async fn wait_for_drain(state: &AppState, route_path: &str) {
        let timeout = std::time::Duration::from_secs(30);
        let poll = std::time::Duration::from_millis(200);
        let start = std::time::Instant::now();
        loop {
            let runtime = state.runtime.load();
            let inflight = runtime
                .concurrency_limits
                .get(route_path)
                .map(|ctrl| ctrl.active_request_count())
                .unwrap_or(0);
            if inflight == 0 {
                return;
            }
            if start.elapsed() >= timeout {
                tracing::warn!(
                    route = route_path,
                    inflight,
                    "backup drain: timeout waiting for active invocations"
                );
                return;
            }
            tokio::time::sleep(poll).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn node_singleton_queue_serializes() {
        let reg = NodeSingletonRegistry::new();
        assert!(reg.try_acquire("/route-a"));
        assert!(!reg.try_acquire("/route-a"));
        // After release, another acquire succeeds.
        reg.release("/route-a");
        assert!(reg.try_acquire("/route-a"));
    }

    #[tokio::test]
    async fn admission_pause_blocks_then_resumes() {
        let pause = AdmissionPauseRegistry::new();
        assert!(!pause.is_paused("/r"));
        pause.pause("/r");
        assert!(pause.is_paused("/r"));
        pause.resume("/r");
        assert!(!pause.is_paused("/r"));
    }
}
