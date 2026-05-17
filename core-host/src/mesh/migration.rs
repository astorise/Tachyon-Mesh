use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

const DEFAULT_REMOTE_RATIO: u64 = 10;
const DEFAULT_MIN_REMOTE_HITS: u64 = 1_000;
const DEFAULT_COOLDOWN: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GeoMigrationPlan {
    pub(crate) subspace: String,
    pub(crate) target_peer: String,
}

#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct SubspaceAccessTracker {
    local_peer_id: String,
    counts: HashMap<(String, String), u64>,
    cooldowns: HashMap<String, Instant>,
    remote_ratio: u64,
    min_remote_hits: u64,
    cooldown: Duration,
}

#[allow(dead_code)]
impl SubspaceAccessTracker {
    pub(crate) fn new(local_peer_id: impl Into<String>) -> Self {
        Self {
            local_peer_id: local_peer_id.into(),
            counts: HashMap::new(),
            cooldowns: HashMap::new(),
            remote_ratio: DEFAULT_REMOTE_RATIO,
            min_remote_hits: DEFAULT_MIN_REMOTE_HITS,
            cooldown: DEFAULT_COOLDOWN,
        }
    }

    pub(crate) fn with_thresholds(
        local_peer_id: impl Into<String>,
        remote_ratio: u64,
        min_remote_hits: u64,
        cooldown: Duration,
    ) -> Self {
        Self {
            local_peer_id: local_peer_id.into(),
            counts: HashMap::new(),
            cooldowns: HashMap::new(),
            remote_ratio,
            min_remote_hits,
            cooldown,
        }
    }

    pub(crate) fn record_access(
        &mut self,
        key_prefix: &str,
        requesting_peer: &str,
    ) -> Option<GeoMigrationPlan> {
        *self
            .counts
            .entry((key_prefix.to_owned(), requesting_peer.to_owned()))
            .or_default() += 1;
        self.evaluate_migration(key_prefix, requesting_peer, Instant::now())
    }

    fn evaluate_migration(
        &mut self,
        key_prefix: &str,
        remote_peer: &str,
        now: Instant,
    ) -> Option<GeoMigrationPlan> {
        if remote_peer == self.local_peer_id {
            return None;
        }
        if self
            .cooldowns
            .get(key_prefix)
            .is_some_and(|deadline| *deadline > now)
        {
            return None;
        }
        let remote_hits = self.estimate(key_prefix, remote_peer);
        let local_peer = self.local_peer_id.clone();
        let local_hits = self.estimate(key_prefix, &local_peer);
        if remote_hits >= self.min_remote_hits
            && remote_hits > local_hits.saturating_mul(self.remote_ratio)
        {
            self.cooldowns
                .insert(key_prefix.to_owned(), now + self.cooldown);
            Some(GeoMigrationPlan {
                subspace: key_prefix.to_owned(),
                target_peer: remote_peer.to_owned(),
            })
        } else {
            None
        }
    }

    pub(crate) fn estimate(&self, key_prefix: &str, peer: &str) -> u64 {
        self.counts
            .get(&(key_prefix.to_owned(), peer.to_owned()))
            .copied()
            .unwrap_or_default()
    }
}

#[derive(Debug, Default)]
#[allow(dead_code)]
pub(crate) struct SubspaceRoutingTable {
    primaries: HashMap<String, String>,
}

#[allow(dead_code)]
impl SubspaceRoutingTable {
    pub(crate) fn apply_update_route(&mut self, subspace: &str, primary_peer: &str) {
        self.primaries
            .insert(subspace.to_owned(), primary_peer.to_owned());
    }

    pub(crate) fn primary_for_key(&self, key: &str) -> Option<&str> {
        self.primaries
            .iter()
            .filter(|(subspace, _)| key.starts_with(subspace.as_str()))
            .max_by_key(|(subspace, _)| subspace.len())
            .map(|(_, peer)| peer.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracker_triggers_remote_migration_and_cools_down() {
        let mut tracker =
            SubspaceAccessTracker::with_thresholds("local", 2, 3, Duration::from_secs(60));

        assert!(tracker.record_access("tenant/users", "remote-a").is_none());
        assert!(tracker.record_access("tenant/users", "remote-a").is_none());
        let plan = tracker
            .record_access("tenant/users", "remote-a")
            .expect("third remote hit should trigger");

        assert_eq!(
            plan,
            GeoMigrationPlan {
                subspace: "tenant/users".to_owned(),
                target_peer: "remote-a".to_owned(),
            }
        );
        assert!(tracker.record_access("tenant/users", "remote-a").is_none());
    }

    #[test]
    fn routing_table_uses_longest_subspace_prefix() {
        let mut table = SubspaceRoutingTable::default();
        table.apply_update_route("tenant", "peer-a");
        table.apply_update_route("tenant/users", "peer-b");

        assert_eq!(table.primary_for_key("tenant/users/42"), Some("peer-b"));
        assert_eq!(table.primary_for_key("tenant/orders/1"), Some("peer-a"));
    }
}
