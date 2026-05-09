use std::{
    collections::VecDeque,
    sync::{Arc, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;

const DEFAULT_CAPACITY: usize = 1024;
pub(crate) const DEFAULT_TAIL: usize = 50;
pub(crate) const MAX_TAIL: usize = 500;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IamAuditEntry {
    pub(crate) timestamp: u64,
    pub(crate) actor: String,
    pub(crate) target_user: Option<String>,
    pub(crate) target_group: Option<String>,
    pub(crate) action: String,
    pub(crate) outcome: String,
    pub(crate) detail: String,
}

#[derive(Clone)]
pub(crate) struct IamAuditLog {
    inner: Arc<RwLock<VecDeque<IamAuditEntry>>>,
    capacity: usize,
}

impl IamAuditLog {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(VecDeque::with_capacity(DEFAULT_CAPACITY))),
            capacity: DEFAULT_CAPACITY,
        }
    }

    pub(crate) fn record(
        &self,
        actor: impl Into<String>,
        action: impl Into<String>,
        target_user: Option<String>,
        target_group: Option<String>,
        outcome: impl Into<String>,
        detail: impl Into<String>,
    ) {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        let entry = IamAuditEntry {
            timestamp,
            actor: actor.into(),
            target_user,
            target_group,
            action: action.into(),
            outcome: outcome.into(),
            detail: detail.into(),
        };
        let mut guard = match self.inner.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if guard.len() >= self.capacity {
            guard.pop_front();
        }
        guard.push_back(entry);
    }

    pub(crate) fn snapshot(&self, user: Option<&str>, lines: usize) -> Vec<IamAuditEntry> {
        let guard = match self.inner.read() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let take = lines.clamp(1, MAX_TAIL);
        let normalized = user.map(|value| value.trim().to_ascii_lowercase());
        let mut filtered: Vec<IamAuditEntry> = guard
            .iter()
            .filter(|entry| match normalized.as_deref() {
                None => true,
                Some(target) => {
                    target.is_empty()
                        || entry
                            .target_user
                            .as_deref()
                            .map(|value| value.eq_ignore_ascii_case(target))
                            .unwrap_or(false)
                        || entry.actor.eq_ignore_ascii_case(target)
                }
            })
            .cloned()
            .collect();
        filtered.reverse();
        filtered.truncate(take);
        filtered
    }
}

impl Default for IamAuditLog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capped_log(capacity: usize) -> IamAuditLog {
        IamAuditLog {
            inner: Arc::new(RwLock::new(VecDeque::with_capacity(capacity))),
            capacity,
        }
    }

    #[test]
    fn record_appends_entries_in_order() {
        let log = IamAuditLog::new();
        log.record(
            "alice",
            "user.disable",
            Some("bob".to_owned()),
            None,
            "ok",
            "",
        );
        log.record(
            "alice",
            "group.upsert",
            None,
            Some("ops".to_owned()),
            "ok",
            "roles=admin",
        );

        let entries = log.snapshot(None, 50);
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0].action, "group.upsert",
            "snapshot is newest-first"
        );
        assert_eq!(entries[1].action, "user.disable");
    }

    #[test]
    fn snapshot_filters_by_target_user() {
        let log = IamAuditLog::new();
        log.record(
            "alice",
            "user.disable",
            Some("bob".to_owned()),
            None,
            "ok",
            "",
        );
        log.record(
            "carol",
            "user.update",
            Some("dave".to_owned()),
            None,
            "ok",
            "",
        );
        log.record(
            "alice",
            "group.upsert",
            None,
            Some("ops".to_owned()),
            "ok",
            "",
        );

        let bob = log.snapshot(Some("bob"), 50);
        assert_eq!(bob.len(), 1);
        assert_eq!(bob[0].action, "user.disable");
    }

    #[test]
    fn snapshot_filters_by_actor() {
        let log = IamAuditLog::new();
        log.record(
            "alice",
            "user.disable",
            Some("bob".to_owned()),
            None,
            "ok",
            "",
        );
        log.record(
            "carol",
            "user.update",
            Some("dave".to_owned()),
            None,
            "ok",
            "",
        );

        let alice = log.snapshot(Some("alice"), 50);
        assert_eq!(alice.len(), 1);
        assert_eq!(alice[0].actor, "alice");
    }

    #[test]
    fn snapshot_filter_is_case_insensitive() {
        let log = IamAuditLog::new();
        log.record(
            "Alice",
            "user.disable",
            Some("Bob".to_owned()),
            None,
            "ok",
            "",
        );

        assert_eq!(log.snapshot(Some("alice"), 50).len(), 1);
        assert_eq!(log.snapshot(Some("BOB"), 50).len(), 1);
    }

    #[test]
    fn snapshot_lines_argument_clamps_to_max() {
        let log = IamAuditLog::new();
        for index in 0..600 {
            log.record(
                "alice",
                format!("event.{index}"),
                Some("bob".to_owned()),
                None,
                "ok",
                "",
            );
        }

        let entries = log.snapshot(None, 10_000);
        assert_eq!(entries.len(), MAX_TAIL);
    }

    #[test]
    fn snapshot_empty_user_filter_returns_all_entries() {
        let log = IamAuditLog::new();
        log.record("alice", "user.list", None, None, "ok", "");
        log.record("bob", "group.list", None, None, "ok", "");

        assert_eq!(log.snapshot(Some(""), 50).len(), 2);
        assert_eq!(log.snapshot(None, 50).len(), 2);
    }

    #[test]
    fn ring_buffer_evicts_oldest_entry_at_capacity() {
        let log = capped_log(3);
        log.record("alice", "first", None, None, "ok", "");
        log.record("alice", "second", None, None, "ok", "");
        log.record("alice", "third", None, None, "ok", "");
        log.record("alice", "fourth", None, None, "ok", "");

        let entries = log.snapshot(None, 50);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].action, "fourth");
        assert!(entries.iter().all(|entry| entry.action != "first"));
    }

    #[test]
    fn snapshot_lines_argument_clamps_to_one_when_zero() {
        let log = IamAuditLog::new();
        log.record("alice", "first", None, None, "ok", "");
        log.record("alice", "second", None, None, "ok", "");

        let entries = log.snapshot(None, 0);
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn outcome_field_distinguishes_ok_and_error() {
        let log = IamAuditLog::new();
        log.record(
            "alice",
            "user.delete",
            Some("bob".to_owned()),
            None,
            "ok",
            "",
        );
        log.record(
            "alice",
            "user.delete",
            Some("alice".to_owned()),
            None,
            "error",
            "self delete refused",
        );

        let entries = log.snapshot(None, 50);
        assert_eq!(entries[0].outcome, "error");
        assert!(entries[0].detail.contains("self delete refused"));
        assert_eq!(entries[1].outcome, "ok");
    }
}
