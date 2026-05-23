use wasmtime::{
    component::{Component, Linker as ComponentLinker},
    Engine, Store,
};

use crate::host_core::scoping::{DeploymentScopes, ScopeCategory, ScopeShape};
use crate::host_core::integrity_config::validate_integrity_config;
use crate::host_core::domain_types::{IntegrityConfig, IntegrityRoute};

fn test_engine() -> Engine {
    Engine::default()
}

/// Build a minimal WAT component that declares a single function import from
/// `tachyon:mesh/bridge-controller@1.1.0`. The component body is otherwise empty
/// (it never calls the import); we only care about whether the linker satisfies it.
fn bridge_importing_component(engine: &Engine) -> Component {
    Component::new(
        engine,
        r#"(component
            (import "tachyon:mesh/bridge-controller@1.1.0" (instance
                (export "create-bridge" (func))
            ))
        )"#,
    )
    .expect("bridge-importing WAT component should compile")
}

#[test]
fn linker_without_bridge_rejects_bridge_importing_component() {
    let engine = test_engine();
    let component = bridge_importing_component(&engine);

    // A bare linker with no bridge-controller registered simulates the production
    // code path where `shape.grants(ScopeCategory::Bridge)` is false and
    // `bridge_controller::add_to_linker` is therefore skipped.
    let linker: ComponentLinker<()> = ComponentLinker::new(&engine);
    let mut store = Store::new(&engine, ());

    let err = linker
        .instantiate(&mut store, &component)
        .expect_err("instantiation without bridge in linker must fail");

    let msg = err.to_string();
    assert!(
        msg.contains("bridge-controller") || msg.contains("missing"),
        "error should name the missing bridge-controller import; got: {msg}"
    );
}

#[test]
fn scope_shape_without_bridge_does_not_grant_bridge() {
    let scopes = DeploymentScopes::from_manifest(&serde_json::json!({
        "secrets": ["db/*"],
        "kv": ["tenant-a/*"],
    }))
    .expect("manifest should parse");
    let shape = ScopeShape::of(&scopes);
    assert!(
        !shape.grants(ScopeCategory::Bridge),
        "shape built from no-bridge manifest must not grant Bridge"
    );
}

#[test]
fn scope_shape_with_bridge_grants_bridge() {
    let scopes = DeploymentScopes::from_manifest(&serde_json::json!({
        "bridge": ["10.0.*:*"],
    }))
    .expect("manifest should parse");
    let shape = ScopeShape::of(&scopes);
    assert!(
        shape.grants(ScopeCategory::Bridge),
        "shape built from bridge-granting manifest must grant Bridge"
    );
}

#[test]
fn scope_categories_are_disjoint_from_routing_and_validation() {
    // Adding or removing scope categories must NOT affect manifest validation
    // success or route ordering (which drives routing selection). Scopes are
    // orthogonal to the capability_mask / routing path.
    let mut route_no_scopes = IntegrityRoute::user("/api/guest-example");
    route_no_scopes.scopes = None;

    let mut route_with_scopes = IntegrityRoute::user("/api/guest-example-2");
    route_with_scopes.scopes = Some(serde_json::json!({ "kv": ["tenant-a/*"] }));

    let mut config = IntegrityConfig::default_sealed();
    config.routes = vec![route_no_scopes, route_with_scopes];

    let validated = validate_integrity_config(config)
        .expect("mixed-scope routes should validate under default require_scopes=false");

    // Routes are sorted alphabetically; scopes do not alter this ordering.
    assert_eq!(validated.routes[0].path, "/api/guest-example");
    assert_eq!(validated.routes[1].path, "/api/guest-example-2");
    // Scope presence/absence does not affect the sealed route lookup.
    assert!(validated.sealed_route("/api/guest-example").is_some());
    assert!(validated.sealed_route("/api/guest-example-2").is_some());
}
