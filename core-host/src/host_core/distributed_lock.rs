use super::*;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

/// Outbox event types published when a lock is acquired or released so the
/// existing `ConfigUpdateOutbox` gossip path can broadcast the state change
/// across the mesh. Peer nodes that import these events update their local
/// `CoreStore` view via the standard config-update subscriber path.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
enum DistributedLockEvent {
    #[serde(rename = "distributed-lock.acquired")]
    Acquired {
        key: String,
        holder: String,
        lease_ttl_ms: u64,
        acquired_ms: u64,
    },
    #[serde(rename = "distributed-lock.released")]
    Released { key: String, holder: String },
}

/// RAII guard that holds a distributed lock and refreshes its lease in the
/// background. Dropping the guard releases the lock via the `CoreStore` if it
/// is still owned by the local holder; if a remote node took over after a
/// missed heartbeat the release becomes a no-op (the take-over is logged).
pub(crate) struct DistributedLockGuard {
    inner: Arc<DistributedLockGuardInner>,
}

struct DistributedLockGuardInner {
    core_store: Arc<store::CoreStore>,
    key: String,
    holder: String,
    stop: Notify,
}

impl DistributedLockGuard {
    #[allow(dead_code)]
    pub(crate) fn key(&self) -> &str {
        &self.inner.key
    }
}

impl Drop for DistributedLockGuard {
    fn drop(&mut self) {
        // Signal the heartbeat task to stop. Use `notify_one` so the
        // heartbeat task wakes from its `notified()` wait and exits.
        self.inner.stop.notify_waiters();
        match self
            .inner
            .core_store
            .lock_release(&self.inner.key, &self.inner.holder)
        {
            Ok(true) => {
                publish_lock_event(
                    &self.inner.core_store,
                    &DistributedLockEvent::Released {
                        key: self.inner.key.clone(),
                        holder: self.inner.holder.clone(),
                    },
                );
            }
            Ok(false) => {
                tracing::debug!(
                    key = %self.inner.key,
                    holder = %self.inner.holder,
                    "distributed lock release: lock already owned by another node"
                );
            }
            Err(error) => {
                tracing::warn!(
                    key = %self.inner.key,
                    holder = %self.inner.holder,
                    "distributed lock release failed: {error:#}"
                );
            }
        }
    }
}

fn publish_lock_event(core_store: &store::CoreStore, event: &DistributedLockEvent) {
    let payload = match serde_json::to_vec(event) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!("failed to encode distributed lock event: {error:#}");
            return;
        }
    };
    if let Err(error) = core_store.append_outbox(store::CoreStoreBucket::ConfigUpdateOutbox, &payload) {
        tracing::warn!("failed to publish distributed lock event to outbox: {error:#}");
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum AcquireOutcome {
    /// Lock was acquired locally and is now held by us.
    Acquired,
    /// Another holder owns a non-expired lock.
    Held,
}

/// Try to acquire `key` without waiting. Returns immediately with the outcome.
pub(crate) fn try_acquire(
    core_store: &Arc<store::CoreStore>,
    key: &str,
    holder: &str,
    lease_ttl: Duration,
) -> Result<(AcquireOutcome, Option<DistributedLockGuard>)> {
    let now_ms = now_unix_ms();
    let lease_ms = lease_ttl.as_millis() as u64;
    let acquired = core_store.lock_try_acquire(key, holder, lease_ms, now_ms)?;
    if !acquired {
        return Ok((AcquireOutcome::Held, None));
    }
    publish_lock_event(
        core_store,
        &DistributedLockEvent::Acquired {
            key: key.to_owned(),
            holder: holder.to_owned(),
            lease_ttl_ms: lease_ms,
            acquired_ms: now_ms,
        },
    );
    let inner = Arc::new(DistributedLockGuardInner {
        core_store: Arc::clone(core_store),
        key: key.to_owned(),
        holder: holder.to_owned(),
        stop: Notify::new(),
    });
    spawn_heartbeat_task(Arc::clone(&inner), lease_ttl);
    Ok((AcquireOutcome::Acquired, Some(DistributedLockGuard { inner })))
}

/// Block until the lock is acquired or `wait_timeout` elapses. Polls with
/// exponential backoff capped at `lease_ttl / 4`. Returns `None` on timeout.
pub(crate) async fn acquire_with_wait(
    core_store: &Arc<store::CoreStore>,
    key: &str,
    holder: &str,
    lease_ttl: Duration,
    wait_timeout: Duration,
) -> Result<Option<DistributedLockGuard>> {
    let deadline = tokio::time::Instant::now() + wait_timeout;
    let mut backoff = Duration::from_millis(50);
    let cap = (lease_ttl / 4).max(Duration::from_millis(100));
    loop {
        match try_acquire(core_store, key, holder, lease_ttl)? {
            (AcquireOutcome::Acquired, Some(guard)) => return Ok(Some(guard)),
            _ => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(None);
        }
        let sleep_for = backoff.min(deadline - tokio::time::Instant::now());
        tokio::time::sleep(sleep_for).await;
        backoff = (backoff * 2).min(cap);
    }
}

fn spawn_heartbeat_task(inner: Arc<DistributedLockGuardInner>, lease_ttl: Duration) {
    // Refresh at ttl/2 so a missed beat still leaves the lock valid for one
    // more interval before another node can steal it.
    let interval = (lease_ttl / 2).max(Duration::from_millis(500));
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = inner.stop.notified() => break,
                _ = tokio::time::sleep(interval) => {
                    let now_ms = now_unix_ms();
                    match inner.core_store.lock_heartbeat(
                        &inner.key,
                        &inner.holder,
                        lease_ttl.as_millis() as u64,
                        now_ms,
                    ) {
                        Ok(true) => {}
                        Ok(false) => {
                            tracing::warn!(
                                key = %inner.key,
                                holder = %inner.holder,
                                "distributed lock heartbeat: another node has taken over the lock"
                            );
                            break;
                        }
                        Err(error) => {
                            tracing::warn!(
                                key = %inner.key,
                                "distributed lock heartbeat failed: {error:#}"
                            );
                        }
                    }
                }
            }
        }
    });
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_store() -> (TempDir, Arc<store::CoreStore>) {
        let dir = TempDir::new().expect("temp dir for core store");
        let store = store::CoreStore::open(&dir.path().join("test.db"))
            .expect("core store should open");
        (dir, Arc::new(store))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn acquire_blocks_concurrent_holders() {
        let (_dir, store) = make_store();
        let (out_a, guard_a) =
            try_acquire(&store, "test:1", "node-a", Duration::from_secs(5)).expect("acquire a");
        assert!(matches!(out_a, AcquireOutcome::Acquired));
        assert!(guard_a.is_some());

        let (out_b, guard_b) =
            try_acquire(&store, "test:1", "node-b", Duration::from_secs(5)).expect("acquire b");
        assert!(matches!(out_b, AcquireOutcome::Held));
        assert!(guard_b.is_none());

        drop(guard_a);
        // After release, node-b can acquire. Give the drop time to run.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let (out_b2, guard_b2) =
            try_acquire(&store, "test:1", "node-b", Duration::from_secs(5)).expect("re-acquire b");
        assert!(matches!(out_b2, AcquireOutcome::Acquired));
        assert!(guard_b2.is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn expired_lease_is_reclaimable() {
        let (_dir, store) = make_store();
        // Inject a stale lock by lowering the system clock view via direct
        // CoreStore methods. Acquire with a 1ms TTL, wait 50ms, then another
        // holder should be able to take it over.
        let (out, guard) =
            try_acquire(&store, "test:2", "node-a", Duration::from_millis(1)).expect("acquire");
        assert!(matches!(out, AcquireOutcome::Acquired));
        // Leak the guard's stop signal so we don't release it cleanly — we
        // want to simulate a crashed holder.
        std::mem::forget(guard);

        tokio::time::sleep(Duration::from_millis(50)).await;

        let (out2, guard2) = try_acquire(
            &store,
            "test:2",
            "node-b",
            Duration::from_millis(100),
        )
        .expect("reclaim");
        assert!(
            matches!(out2, AcquireOutcome::Acquired),
            "expected expired lease to be reclaimable"
        );
        assert!(guard2.is_some());
    }
}
