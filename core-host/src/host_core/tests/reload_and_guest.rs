use super::support_and_cache::*;
use crate::*;

#[tokio::test]
async fn reload_runtime_from_disk_swaps_in_new_routes() {
    let temp_dir = unique_test_dir("graceful-reload-state");
    let manifest_path = temp_dir.join("integrity.lock");
    let initial = IntegrityConfig {
        routes: vec![IntegrityRoute::user(DEFAULT_ROUTE)],
        ..IntegrityConfig::default_sealed()
    };
    write_test_manifest(&manifest_path, &initial, 11);
    let state = build_test_state_with_manifest(
        initial,
        telemetry::init_test_telemetry(),
        manifest_path.clone(),
    );

    let mut reloaded = IntegrityConfig {
        routes: vec![IntegrityRoute::user(DEFAULT_ROUTE)],
        ..IntegrityConfig::default_sealed()
    };
    reloaded
        .routes
        .push(IntegrityRoute::user("/api/guest-loop"));
    write_test_manifest(&manifest_path, &reloaded, 12);

    reload_runtime_from_disk(&state)
        .await
        .expect("runtime should reload from manifest");

    let runtime = state.runtime.load_full();
    assert!(runtime.config.sealed_route("/api/guest-loop").is_some());
    assert!(runtime.concurrency_limits.contains_key("/api/guest-loop"));
}

#[tokio::test]
async fn reload_runtime_from_disk_clears_cached_route_identity_tokens() {
    let temp_dir = unique_test_dir("graceful-reload-identity-cache");
    let manifest_path = temp_dir.join("integrity.lock");
    let initial = IntegrityConfig {
        routes: vec![IntegrityRoute::user(DEFAULT_ROUTE)],
        ..IntegrityConfig::default_sealed()
    };
    write_test_manifest(&manifest_path, &initial, 41);
    let state = build_test_state_with_manifest(
        initial,
        telemetry::init_test_telemetry(),
        manifest_path.clone(),
    );
    state
        .host_identity
        .sign_route(&IntegrityRoute::user(DEFAULT_ROUTE))
        .expect("identity token should sign");
    assert_eq!(
        state
            .host_identity
            .route_token_cache_len()
            .expect("cache length should read"),
        1
    );

    let reloaded = IntegrityConfig {
        routes: vec![IntegrityRoute::user(DEFAULT_ROUTE)],
        ..IntegrityConfig::default_sealed()
    };
    write_test_manifest(&manifest_path, &reloaded, 42);
    reload_runtime_from_disk(&state)
        .await
        .expect("runtime should reload from manifest");

    assert_eq!(
        state
            .host_identity
            .route_token_cache_len()
            .expect("cache length should read"),
        0
    );
}

#[tokio::test]
async fn reload_runtime_from_disk_keeps_previous_state_on_invalid_manifest() {
    let temp_dir = unique_test_dir("graceful-reload-invalid");
    let manifest_path = temp_dir.join("integrity.lock");
    let initial = IntegrityConfig {
        routes: vec![IntegrityRoute::user(DEFAULT_ROUTE)],
        ..IntegrityConfig::default_sealed()
    };
    write_test_manifest(&manifest_path, &initial, 13);
    let state = build_test_state_with_manifest(
        initial,
        telemetry::init_test_telemetry(),
        manifest_path.clone(),
    );

    fs::write(&manifest_path, "{ invalid json").expect("invalid manifest should be written");

    let error = reload_runtime_from_disk(&state)
        .await
        .expect_err("invalid manifest should not replace the runtime");

    assert!(error
        .to_string()
        .contains("failed to parse integrity manifest"));
    let runtime = state.runtime.load_full();
    assert!(runtime.config.sealed_route(DEFAULT_ROUTE).is_some());
    assert!(runtime.config.sealed_route("/api/guest-loop").is_none());
}

#[tokio::test]
async fn reload_runtime_from_disk_drains_previous_generation_until_response_flush() {
    let temp_dir = unique_test_dir("graceful-drain");
    let manifest_path = temp_dir.join("integrity.lock");
    let initial = IntegrityConfig {
        routes: vec![draining_test_route("guest-flaky", "1.0.0")],
        ..IntegrityConfig::default_sealed()
    };
    write_test_manifest(&manifest_path, &initial, 31);
    let state = build_test_state_with_manifest(
        initial,
        telemetry::init_test_telemetry(),
        manifest_path.clone(),
    );
    let app = build_app(state.clone());

    let slow_request = {
        let app = app.clone();
        tokio::spawn(async move {
            app.oneshot(
                Request::post("/api/drain")
                    .body(Body::from("sleep:250"))
                    .expect("slow request should build"),
            )
            .await
            .expect("slow request should complete")
        })
    };

    tokio::time::sleep(Duration::from_millis(50)).await;
    let initial_runtime = state.runtime.load_full();
    let initial_control = initial_runtime
        .concurrency_limits
        .get("/api/drain")
        .cloned()
        .expect("initial route control should exist");
    assert_eq!(initial_control.active_request_count(), 1);
    drop(initial_runtime);

    let reloaded = IntegrityConfig {
        routes: vec![draining_test_route("guest-example", "2.0.0")],
        ..IntegrityConfig::default_sealed()
    };
    write_test_manifest(&manifest_path, &reloaded, 32);
    reload_runtime_from_disk(&state)
        .await
        .expect("runtime should reload from manifest");

    let fresh_response = app
        .clone()
        .oneshot(
            Request::post("/api/drain")
                .body(Body::from("hello-v2"))
                .expect("fresh request should build"),
        )
        .await
        .expect("fresh request should complete");
    let fresh_body = fresh_response
        .into_body()
        .collect()
        .await
        .expect("fresh response body should collect")
        .to_bytes();
    assert!(
        String::from_utf8_lossy(&fresh_body).contains("FaaS received: hello-v2"),
        "unexpected fresh body: {:?}",
        fresh_body
    );

    let slow_response = slow_request
        .await
        .expect("slow request task should join cleanly");
    assert_eq!(
        state
            .draining_runtimes
            .lock()
            .expect("draining runtime list should not be poisoned")
            .len(),
        1
    );
    run_draining_runtime_reaper_tick(&state);
    assert_eq!(
        state
            .draining_runtimes
            .lock()
            .expect("draining runtime list should not be poisoned")
            .len(),
        1,
        "the old generation should remain while its response body is still owned"
    );
    assert_eq!(initial_control.active_request_count(), 1);

    let slow_body = slow_response
        .into_body()
        .collect()
        .await
        .expect("slow response body should collect")
        .to_bytes();
    assert_eq!(slow_body, Bytes::from_static(b"slept:250"));
    run_draining_runtime_reaper_tick(&state);
    assert_eq!(initial_control.active_request_count(), 0);
    assert_eq!(
        state
            .draining_runtimes
            .lock()
            .expect("draining runtime list should not be poisoned")
            .len(),
        0,
        "the old generation should be reaped once the response finishes flushing"
    );
}

#[test]
fn draining_runtime_reaper_forces_timeout_after_deadline() {
    let state = build_test_state(
        IntegrityConfig {
            routes: vec![draining_test_route("guest-flaky", "1.0.0")],
            ..IntegrityConfig::default_sealed()
        },
        telemetry::init_test_telemetry(),
    );
    let runtime = state.runtime.load_full();
    let control = runtime
        .concurrency_limits
        .get("/api/drain")
        .cloned()
        .expect("route control should exist");
    let _guard = control.begin_request();
    let draining_since = Instant::now()
        .checked_sub(DRAINING_ROUTE_TIMEOUT + Duration::from_secs(1))
        .expect("deadline subtraction should remain valid");
    runtime.mark_draining(draining_since);
    state
        .draining_runtimes
        .lock()
        .expect("draining runtime list should not be poisoned")
        .push(DrainingRuntime {
            runtime,
            draining_since,
        });

    run_draining_runtime_reaper_tick(&state);

    assert_eq!(
        state
            .draining_runtimes
            .lock()
            .expect("draining runtime list should not be poisoned")
            .len(),
        0
    );
    assert!(control.semaphore.is_closed());
}

#[tokio::test]
async fn run_mode_executes_gc_batch_target_and_deletes_stale_files() {
    let temp_dir = unique_test_dir("batch-gc");
    let cache_dir = temp_dir.join("cache");
    fs::create_dir_all(cache_dir.join("nested")).expect("cache directory should exist");
    let stale_file = cache_dir.join("nested").join("stale.txt");
    fs::write(&stale_file, "stale").expect("stale file should be written");

    let manifest_path = temp_dir.join("integrity.lock");
    let mut config = IntegrityConfig::default_sealed();
    config.routes.clear();
    config.batch_targets = vec![gc_batch_target(&cache_dir, 0)];
    write_test_manifest(&manifest_path, &config, 14);

    let success = execute_batch_target_from_manifest(manifest_path, "gc-job")
        .await
        .expect("batch target should execute successfully");

    assert!(success, "batch target should exit successfully");
    assert!(
        !stale_file.exists(),
        "batch GC target should delete stale files"
    );
}

#[test]
fn execute_guest_returns_component_response_payload() {
    let config = IntegrityConfig::default_sealed();
    let engine = build_test_engine(&config);
    let route = config
        .sealed_route("/api/guest-example")
        .expect("sealed route should exist")
        .clone();
    let response = execute_guest(
        &engine,
        "guest-example",
        GuestRequest::new("POST", "/api/guest-example", "Hello Lean FaaS!"),
        &route,
        test_guest_execution_context_with_secret_access(
            config,
            30,
            SecretAccess::from_route(&route, &SecretsVault::load()),
        ),
    )
    .expect("guest execution should succeed");

    assert_eq!(
        response,
        GuestExecutionOutcome {
            output: GuestExecutionOutput::Http(GuestHttpResponse::new(
                StatusCode::OK,
                Bytes::from(expected_guest_example_body(
                    "FaaS received: Hello Lean FaaS!"
                )),
            )),
            fuel_consumed: None,
        }
    );
}

#[test]
fn execute_guest_falls_back_to_legacy_stdout_for_non_component_module() {
    let config = IntegrityConfig::default_sealed();
    let engine = build_test_engine(&config);
    let route = IntegrityRoute::user("/api/guest-call-legacy");
    let response = execute_guest(
        &engine,
        "guest-call-legacy",
        GuestRequest::new("GET", "/api/guest-call-legacy", Bytes::new()),
        &route,
        test_guest_execution_context(config, 31),
    )
    .expect("legacy guest execution should succeed");

    assert_eq!(
        response,
        GuestExecutionOutcome {
            output: GuestExecutionOutput::LegacyStdout(Bytes::from("legacy guest stdout\n")),
            fuel_consumed: None,
        }
    );
}

#[test]
fn execute_legacy_guest_reads_stdin_for_tcp_echo_module() {
    let config = IntegrityConfig::default_sealed();
    let engine = build_test_engine(&config);
    let route = tcp_echo_test_route(1);
    let response = execute_guest(
        &engine,
        "guest-tcp-echo",
        GuestRequest::new(
            "TCP",
            "tcp://guest-tcp-echo",
            Bytes::from_static(b"ping over tcp"),
        ),
        &route,
        test_guest_execution_context(config, 32),
    )
    .expect("legacy guest execution should succeed");

    assert_eq!(
        response,
        GuestExecutionOutcome {
            output: GuestExecutionOutput::LegacyStdout(Bytes::from_static(b"ping over tcp")),
            fuel_consumed: None,
        }
    );
}

#[cfg(feature = "ai-inference")]
#[test]
fn execute_guest_ai_uses_preloaded_model_alias_and_returns_mock_text() {
    let mut route = IntegrityRoute::user("/api/guest-ai");
    route.models = vec![IntegrityModelBinding {
        alias: "llama3".to_owned(),
        path: "mock:llama3".to_owned(),
        device: ModelDevice::Cuda,
        qos: RouteQos::Standard,
        dynamic: false,
        hardware_strategy: Default::default(),
    }];
    let config = IntegrityConfig {
        routes: vec![route.clone()],
        ..IntegrityConfig::default_sealed()
    };
    let engine = build_test_engine(&config);

    let response = execute_guest(
            &engine,
            "guest-ai",
            GuestRequest::new(
                "POST",
                "/api/guest-ai",
                Bytes::from_static(
                    br#"{"model":"llama3","shape":[1,4],"values":[1.0,2.0,3.0,4.0],"output_len":17,"response_kind":"text"}"#,
                ),
            ),
            &route,
            test_guest_execution_context(config, 35),
        )
        .expect("AI guest execution should succeed");

    let GuestExecutionOutcome {
        output: GuestExecutionOutput::LegacyStdout(stdout),
        ..
    } = response
    else {
        unreachable!("AI guest should return legacy stdout");
    };

    let payload: Value = serde_json::from_slice(&stdout).expect("guest response should be JSON");
    assert_eq!(payload["model"], Value::String("llama3".to_owned()));
    assert_eq!(
        payload["text"],
        Value::String("MOCK_LLM_RESPONSE".to_owned())
    );
    assert_eq!(payload["output_bytes"], Value::from(17));
}

#[test]
fn execute_guest_persists_volume_data_for_component_guest() {
    let volume_dir = unique_test_dir("tachyon-volume-test");
    let route = volume_test_route(&volume_dir, false);
    let config = IntegrityConfig {
        routes: vec![route.clone()],
        ..IntegrityConfig::default_sealed()
    };
    let engine = build_test_engine(&config);
    let save_response = execute_guest(
        &engine,
        "guest-volume",
        GuestRequest::new("POST", "/api/guest-volume", "Hello Stateful World"),
        &route,
        test_guest_execution_context(config.clone(), 32),
    )
    .expect("volume guest should write successfully");

    assert_eq!(
        save_response,
        GuestExecutionOutcome {
            output: GuestExecutionOutput::Http(GuestHttpResponse::new(StatusCode::OK, "Saved",)),
            fuel_consumed: None,
        }
    );

    let read_response = execute_guest(
        &engine,
        "guest-volume",
        GuestRequest::new("GET", "/api/guest-volume", Bytes::new()),
        &route,
        test_guest_execution_context(config.clone(), 33),
    )
    .expect("volume guest should read successfully");

    assert_eq!(
        read_response,
        GuestExecutionOutcome {
            output: GuestExecutionOutput::Http(GuestHttpResponse::new(
                StatusCode::OK,
                "Hello Stateful World",
            )),
            fuel_consumed: None,
        }
    );
    assert_eq!(
        fs::read_to_string(volume_dir.join("state.txt")).expect("host volume file should exist"),
        "Hello Stateful World"
    );

    let _ = fs::remove_dir_all(volume_dir);
}

/// Integration test: `guest-openai` with `stream: true` delivers incremental
/// SSE frames whose deltas concatenate to the non-streamed response content.
/// Both the buffered reference and the streaming call go through
/// `execute_streaming_guest` so the same linker path is used for both.
/// Skipped automatically when the wasm32-wasip2 artifact is not present
/// (requires `cargo build -p guest-openai --target wasm32-wasip2` first).
#[cfg(feature = "ai-inference")]
#[tokio::test]
async fn streaming_guest_openai_sse_deltas_reconstruct_buffered_output() {
    let mut route = IntegrityRoute::user("/ai/v1/chat/completions");
    route.models = vec![IntegrityModelBinding {
        alias: "llama3".to_owned(),
        path: "mock:llama3".to_owned(),
        device: ModelDevice::Cpu,
        qos: RouteQos::Standard,
        dynamic: false,
        hardware_strategy: Default::default(),
    }];
    let config = IntegrityConfig {
        routes: vec![route.clone()],
        ..IntegrityConfig::default_sealed()
    };
    let engine = build_test_engine(&config);
    // Skip if the wasm component was not built — unless a CI run pins
    // `TACHYON_REQUIRE_GUEST_OPENAI`, in which case a missing artifact is a
    // hard failure (a silent skip would be a false green).
    if let Err(missing) = resolve_guest_module_path("guest-openai") {
        assert!(
            std::env::var_os("TACHYON_REQUIRE_GUEST_OPENAI").is_none(),
            "guest-openai wasm artifact required but not found ({missing}); \
             build it with `cargo build -p guest-openai --target wasm32-wasip2 --release`"
        );
        eprintln!("SKIP: guest-openai artifact not present; run `cargo build -p guest-openai --target wasm32-wasip2` first");
        return;
    }

    let messages = serde_json::json!([{"role": "user", "content": "ping"}]);

    // Helper: run execute_streaming_guest, return (status, headers, body_string).
    let run_streaming = |body_json: serde_json::Value, seed: u8| {
        let route_c = route.clone();
        let engine_c = engine.clone();
        let config_c = config.clone();
        let req = GuestRequest::new(
            "POST",
            "/ai/v1/chat/completions",
            Bytes::from(serde_json::to_vec(&body_json).expect("encode")),
        );
        let (htx, hrx) = tokio::sync::oneshot::channel::<(StatusCode, GuestHttpFields)>();
        let (ctx, crx) = tokio::sync::mpsc::channel::<Bytes>(64);
        std::thread::spawn(move || {
            execute_streaming_guest(
                &engine_c,
                &route_c,
                "guest-openai",
                req,
                htx,
                ctx,
                &test_guest_execution_context(config_c, seed),
            );
        });
        (hrx, crx)
    };

    // ── Buffered reference: no `stream` field → fallback path sends JSON body ──
    // The streaming execution path is used for both calls; the buffered case
    // uses the channel-based fallback (guest returns normally without calling
    // get-streaming-response).
    let (hrx_ref, mut crx_ref) = run_streaming(
        serde_json::json!({"model": "llama3", "messages": messages}),
        91,
    );
    let (ref_status, ref_headers) = hrx_ref.await.expect("reference headers should arrive");
    let mut ref_body_bytes = Vec::new();
    while let Some(chunk) = crx_ref.recv().await {
        ref_body_bytes.extend_from_slice(&chunk);
    }
    assert_eq!(
        ref_status,
        StatusCode::OK,
        "buffered reference returned non-200; headers={ref_headers:?} body={}",
        String::from_utf8_lossy(&ref_body_bytes)
    );
    let ref_json: Value =
        serde_json::from_slice(&ref_body_bytes).expect("buffered reference should be JSON");
    let buffered_content = ref_json["choices"][0]["message"]["content"]
        .as_str()
        .expect("buffered reference must have content")
        .to_owned();

    // Unlike a stream, a buffered response has no extra frame to break a naive
    // client with, so usage is unconditional here — OpenAI reports it the same
    // way. The counts come back beside the text through `compute-detailed`.
    let buffered_usage = &ref_json["usage"];
    assert!(
        !buffered_usage.is_null(),
        "a buffered response must report usage: {ref_json}"
    );
    let buffered_prompt_tokens = buffered_usage["prompt_tokens"]
        .as_u64()
        .expect("prompt_tokens");
    let buffered_completion_tokens = buffered_usage["completion_tokens"]
        .as_u64()
        .expect("completion_tokens");
    assert!(
        buffered_prompt_tokens > 0 && buffered_completion_tokens > 0,
        "buffered usage must be measured, not defaulted: {buffered_usage}"
    );
    assert_eq!(
        buffered_usage["total_tokens"].as_u64(),
        Some(buffered_prompt_tokens + buffered_completion_tokens)
    );

    // ── Streaming call: stream: true → SSE path ──────────────────────────
    let (hrx_s, mut crx_s) = run_streaming(
        serde_json::json!({"model": "llama3", "messages": messages, "stream": true}),
        92,
    );

    // Headers must arrive before any body bytes (true TTFT signal).
    let (stream_status, stream_headers) = hrx_s.await.expect("streaming headers should arrive");
    assert_eq!(stream_status, StatusCode::OK);
    assert!(
        stream_headers
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("content-type") && v.contains("text/event-stream")),
        "streaming response must carry content-type: text/event-stream; got: {stream_headers:?}"
    );

    let mut sse_body = String::new();
    while let Some(chunk) = crx_s.recv().await {
        sse_body.push_str(&String::from_utf8_lossy(&chunk));
    }

    assert!(
        sse_body.contains("data: [DONE]"),
        "SSE stream must end with data: [DONE]\n{sse_body:?}"
    );

    // Concatenated deltas == buffered content.
    let delta_content: String = sse_body
        .lines()
        .filter(|l| l.starts_with("data: ") && !l.contains("[DONE]"))
        .filter_map(|l| {
            let v: Value = serde_json::from_str(&l["data: ".len()..]).ok()?;
            v["choices"][0]["delta"]["content"]
                .as_str()
                .map(str::to_owned)
        })
        .collect();

    assert_eq!(
        delta_content, buffered_content,
        "concatenated SSE deltas must equal the buffered response content"
    );

    // ── Usage is opt-in, exactly as it is on OpenAI ──────────────────────
    // Without `stream_options.include_usage`, no chunk may carry a `usage`
    // object: an extra trailing chunk with an empty `choices` array breaks
    // clients that index `choices[0]` unconditionally, which is why OpenAI
    // gates it too.
    for frame in sse_data_frames(&sse_body) {
        assert!(
            frame.get("usage").is_none_or(Value::is_null),
            "an unrequested stream must not carry usage: {frame}"
        );
    }

    // ── With it: one trailing chunk, no choices, real counts ─────────────
    let (hrx_u, mut crx_u) = run_streaming(
        serde_json::json!({
            "model": "llama3",
            "messages": messages,
            "stream": true,
            "stream_options": {"include_usage": true},
        }),
        93,
    );
    let (usage_status, _) = hrx_u.await.expect("usage stream headers should arrive");
    assert_eq!(usage_status, StatusCode::OK);
    let mut usage_body = String::new();
    while let Some(chunk) = crx_u.recv().await {
        usage_body.push_str(&String::from_utf8_lossy(&chunk));
    }

    let frames = sse_data_frames(&usage_body);
    let carrying_usage: Vec<&Value> = frames
        .iter()
        .filter(|frame| frame.get("usage").is_some_and(|usage| !usage.is_null()))
        .collect();
    assert_eq!(
        carrying_usage.len(),
        1,
        "exactly one chunk carries usage\n{usage_body}"
    );

    let usage_frame = carrying_usage[0];
    assert!(
        usage_frame["choices"]
            .as_array()
            .is_some_and(|choices| choices.is_empty()),
        "the usage chunk must carry no choices: {usage_frame}"
    );
    // It must also be the last chunk before `[DONE]`, or a client that stops
    // reading at `finish_reason` never sees it.
    assert!(
        std::ptr::eq(usage_frame, frames.last().expect("at least one frame")),
        "usage must be the final chunk before [DONE]\n{usage_body}"
    );

    // Counts crossed the whole boundary: decode loop → host channel → WIT
    // resource → guest → SSE. The mock backend reports word counts, so these
    // are synthetic but non-zero and internally consistent.
    let usage = &usage_frame["usage"];
    assert_eq!(
        usage, buffered_usage,
        "the same prompt must cost the same whether streamed or buffered"
    );
    let prompt_tokens = usage["prompt_tokens"].as_u64().expect("prompt_tokens");
    let completion_tokens = usage["completion_tokens"]
        .as_u64()
        .expect("completion_tokens");
    assert!(
        prompt_tokens > 0,
        "prompt_tokens must be measured, not defaulted: {usage}"
    );
    assert!(
        completion_tokens > 0,
        "completion_tokens must be measured, not defaulted: {usage}"
    );
    assert_eq!(
        usage["total_tokens"].as_u64(),
        Some(prompt_tokens + completion_tokens),
        "total_tokens must be the sum: {usage}"
    );

    // Every id in one stream must agree — the usage chunk included, since it is
    // built after the loop and could easily have picked up a fresh one.
    let ids: Vec<&str> = frames
        .iter()
        .filter_map(|frame| frame["id"].as_str())
        .collect();
    assert!(!ids.is_empty());
    assert!(
        ids.windows(2).all(|pair| pair[0] == pair[1]),
        "all chunks of one response must share an id, got {ids:?}"
    );
    assert!(
        ids[0].starts_with("chatcmpl-") && ids[0] != "chatcmpl-tachyon",
        "the completion id must be per-response, got {}",
        ids[0]
    );
}

/// Parse every `data:` frame of an SSE body except the `[DONE]` sentinel.
#[cfg(feature = "ai-inference")]
fn sse_data_frames(body: &str) -> Vec<Value> {
    body.lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter(|payload| *payload != "[DONE]")
        .map(|payload| {
            serde_json::from_str(payload).unwrap_or_else(|error| {
                panic!("SSE frame must be JSON ({error}): {payload}");
            })
        })
        .collect()
}
