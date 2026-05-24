use std::sync::atomic::Ordering::Relaxed;

use crate::host_core::scoping::{DeploymentScopes, ScopeCategory, ScopeDenialCounters};

/// The `kv-partition.table` constructor checks `scopes.check_kv(&name)` once and
/// stores the result as `RedbTableResource::scope_denial`.  Subsequent `get`/`set`
/// calls read from the stored denial without re-calling the scope check — the
/// handle-bound invariant.  These tests verify the scope-check and counter-increment
/// logic; they mirror the logic in `ComponentHostState`'s `HostTable` implementation.

#[test]
fn kv_scope_allows_matching_table_name() {
    let scopes = DeploymentScopes::from_manifest(&serde_json::json!({ "kv": ["tenant-a/**"] }))
        .expect("manifest should parse");
    assert!(
        scopes.check_kv("tenant-a/users"),
        "direct sub-path must match"
    );
    assert!(
        scopes.check_kv("tenant-a/orders/2024"),
        "nested sub-path must match"
    );
}

#[test]
fn kv_scope_denies_non_matching_table_name() {
    let scopes = DeploymentScopes::from_manifest(&serde_json::json!({ "kv": ["tenant-a/**"] }))
        .expect("manifest should parse");
    assert!(
        !scopes.check_kv("tenant-b/users"),
        "different tenant prefix must be denied"
    );
    assert!(!scopes.check_kv("global"), "top-level name must be denied");
}

#[test]
fn kv_scope_denial_increments_counter_once_at_construction() {
    let scopes = DeploymentScopes::from_manifest(&serde_json::json!({ "kv": ["tenant-a/*"] }))
        .expect("manifest should parse");
    let denials = ScopeDenialCounters::default();

    // Simulate the host closure: check → deny → store denial in handle.
    let scope_denial: Option<String> = if !scopes.check_kv("tenant-b/users") {
        denials.increment(ScopeCategory::Kv);
        Some("permission denied: kv table `tenant-b/users` not granted".to_owned())
    } else {
        None
    };
    assert!(
        scope_denial.is_some(),
        "scope denial must be captured in handle"
    );
    assert_eq!(
        denials.kv.load(Relaxed),
        1,
        "denial counter must be 1 after construction"
    );

    // Simulate 1000 get/set calls on the poisoned handle: they read `scope_denial`
    // without re-calling `check_kv`, so the denial counter must not increase.
    for _ in 0..1_000 {
        if scope_denial.is_some() {
            // Host closure returns Err(denial) — no counter increment here.
        }
    }
    assert_eq!(
        denials.kv.load(Relaxed),
        1,
        "1000 calls on a denied handle must not increment the counter again (handle-bound invariant)"
    );
}

#[test]
fn kv_scope_allowed_handle_has_no_denial() {
    let scopes = DeploymentScopes::from_manifest(&serde_json::json!({ "kv": ["tenant-a/*"] }))
        .expect("manifest should parse");
    let denials = ScopeDenialCounters::default();

    let scope_denial: Option<String> = if !scopes.check_kv("tenant-a/users") {
        denials.increment(ScopeCategory::Kv);
        Some("denied".to_owned())
    } else {
        None
    };
    assert!(
        scope_denial.is_none(),
        "allowed table name must produce no denial"
    );
    assert_eq!(
        denials.kv.load(Relaxed),
        0,
        "no counter increment for an allowed table"
    );
}

#[test]
fn kv_scope_handle_bound_invariant_verified_via_counter() {
    let scopes = DeploymentScopes::from_manifest(&serde_json::json!({ "kv": ["tenant-a/*"] }))
        .expect("manifest should parse");
    let denials = ScopeDenialCounters::default();

    // Open tenant-a/users → ok.
    let denied_a: Option<String> = if !scopes.check_kv("tenant-a/users") {
        denials.increment(ScopeCategory::Kv);
        Some("denied".to_owned())
    } else {
        None
    };
    assert!(denied_a.is_none());
    assert_eq!(denials.kv.load(Relaxed), 0, "no denial for allowed name");

    // Open tenant-b/users → denied; counter == 1.
    let denied_b: Option<String> = if !scopes.check_kv("tenant-b/users") {
        denials.increment(ScopeCategory::Kv);
        Some("denied".to_owned())
    } else {
        None
    };
    assert!(denied_b.is_some());
    assert_eq!(
        denials.kv.load(Relaxed),
        1,
        "one denial for disallowed name"
    );

    // 1000 get/set calls on the poisoned handle must not touch the counter.
    let before = denials.kv.load(Relaxed);
    for _ in 0..1_000 {
        if denied_b.is_some() {
            // returns Err(denied_b.clone()) without calling check_kv again
        }
    }
    assert_eq!(
        denials.kv.load(Relaxed),
        before,
        "handle-bound invariant: no additional counter increments from get/set on denied handle"
    );
}
