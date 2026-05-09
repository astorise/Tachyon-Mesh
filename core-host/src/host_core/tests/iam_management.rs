use super::support_and_cache::*;
use crate::auth::{AuditLogQuery, IamUserSummaryResponse};
use crate::iam_audit::{IamAuditEntry, MAX_TAIL};
use axum::extract::{Query, State};

// =====================================================================
// Endpoint surface tests — exercise the admin middleware without
// requiring a compiled authn guest. The 401 path short-circuits before
// any WASM is loaded.
// =====================================================================

#[tokio::test]
async fn iam_users_list_rejects_unauthenticated_caller() {
    let app = build_app(build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    ));

    let response = app
        .oneshot(
            Request::get("/admin/iam/users")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn iam_user_patch_rejects_unauthenticated_caller() {
    let app = build_app(build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    ));

    let response = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/admin/iam/users/alice")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn iam_user_delete_rejects_unauthenticated_caller() {
    let app = build_app(build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    ));

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/admin/iam/users/alice")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn iam_groups_list_rejects_unauthenticated_caller() {
    let app = build_app(build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    ));

    let response = app
        .oneshot(
            Request::get("/admin/iam/groups")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn iam_groups_post_rejects_unauthenticated_caller() {
    let app = build_app(build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    ));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/iam/groups")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"name":"ops","description":"","roles":[],"scopes":[]}"#,
                ))
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn iam_group_delete_rejects_unauthenticated_caller() {
    let app = build_app(build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    ));

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/admin/iam/groups/ops")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_logs_rejects_unauthenticated_caller() {
    let app = build_app(build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    ));

    let response = app
        .oneshot(
            Request::get("/admin/logs?user=alice&lines=10")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// =====================================================================
// audit_log_handler direct invocations — exercise the query parsing and
// snapshot composition without going through the bearer middleware.
// =====================================================================

#[tokio::test]
async fn audit_log_handler_filters_by_target_user() {
    let state = build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    );
    state.iam_audit_log.record(
        "admin",
        "user.disable",
        Some("alice".to_owned()),
        None,
        "ok",
        "",
    );
    state.iam_audit_log.record(
        "admin",
        "user.disable",
        Some("bob".to_owned()),
        None,
        "ok",
        "",
    );
    state.iam_audit_log.record(
        "admin",
        "group.upsert",
        None,
        Some("ops".to_owned()),
        "ok",
        "",
    );

    let response = auth::audit_log_handler(
        State(state.clone()),
        Query(AuditLogQuery {
            user: Some("alice".to_owned()),
            lines: Some(50),
        }),
    )
    .await;

    let entries = response.0;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].target_user.as_deref(), Some("alice"));
}

#[tokio::test]
async fn audit_log_handler_filters_by_actor() {
    let state = build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    );
    state.iam_audit_log.record(
        "alice",
        "login.stage",
        Some("alice".to_owned()),
        None,
        "ok",
        "",
    );
    state.iam_audit_log.record(
        "carol",
        "login.stage",
        Some("carol".to_owned()),
        None,
        "ok",
        "",
    );

    let response = auth::audit_log_handler(
        State(state),
        Query(AuditLogQuery {
            user: Some("alice".to_owned()),
            lines: Some(50),
        }),
    )
    .await;

    assert_eq!(response.0.len(), 1);
    assert_eq!(response.0[0].actor, "alice");
}

#[tokio::test]
async fn audit_log_handler_clamps_lines_to_max_500() {
    let state = build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    );
    for index in 0..600 {
        state.iam_audit_log.record(
            "alice",
            format!("event.{index}"),
            Some("alice".to_owned()),
            None,
            "ok",
            "",
        );
    }

    let response = auth::audit_log_handler(
        State(state),
        Query(AuditLogQuery {
            user: None,
            lines: Some(10_000),
        }),
    )
    .await;

    assert_eq!(response.0.len(), MAX_TAIL);
}

#[tokio::test]
async fn audit_log_handler_default_returns_50_entries() {
    let state = build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    );
    for index in 0..120 {
        state.iam_audit_log.record(
            "alice",
            format!("event.{index}"),
            Some("alice".to_owned()),
            None,
            "ok",
            "",
        );
    }

    let response = auth::audit_log_handler(
        State(state),
        Query(AuditLogQuery {
            user: None,
            lines: None,
        }),
    )
    .await;

    assert_eq!(response.0.len(), 50);
}

#[tokio::test]
async fn audit_log_handler_returns_newest_first() {
    let state = build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    );
    state.iam_audit_log.record(
        "admin",
        "user.disable",
        Some("alice".to_owned()),
        None,
        "ok",
        "",
    );
    state.iam_audit_log.record(
        "admin",
        "group.upsert",
        None,
        Some("ops".to_owned()),
        "ok",
        "",
    );

    let response = auth::audit_log_handler(
        State(state),
        Query(AuditLogQuery {
            user: None,
            lines: Some(50),
        }),
    )
    .await;

    assert_eq!(response.0.len(), 2);
    assert_eq!(response.0[0].action, "group.upsert");
    assert_eq!(response.0[1].action, "user.disable");
}

#[tokio::test]
async fn audit_log_handler_records_outcomes_distinctly() {
    let state = build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    );
    state.iam_audit_log.record(
        "admin",
        "user.delete",
        Some("alice".to_owned()),
        None,
        "ok",
        "",
    );
    state.iam_audit_log.record(
        "alice",
        "user.delete",
        Some("alice".to_owned()),
        None,
        "error",
        "self delete refused",
    );

    let response = auth::audit_log_handler(
        State(state),
        Query(AuditLogQuery {
            user: Some("alice".to_owned()),
            lines: Some(50),
        }),
    )
    .await;

    let entries: Vec<IamAuditEntry> = response.0;
    assert_eq!(entries.len(), 2);
    let outcomes: Vec<&str> = entries.iter().map(|e| e.outcome.as_str()).collect();
    assert!(outcomes.contains(&"ok"));
    assert!(outcomes.contains(&"error"));
    assert!(entries
        .iter()
        .any(|entry| entry.detail.contains("self delete refused")));
}

// =====================================================================
// IamUserSummaryResponse serialization — guarantees the wire format the
// frontend depends on stays stable.
// =====================================================================

#[test]
fn user_summary_response_serializes_to_camel_case() {
    let summary = IamUserSummaryResponse {
        username: "alice".to_owned(),
        first_name: "Alice".to_owned(),
        last_name: "Mesh".to_owned(),
        roles: vec!["admin".to_owned()],
        scopes: vec!["scope:a".to_owned()],
        groups: vec!["platform-admins".to_owned()],
        disabled_at: Some(1_700_000_000),
        created_at: 1_699_000_000,
        last_login_at: Some(1_700_001_000),
    };
    let serialized = serde_json::to_value(&summary).expect("serialization should succeed");
    assert_eq!(serialized["firstName"], "Alice");
    assert_eq!(serialized["lastName"], "Mesh");
    assert_eq!(serialized["disabledAt"], 1_700_000_000);
    assert_eq!(serialized["createdAt"], 1_699_000_000);
    assert_eq!(serialized["lastLoginAt"], 1_700_001_000);
}
