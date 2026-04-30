    #[test]
    fn split_guest_stdout_removes_json_log_lines() {
        let stdout = Bytes::from(
            "{\"level\":\"INFO\",\"target\":\"guest_example\",\"fields\":{\"message\":\"guest-example received a request payload\"}}\nFaaS received: Hello Lean FaaS!\n",
        );

        let response = split_guest_stdout("guest-example", stdout);

        assert_eq!(
            String::from_utf8_lossy(&response),
            "FaaS received: Hello Lean FaaS!\n"
        );
    }

    #[test]
    fn verify_integrity_signature_accepts_valid_material() {
        let payload = canonical_config_payload(&IntegrityConfig::default_sealed())
            .expect("payload should serialize");
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let signature = signing_key.sign(&Sha256::digest(payload.as_bytes()));

        verify_integrity_signature(
            &payload,
            &hex::encode(signing_key.verifying_key().to_bytes()),
            &hex::encode(signature.to_bytes()),
        )
        .expect("signature should verify");
    }

    #[test]
    fn verify_integrity_signature_rejects_tampered_payload() {
        let payload = canonical_config_payload(&IntegrityConfig::default_sealed())
            .expect("payload should serialize");
        let signing_key = SigningKey::from_bytes(&[9_u8; 32]);
        let signature = signing_key.sign(&Sha256::digest(payload.as_bytes()));

        let error = verify_integrity_signature(
            "{\"tampered\":true}",
            &hex::encode(signing_key.verifying_key().to_bytes()),
            &hex::encode(signature.to_bytes()),
        )
        .expect_err("tampered payload should fail verification");

        assert!(
            error.to_string().contains("Integrity Validation Failed"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn system_runtime_environment_only_exposes_host_public_key_to_system_routes() {
        let host_identity = test_host_identity(12);
        let mut system_route = IntegrityRoute::system("/system/storage-broker");
        system_route
            .env
            .insert("QUEUE_URL".to_owned(), "http://queue.local/mock".to_owned());
        let system_env = system_runtime_environment(&system_route, &host_identity);
        let user_env =
            system_runtime_environment(&IntegrityRoute::user("/api/guest"), &host_identity);

        assert_eq!(
            system_env,
            vec![
                ("QUEUE_URL".to_owned(), "http://queue.local/mock".to_owned()),
                (
                    TACHYON_SYSTEM_PUBLIC_KEY_ENV.to_owned(),
                    host_identity.public_key_hex.clone()
                )
            ]
        );
        assert!(user_env.is_empty());
    }

    fn signed_manifest_for(config: &IntegrityConfig, signing_key: &SigningKey) -> Vec<u8> {
        let payload = canonical_config_payload(config).expect("payload should serialize");
        let signature = signing_key.sign(&Sha256::digest(payload.as_bytes()));
        let manifest = IntegrityManifest {
            config_payload: payload,
            public_key: hex::encode(signing_key.verifying_key().to_bytes()),
            signature: hex::encode(signature.to_bytes()),
        };
        serde_json::to_vec(&manifest).expect("manifest should serialize")
    }

    #[tokio::test]
    async fn admin_manifest_update_accepts_higher_version_and_emits_outbox_event() {
        let mut current = IntegrityConfig::default_sealed();
        current.config_version = 1;
        let telemetry = telemetry::init_test_telemetry();
        let state = build_test_state(current.clone(), telemetry);

        let mut next = current.clone();
        next.config_version = 7;
        let signing_key = SigningKey::from_bytes(&[42_u8; 32]);
        let body = Bytes::from(signed_manifest_for(&next, &signing_key));

        let response = admin_manifest_update_handler(State(state.clone()), body).await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        // The outbox should now hold exactly one event for the new version.
        let rows = state
            .core_store
            .peek_outbox(store::CoreStoreBucket::ConfigUpdateOutbox, 16)
            .expect("peek outbox");
        assert_eq!(rows.len(), 1);
        let event: ConfigUpdateEvent =
            serde_json::from_slice(&rows[0].1).expect("event payload parses");
        assert_eq!(event.version, 7);
        assert_eq!(event.origin_node_id, state.host_identity.public_key_hex);
        assert!(event.checksum.starts_with("sha256:"));
    }

    #[tokio::test]
    async fn admin_manifest_update_rejects_rollback() {
        let mut current = IntegrityConfig::default_sealed();
        current.config_version = 9;
        let telemetry = telemetry::init_test_telemetry();
        let state = build_test_state(current.clone(), telemetry);

        let mut older = current.clone();
        older.config_version = 5;
        let signing_key = SigningKey::from_bytes(&[42_u8; 32]);
        let body = Bytes::from(signed_manifest_for(&older, &signing_key));

        let response = admin_manifest_update_handler(State(state.clone()), body).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);

        // Outbox stays empty on rejection.
        let rows = state
            .core_store
            .peek_outbox(store::CoreStoreBucket::ConfigUpdateOutbox, 16)
            .expect("peek outbox");
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn enrollment_start_then_approve_then_poll_round_trips() {
        let telemetry = telemetry::init_test_telemetry();
        let state = build_test_state(IntegrityConfig::default_sealed(), telemetry);

        let start = admin_enrollment_start_handler(
            State(state.clone()),
            axum::Json(AdminEnrollmentStartRequest {
                node_public_key: "deadbeef".to_owned(),
            }),
        )
        .await;
        assert_eq!(start.status(), StatusCode::CREATED);
        let body_bytes = axum::body::to_bytes(start.into_body(), 16 * 1024)
            .await
            .expect("body collects");
        let start_body: AdminEnrollmentStartResponse =
            serde_json::from_slice(&body_bytes).expect("response is JSON");

        // Wrong PIN — caller error.
        let bad = admin_enrollment_approve_handler(
            State(state.clone()),
            axum::Json(AdminEnrollmentApproveRequest {
                session_id: start_body.session_id.clone(),
                pin: "BAD-PIN".to_owned(),
                signed_certificate_hex: "01020304".to_owned(),
            }),
        )
        .await;
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

        // Right PIN — accepted.
        let approve = admin_enrollment_approve_handler(
            State(state.clone()),
            axum::Json(AdminEnrollmentApproveRequest {
                session_id: start_body.session_id.clone(),
                pin: start_body.pin.clone(),
                signed_certificate_hex: "01020304".to_owned(),
            }),
        )
        .await;
        assert_eq!(approve.status(), StatusCode::ACCEPTED);

        // Pending node polls and gets the cert; subsequent polls return None
        // because the session is consumed.
        let poll = admin_enrollment_poll_handler(
            State(state.clone()),
            axum::extract::Path(start_body.session_id.clone()),
        )
        .await;
        assert_eq!(poll.status(), StatusCode::OK);
        let cert_bytes = axum::body::to_bytes(poll.into_body(), 1024)
            .await
            .expect("poll response body should be readable");
        assert_eq!(cert_bytes.as_ref(), b"01020304");

        let poll_again = admin_enrollment_poll_handler(
            State(state.clone()),
            axum::extract::Path(start_body.session_id),
        )
        .await;
        assert_eq!(poll_again.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn admin_manifest_update_rejects_tampered_signature() {
        let mut current = IntegrityConfig::default_sealed();
        current.config_version = 1;
        let telemetry = telemetry::init_test_telemetry();
        let state = build_test_state(current.clone(), telemetry);

        let mut next = current.clone();
        next.config_version = 7;
        let signing_key = SigningKey::from_bytes(&[42_u8; 32]);
        let mut bytes = signed_manifest_for(&next, &signing_key);
        // Flip a byte inside the JSON payload — signature no longer matches.
        let pos = bytes
            .iter()
            .position(|b| *b == b'1')
            .expect("contains a '1'");
        bytes[pos] = b'2';
        let response =
            admin_manifest_update_handler(State(state.clone()), Bytes::from(bytes)).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn trace_context_honors_well_formed_inbound_traceparent() {
        let mut headers = HeaderMap::new();
        let inbound = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
        headers.insert(
            "traceparent",
            HeaderValue::from_str(inbound).expect("traceparent value is valid ASCII"),
        );
        assert_eq!(trace_context_for_request(&headers), inbound);
    }

    #[test]
    fn trace_context_rejects_malformed_inbound_and_mints_fresh() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "traceparent",
            HeaderValue::from_static("not a valid traceparent"),
        );
        let value = trace_context_for_request(&headers);
        assert!(
            is_valid_w3c_traceparent(&value),
            "minted traceparent must be a valid W3C value, got `{value}`"
        );
    }

    #[test]
    fn trace_context_mints_fresh_when_header_absent() {
        let value = trace_context_for_request(&HeaderMap::new());
        assert!(
            is_valid_w3c_traceparent(&value),
            "minted traceparent must be valid, got `{value}`"
        );
        // Smoke-check that two consecutive mints differ — a sanity check on entropy.
        let other = trace_context_for_request(&HeaderMap::new());
        assert_ne!(value, other);
    }

    #[test]
    fn host_identity_round_trips_signed_route_claims() {
        let host_identity = test_host_identity(13);
        let route = IntegrityRoute::user("/api/tenant-a");
        let token = host_identity
            .sign_route(&route)
            .expect("identity token should sign");

        let claims = host_identity
            .verify_token(&token)
            .expect("identity token should verify");

        assert_eq!(claims.route_path, "/api/tenant-a");
        assert_eq!(claims.role, RouteRole::User);
        assert!(claims.expires_at >= claims.issued_at);
    }

    #[test]
    fn host_identity_rejects_expired_tokens() {
        let host_identity = test_host_identity(14);
        let now = unix_timestamp_seconds().expect("system clock should be available");
        let token = host_identity
            .sign_claims(&CallerIdentityClaims {
                route_path: "/api/tenant-a".to_owned(),
                role: RouteRole::User,
                tenant_id: None,
                token_id: None,
                issued_at: now.saturating_sub(10),
                expires_at: now.saturating_sub(1),
            })
            .expect("expired identity token should still sign");

        let error = host_identity
            .verify_token(&token)
            .expect_err("expired identity token should be rejected");

        assert!(error.contains("expired"), "unexpected error: {error}");
    }
