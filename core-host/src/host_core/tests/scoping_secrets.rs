use std::sync::atomic::Ordering::Relaxed;

use crate::host_core::scoping::{DeploymentScopes, ScopeCategory, ScopeDenialCounters};

/// The host closure for `get-secret` calls `scopes.check_secrets(&name)` before
/// forwarding to the secrets backend.  These tests verify the scope-check and
/// counter-increment logic that the closure delegates to.

#[test]
fn secrets_scope_allows_matching_name() {
    let scopes =
        DeploymentScopes::from_manifest(&serde_json::json!({ "secrets": ["db/prod/*"] }))
            .expect("manifest should parse");
    assert!(scopes.check_secrets("db/prod/password"), "glob must match sub-path");
    assert!(scopes.check_secrets("db/prod/p"), "short name under glob must match");
}

#[test]
fn secrets_scope_denies_non_matching_name() {
    let scopes =
        DeploymentScopes::from_manifest(&serde_json::json!({ "secrets": ["db/prod/*"] }))
            .expect("manifest should parse");
    assert!(!scopes.check_secrets("auth/master"), "different prefix must not match");
    assert!(!scopes.check_secrets("db/staging/pw"), "different branch must not match");
}

#[test]
fn secrets_scope_denial_increments_counter() {
    let scopes =
        DeploymentScopes::from_manifest(&serde_json::json!({ "secrets": ["db/prod/*"] }))
            .expect("manifest should parse");
    let denials = ScopeDenialCounters::default();

    // Simulate the host closure: check → deny → increment.
    if !scopes.check_secrets("auth/master") {
        denials.increment(ScopeCategory::Secrets);
    }
    assert_eq!(
        denials.secrets.load(Relaxed),
        1,
        "one denial must increment the secrets counter"
    );
}

#[test]
fn secrets_scope_allow_path_does_not_increment_counter() {
    let scopes =
        DeploymentScopes::from_manifest(&serde_json::json!({ "secrets": ["db/prod/*"] }))
            .expect("manifest should parse");
    let denials = ScopeDenialCounters::default();

    // Allowed path: the counter must stay at zero.
    if !scopes.check_secrets("db/prod/password") {
        denials.increment(ScopeCategory::Secrets);
    }
    assert_eq!(
        denials.secrets.load(Relaxed),
        0,
        "an allowed access must not increment the secrets counter"
    );
}

#[test]
fn secrets_scope_multi_pattern_or_semantics() {
    let scopes = DeploymentScopes::from_manifest(&serde_json::json!({
        "secrets": ["db/prod/*", "auth/service-accounts/*"]
    }))
    .expect("manifest should parse");
    assert!(scopes.check_secrets("db/prod/pw"), "first pattern must match");
    assert!(
        scopes.check_secrets("auth/service-accounts/svc"),
        "second pattern must match"
    );
    assert!(
        !scopes.check_secrets("auth/master"),
        "unmatched name must be denied"
    );
}

#[test]
fn secrets_absent_category_denies_all() {
    // When `secrets` is not present in the manifest, the interface is not linked.
    // At runtime, `check_secrets` returns false for every name.
    let scopes =
        DeploymentScopes::from_manifest(&serde_json::json!({ "kv": ["t/*"] }))
            .expect("manifest should parse");
    assert!(
        !scopes.check_secrets("db/prod/pw"),
        "absent secrets category must deny all names"
    );
}
