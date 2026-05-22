use super::*;
use std::hash::{DefaultHasher, Hash, Hasher};

/// Returns `true` if the local node is the deterministic leader for `resource_key`,
/// given the current set of registered mesh nodes loaded from `core_store`.
///
/// Uses `hash(key) % nodes` and compares against `local_node_id()` so every node
/// converges on the same answer without any communication, assuming the registry
/// snapshot is consistent. During registry propagation gaps two nodes may briefly
/// consider themselves leader for the same key — acceptable for idempotent
/// operations (backups, scheduled triggers) where double-execution is wasteful
/// but not destructive. Use `DistributedLock` for exclusive critical sections.
pub(crate) fn am_i_leader_in(node_ids: &[String], resource_key: &str) -> bool {
    let local_id = local_node_id();
    match leader_for_in(node_ids, resource_key) {
        Some(elected) => elected == local_id,
        // Empty registry → single-node bootstrap, always leader.
        None => true,
    }
}

/// Returns the elected leader node id for `resource_key` given a pre-loaded list
/// of node ids, or `None` when the list is empty.
pub(crate) fn leader_for_in(node_ids: &[String], resource_key: &str) -> Option<String> {
    if node_ids.is_empty() {
        return None;
    }
    let mut sorted: Vec<&str> = node_ids.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    sorted.dedup();
    let mut h = DefaultHasher::new();
    resource_key.hash(&mut h);
    let idx = (h.finish() as usize) % sorted.len();
    Some(sorted[idx].to_owned())
}

/// Convenience wrapper that loads the current node registry from `core_store`
/// and delegates to [`am_i_leader_in`]. Use this from runtime code that has
/// access to `AppState`; use [`am_i_leader_in`] in pure tests.
pub(crate) fn am_i_leader(state: &AppState, resource_key: &str) -> bool {
    let nodes = list_node_ids(state);
    am_i_leader_in(&nodes, resource_key)
}

fn list_node_ids(state: &AppState) -> Vec<String> {
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

/// Identifier the local node uses when comparing against an elected leader.
/// Honors the `TACHYON_NODE_ID` env var if set, otherwise falls back to the
/// system hostname (which k8s pods receive as `HOSTNAME`). The same identifier
/// MUST be registered into the node registry by `system-faas-node-registry`
/// for the comparison to succeed.
pub(crate) fn local_node_id() -> String {
    if let Ok(id) = std::env::var("TACHYON_NODE_ID") {
        if !id.trim().is_empty() {
            return id;
        }
    }
    std::env::var("HOSTNAME")
        .ok()
        .filter(|h| !h.trim().is_empty())
        .unwrap_or_else(|| "tachyon-node-local".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_registry_returns_none() {
        assert!(leader_for_in(&[], "any-key").is_none());
    }

    #[test]
    fn single_node_is_always_leader() {
        let nodes = vec!["node-a".to_owned()];
        assert_eq!(leader_for_in(&nodes, "any-key"), Some("node-a".to_owned()));
    }

    #[test]
    fn deterministic_across_calls() {
        let nodes = vec![
            "node-a".to_owned(),
            "node-b".to_owned(),
            "node-c".to_owned(),
        ];
        let first = leader_for_in(&nodes, "route:/api/backup");
        let second = leader_for_in(&nodes, "route:/api/backup");
        assert_eq!(first, second);
    }

    #[test]
    fn different_keys_can_elect_different_leaders() {
        let nodes = vec![
            "node-a".to_owned(),
            "node-b".to_owned(),
            "node-c".to_owned(),
            "node-d".to_owned(),
            "node-e".to_owned(),
        ];
        let mut elected = std::collections::HashSet::new();
        for i in 0..50 {
            if let Some(leader) = leader_for_in(&nodes, &format!("key-{i}")) {
                elected.insert(leader);
            }
        }
        assert!(
            elected.len() >= 2,
            "expected at least 2 distinct leaders, got {elected:?}"
        );
    }
}
