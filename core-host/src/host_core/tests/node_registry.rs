// Integration tests for the node-registry and capabilities layers that operate
// directly on the ReDB store (via core_store kv_partition helpers).
//
// The enrollment ceremony itself now runs inside the `system-faas-node-registry`
// WASM component and is exercised by the vcluster E2E workflow (e2e-mesh.yml).
// These tests cover the host-side persistence and the Axum handlers that read /
// write `core_store` directly.

use super::support_and_cache::*;
use axum::{
    body::to_bytes,
    extract::{Path, State},
    response::Response,
    Json,
};
use serde_json::Value;

async fn response_json(response: Response) -> Value {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should collect");
    serde_json::from_slice(&body).expect("response should be JSON")
}

/// Simulates an approved enrollment by writing the row directly into the
/// ReDB kv-partition (the same path the FaaS WASM takes at runtime).
/// Verifies that `admin_nodes_handler` surfaces the row correctly.
#[tokio::test]
async fn pre_enrolled_node_appears_in_nodes_list() {
    let state = build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    );

    let node = RegistryEnrolledNode {
        node_id: "node-abcdef12".to_owned(),
        public_key: "public-key-abcdef12".to_owned(),
        status: "awaiting-capabilities".to_owned(),
        approved_at: 1,
        last_seen: 1,
        region: None,
        zone: None,
        capabilities: RegistryNodeCapabilities::default(),
    };
    state
        .core_store
        .kv_partition_set(
            "node-registry",
            "node-abcdef12",
            &serde_json::to_vec(&node).expect("node should encode"),
        )
        .expect("node should persist");

    let list = response_json(admin_nodes_handler(State(state)).await).await;
    assert_eq!(list[0]["nodeId"], "node-abcdef12");
    assert_eq!(list[0]["status"], "awaiting-capabilities");
}

#[tokio::test]
async fn capability_post_updates_node_and_nodes_list_reports_online_status() {
    let state = build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    );
    let node = RegistryEnrolledNode {
        node_id: "node-a".to_owned(),
        public_key: "pk".to_owned(),
        status: "awaiting-capabilities".to_owned(),
        approved_at: 1,
        last_seen: 1,
        region: None,
        zone: None,
        capabilities: RegistryNodeCapabilities::default(),
    };
    state
        .core_store
        .kv_partition_set(
            "node-registry",
            "node-a",
            &serde_json::to_vec(&node).expect("node should encode"),
        )
        .expect("node should persist");

    let response = admin_node_capabilities_handler(
        State(state.clone()),
        Path("node-a".to_owned()),
        Json(RegistryNodeCapabilities {
            total_ram_mb: 32768,
            available_ram_mb: 12000,
            accelerators: vec!["cpu".to_owned(), "gpu".to_owned()],
            gpus: vec![RegistryGpuStats {
                id: "cuda:0".to_owned(),
                model: "NVIDIA GPU".to_owned(),
                vram_total_mb: 24576,
                vram_used_mb: 1024,
                compute_utilization: 0.0,
            }],
            active_systems: vec![RegistryActiveSystem {
                slug: "gateway".to_owned(),
                version: "1.1.0-alpha".to_owned(),
            }],
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let list = response_json(admin_nodes_handler(State(state)).await).await;
    assert_eq!(list[0]["nodeId"], "node-a");
    assert_eq!(list[0]["status"], "online");
    assert_eq!(list[0]["capabilities"]["totalRamMb"], 32768);
}
