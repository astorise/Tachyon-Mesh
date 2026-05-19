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

#[tokio::test]
async fn enrollment_approval_persists_node_registry_row() {
    let manifest_path = std::env::temp_dir().join(format!(
        "tachyon-node-registry-{}.lock",
        uuid::Uuid::new_v4()
    ));
    let state = build_test_state_with_manifest(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
        manifest_path.clone(),
    );
    let session = state
        .enrollment_manager
        .start_session("public-key-abcdef12".to_owned());

    let response = admin_enrollment_approve_handler(
        State(state.clone()),
        Json(AdminEnrollmentApproveRequest {
            session_id: session.session_id,
            pin: session.pin,
            signed_certificate_hex: "cafe".to_owned(),
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    let raw = state
        .core_store
        .kv_partition_get("node-registry", "node-abcdef12")
        .expect("node registry should read")
        .expect("node row should exist after reopen");
    let node: Value = serde_json::from_slice(&raw).expect("node row should decode");
    assert_eq!(node["status"], "awaiting-capabilities");
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
