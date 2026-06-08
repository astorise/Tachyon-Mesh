use super::support_and_cache::*;
use crate::auth::{AuditLogQuery, AuthClaims, IamUserSummaryResponse, UpdateUserRequest};
use crate::iam_audit::{IamAuditEntry, MAX_TAIL};
use axum::extract::{Path, Query, State};
#[cfg(feature = "experimental")]
use axum::http::header::AUTHORIZATION;
#[cfg(feature = "experimental")]
use axum::http::HeaderMap;
use axum::Extension;
use std::sync::Arc;

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

fn admin_claims() -> AuthClaims {
    AuthClaims {
        subject: "admin".to_owned(),
        roles: vec!["admin".to_owned()],
        scopes: Vec::new(),
    }
}

#[tokio::test]
async fn iam_handlers_surface_missing_authn_guest_errors() {
    let mut state = build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    );
    state.auth_manager = Arc::new(auth::test_auth_manager_with_modules(
        "missing-authn",
        "missing-authz",
        unique_test_dir("missing-authn-state"),
    ));
    let claims = admin_claims();

    let users = auth::list_users_handler(State(state.clone()), Extension(claims.clone())).await;
    assert!(
        users.is_err(),
        "list_users_handler must not synthesize an empty success response"
    );

    let groups = auth::list_groups_handler(State(state.clone()), Extension(claims.clone())).await;
    assert!(
        groups.is_err(),
        "list_groups_handler must not synthesize an empty success response"
    );

    let delete_user = auth::delete_user_handler(
        State(state.clone()),
        Extension(claims.clone()),
        Path("alice".to_owned()),
    )
    .await;
    assert!(
        delete_user.is_err(),
        "delete_user_handler must propagate authn delete errors"
    );

    let delete_group = auth::delete_group_handler(
        State(state),
        Extension(claims),
        Path("platform-admins".to_owned()),
    )
    .await;
    assert!(
        delete_group.is_err(),
        "delete_group_handler must propagate authn delete errors"
    );
}

#[tokio::test]
async fn update_user_handler_records_action_for_requested_state_change() {
    for (disabled, expected_action) in [
        (Some(true), "user.disable"),
        (Some(false), "user.enable"),
        (None, "user.update"),
    ] {
        let mut state = build_test_state(
            IntegrityConfig::default_sealed(),
            telemetry::init_test_telemetry(),
        );
        state.auth_manager = Arc::new(auth::test_auth_manager_with_modules(
            "missing-authn",
            "missing-authz",
            unique_test_dir("missing-authn-state"),
        ));

        let result = auth::update_user_handler(
            State(state.clone()),
            Extension(admin_claims()),
            Path("alice".to_owned()),
            axum::Json(UpdateUserRequest {
                add_groups: None,
                remove_groups: None,
                add_roles: None,
                remove_roles: None,
                add_scopes: None,
                remove_scopes: None,
                disabled,
            }),
        )
        .await;

        assert!(result.is_err());
        let entries = state.iam_audit_log.snapshot(Some("alice"), 10);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, expected_action);
        assert_eq!(entries[0].outcome, "error");
    }
}

#[tokio::test]
async fn admin_status_handler_reports_runtime_counts() {
    let state = build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    );

    let body = auth::admin_status_handler(State(state)).await;

    assert!(body.contains("routes="));
    assert!(body.contains("batch_targets="));
    assert!(body.contains("status=ready"));
}

#[cfg(feature = "experimental")]
#[tokio::test]
async fn authorize_admin_headers_only_handles_admin_paths_and_propagates_auth_errors() {
    let mut state = build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    );
    state.auth_manager = Arc::new(auth::test_auth_manager_with_modules(
        "missing-authn",
        "missing-authz",
        unique_test_dir("missing-authn-state"),
    ));

    let headers = HeaderMap::new();
    assert!(
        auth::authorize_admin_headers(&state, "GET", "/api/user", &headers)
            .await
            .is_none(),
        "non-admin paths must bypass this helper"
    );

    let response = auth::authorize_admin_headers(&state, "GET", "/admin/status", &headers)
        .await
        .expect("missing bearer should produce a response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, "Bearer token".parse().expect("header"));
    let response = auth::authorize_admin_headers(&state, "GET", "/admin/status", &headers)
        .await
        .expect("authn failure should produce a response");
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}
