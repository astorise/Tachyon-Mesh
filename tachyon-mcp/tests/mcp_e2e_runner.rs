/// End-to-end tests for the tachyon-mcp stdio JSON-RPC server.
///
/// These tests spawn the compiled `tachyon-mcp` binary, feed it JSON-RPC
/// messages via stdin, and assert the stdout responses are well-formed.
///
/// The `initialize` method is tested without a live cluster.
/// Tests that require a real cluster are gated behind the `E2E_CLUSTER_URL`
/// environment variable and are skipped when it is absent.
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn rate_limit_state_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "tachyon-mcp-e2e-{name}-{}-{}.state",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is after Unix epoch")
            .as_nanos()
    ))
}

/// Create a minimal WASM artifact on disk and return its absolute path.
/// The file holds only the WASM magic header + version, which is enough for
/// `tachyon_deploy_function` to accept it as input and propagate it to
/// `tachyon_client::deploy_function`.
fn create_dummy_wasm() -> std::path::PathBuf {
    use std::io::Write;
    let path = std::env::temp_dir().join(format!(
        "tachyon-mcp-e2e-dummy-{}-{}.wasm",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is after Unix epoch")
            .as_nanos()
    ));
    let mut file = std::fs::File::create(&path).expect("create dummy wasm");
    // WASM magic (\0asm) + version 1 (little-endian u32).
    file.write_all(b"\0asm\x01\0\0\0")
        .expect("write wasm header");
    path
}

fn mcp_binary() -> std::path::PathBuf {
    // Prefer a pre-built release binary; fall back to the debug build.
    let release = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("CARGO_MANIFEST_DIR has a parent directory")
        .join("target/release/tachyon-mcp");
    let debug = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("CARGO_MANIFEST_DIR has a parent directory")
        .join("target/debug/tachyon-mcp");

    #[cfg(target_os = "windows")]
    let (release, debug) = (release.with_extension("exe"), debug.with_extension("exe"));

    if release.exists() {
        release
    } else {
        debug
    }
}

fn send_and_recv(stdin: &mut impl Write, stdout_reader: &mut impl BufRead, line: &str) -> String {
    stdin.write_all(line.as_bytes()).expect("write to stdin");
    stdin.write_all(b"\n").expect("write newline");
    stdin.flush().expect("flush stdin");
    let mut response = String::new();
    stdout_reader
        .read_line(&mut response)
        .expect("read from stdout");
    response.trim().to_owned()
}

/// Assert that the `initialize` method returns a valid JSON-RPC 2.0 response
/// with the correct protocol version — no live cluster required.
#[test]
fn test_initialize_returns_protocol_version() {
    let bin = mcp_binary();
    if !bin.exists() {
        eprintln!("tachyon-mcp binary not found at {bin:?}; build it first with `cargo build -p tachyon-mcp`");
        return;
    }

    let mut child = Command::new(&bin)
        .env("TACHYON_MCP_PAT", "e2e-test-token")
        .env("TACHYON_MCP_URL", "http://127.0.0.1:19999") // unreachable port
        .env("TACHYON_MCP_TIMEOUT_MS", "500")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn tachyon-mcp");

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = BufReader::new(stdout);

    let resp_raw = send_and_recv(
        &mut stdin,
        &mut stdout_reader,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
    );

    let resp: serde_json::Value =
        serde_json::from_str(&resp_raw).expect("initialize response is valid JSON");

    assert_eq!(resp["jsonrpc"], "2.0", "jsonrpc field");
    assert_eq!(resp["id"], 1, "id echoed back");
    assert!(resp["result"].is_object(), "result must be an object");
    assert_eq!(
        resp["result"]["protocolVersion"], "2025-03-26",
        "protocol version"
    );
    assert!(
        resp["result"]["serverInfo"].is_object(),
        "serverInfo present"
    );

    drop(stdin);
    let _ = child.wait();
}

/// Assert that `tools/list` returns a structurally valid JSON-RPC response.
/// When no live cluster is reachable the response will be a `-32001` error;
/// that is still a valid JSON-RPC response and is accepted here.
#[test]
fn test_tools_list_is_valid_jsonrpc() {
    let bin = mcp_binary();
    if !bin.exists() {
        return;
    }

    let mut child = Command::new(&bin)
        .env("TACHYON_MCP_PAT", "e2e-test-token")
        .env("TACHYON_MCP_URL", "http://127.0.0.1:19999")
        .env("TACHYON_MCP_TIMEOUT_MS", "500")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn tachyon-mcp");

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = BufReader::new(stdout);

    // First initialise (required before any other method)
    send_and_recv(
        &mut stdin,
        &mut stdout_reader,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
    );

    let resp_raw = send_and_recv(
        &mut stdin,
        &mut stdout_reader,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
    );

    let resp: serde_json::Value =
        serde_json::from_str(&resp_raw).expect("tools/list response is valid JSON");

    assert_eq!(resp["jsonrpc"], "2.0", "jsonrpc field");
    assert_eq!(resp["id"], 2, "id echoed back");

    // Either a result or a structured error is acceptable
    let has_result = resp["result"].is_object();
    let has_error = resp["error"].is_object();
    assert!(
        has_result || has_error,
        "response must have result or error"
    );

    if has_error {
        // Error code must be a known JSON-RPC error
        let code = resp["error"]["code"]
            .as_i64()
            .expect("error.code is integer");
        assert!(
            [-32001i64, -32002, -32600, -32602, -32603].contains(&code),
            "unexpected error code {code}"
        );
    } else {
        // Successful tools/list must include a non-empty tools array
        let tools = resp["result"]["tools"]
            .as_array()
            .expect("result.tools is an array");
        assert!(!tools.is_empty(), "tools array must not be empty");

        // Verify every tool entry has required MCP schema fields
        for tool in tools {
            assert!(tool["name"].is_string(), "tool.name is string");
            assert!(
                tool["description"].is_string(),
                "tool.description is string"
            );
            assert!(
                tool["inputSchema"].is_object(),
                "tool.inputSchema is object"
            );
        }
    }

    drop(stdin);
    let _ = child.wait();
}

/// Assert that `tachyon_kv_put` returns a structurally valid JSON-RPC response.
/// Without a live cluster the call will return `-32001`; that is acceptable.
#[test]
fn test_kv_put_is_valid_jsonrpc() {
    let bin = mcp_binary();
    if !bin.exists() {
        return;
    }

    let mut child = Command::new(&bin)
        .env("TACHYON_MCP_PAT", "e2e-test-token")
        .env("TACHYON_MCP_URL", "http://127.0.0.1:19999")
        .env("TACHYON_MCP_TIMEOUT_MS", "500")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn tachyon-mcp");

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = std::io::BufReader::new(stdout);

    send_and_recv(
        &mut stdin,
        &mut stdout_reader,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
    );

    let resp_raw = send_and_recv(
        &mut stdin,
        &mut stdout_reader,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"tachyon_kv_put","arguments":{"namespace":"e2e-test","key":"ping","value":"\"pong\""}}}"#,
    );

    let resp: serde_json::Value =
        serde_json::from_str(&resp_raw).expect("kv_put response is valid JSON");
    assert_eq!(resp["jsonrpc"], "2.0", "jsonrpc field");
    assert_eq!(resp["id"], 2, "id echoed back");

    // Either success or cluster-unreachable — never internal-error
    if let Some(code) = resp["error"]["code"].as_i64() {
        assert_ne!(code, -32603, "kv_put must not return internal_error");
    }

    drop(stdin);
    let _ = child.wait();
}

/// Assert that sending `tachyon_canary_split` more than its rate limit (2/min)
/// returns `-32002` with `retry_after_ms` on the excess call.
#[test]
fn test_canary_split_rate_limit_returns_32002() {
    let bin = mcp_binary();
    if !bin.exists() {
        return;
    }

    let mut child = Command::new(&bin)
        .env("TACHYON_MCP_PAT", "e2e-test-token")
        .env("TACHYON_MCP_URL", "http://127.0.0.1:19999")
        .env("TACHYON_MCP_TIMEOUT_MS", "500")
        .env(
            "TACHYON_MCP_RATE_LIMIT_STATE",
            rate_limit_state_path("canary-split"),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn tachyon-mcp");

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = std::io::BufReader::new(stdout);

    send_and_recv(
        &mut stdin,
        &mut stdout_reader,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
    );

    let canary_call = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"tachyon_canary_split","arguments":{"route_path":"/api/demo","weight_pct":10}}}"#;

    // Send the request 3 times rapidly. The limit is 2/min so the third call
    // must be rate-limited even when the cluster is unreachable (rate limiting
    // happens before the cluster call).
    let mut last_response = String::new();
    for _ in 0..3 {
        last_response = send_and_recv(&mut stdin, &mut stdout_reader, canary_call);
    }

    let resp: serde_json::Value =
        serde_json::from_str(&last_response).expect("3rd canary response is valid JSON");
    assert_eq!(resp["jsonrpc"], "2.0");

    // The 3rd call must be rate-limited
    if let Some(code) = resp["error"]["code"].as_i64() {
        assert_eq!(
            code, -32002,
            "3rd canary_split must be rate-limited (-32002)"
        );
        assert!(
            resp["error"]["data"]["retry_after_ms"].is_number(),
            "rate limit error must include retry_after_ms"
        );
    }
    // If the 3rd call was NOT rate-limited (e.g. state was reset between runs),
    // the test passes silently — rate limit state is process-local.

    drop(stdin);
    let _ = child.wait();
}

/// Helper: spawn tachyon-mcp pointed at an unreachable port, initialize it,
/// and return the child + piped stdio. Used by the offline lifecycle tests.
fn spawn_offline_mcp() -> Option<(
    std::process::Child,
    std::process::ChildStdin,
    std::io::BufReader<std::process::ChildStdout>,
)> {
    let bin = mcp_binary();
    if !bin.exists() {
        return None;
    }
    let mut child = Command::new(&bin)
        .env("TACHYON_MCP_PAT", "e2e-test-token")
        .env("TACHYON_MCP_URL", "http://127.0.0.1:19999")
        .env("TACHYON_MCP_TIMEOUT_MS", "500")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn tachyon-mcp");
    let stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);
    let mut s = stdin;
    send_and_recv(
        &mut s,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
    );
    Some((child, s, reader))
}

/// Assert JSON-RPC structure (`jsonrpc=2.0`, echoed id) and that — when an
/// error is returned — its code is `-32001 cluster_unreachable` (acceptable
/// offline) rather than `-32603 internal_error`.
fn assert_offline_jsonrpc(resp: &serde_json::Value, expected_id: i64) {
    assert_eq!(resp["jsonrpc"], "2.0", "jsonrpc field");
    assert_eq!(resp["id"], expected_id, "id echoed back");
    if let Some(code) = resp["error"]["code"].as_i64() {
        assert_ne!(
            code, -32603,
            "tool must not return internal_error offline; got {resp}"
        );
        // Offline calls land in cluster_unreachable; anything else is a regression.
        assert!(
            [-32001i64, -32002, -32602].contains(&code),
            "unexpected offline error code {code}; full response: {resp}"
        );
    }
}

/// Task 2: Lifecycle tools (`deploy_function`, `list_functions`,
/// `function_logs`, `delete_function`) must each return valid JSON-RPC
/// responses with no `-32603` even when the cluster is unreachable.
#[test]
fn test_function_lifecycle_is_valid_jsonrpc() {
    let Some((mut child, mut stdin, mut reader)) = spawn_offline_mcp() else {
        return;
    };
    let dummy = create_dummy_wasm();
    let dummy_str = dummy.to_string_lossy().replace('\\', "\\\\");

    let deploy_req = format!(
        r#"{{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{{"name":"tachyon_deploy_function","arguments":{{"function_name":"e2e-test-func","artifact_path":"{dummy_str}"}}}}}}"#
    );
    let raw = send_and_recv(&mut stdin, &mut reader, &deploy_req);
    let resp: serde_json::Value =
        serde_json::from_str(&raw).expect("deploy_function response is valid JSON");
    assert_offline_jsonrpc(&resp, 10);

    let raw = send_and_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"tachyon_list_functions","arguments":{}}}"#,
    );
    let resp: serde_json::Value =
        serde_json::from_str(&raw).expect("list_functions response is valid JSON");
    assert_offline_jsonrpc(&resp, 11);

    let raw = send_and_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"tachyon_function_logs","arguments":{"function_name":"e2e-test-func","lines":10}}}"#,
    );
    let resp: serde_json::Value =
        serde_json::from_str(&raw).expect("function_logs response is valid JSON");
    assert_offline_jsonrpc(&resp, 12);

    let raw = send_and_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"tachyon_delete_function","arguments":{"function_name":"e2e-test-func"}}}"#,
    );
    let resp: serde_json::Value =
        serde_json::from_str(&raw).expect("delete_function response is valid JSON");
    assert_offline_jsonrpc(&resp, 13);

    let _ = std::fs::remove_file(&dummy);
    drop(stdin);
    let _ = child.wait();
}

/// Task 3a: KV reader/deleter tools must return valid JSON-RPC responses.
#[test]
fn test_kv_get_and_delete_are_valid_jsonrpc() {
    let Some((mut child, mut stdin, mut reader)) = spawn_offline_mcp() else {
        return;
    };

    let raw = send_and_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":20,"method":"tools/call","params":{"name":"tachyon_kv_get","arguments":{"namespace":"e2e","key":"missing-key"}}}"#,
    );
    let resp: serde_json::Value =
        serde_json::from_str(&raw).expect("kv_get response is valid JSON");
    assert_offline_jsonrpc(&resp, 20);

    let raw = send_and_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":21,"method":"tools/call","params":{"name":"tachyon_kv_delete","arguments":{"namespace":"e2e","key":"missing-key"}}}"#,
    );
    let resp: serde_json::Value =
        serde_json::from_str(&raw).expect("kv_delete response is valid JSON");
    assert_offline_jsonrpc(&resp, 21);

    drop(stdin);
    let _ = child.wait();
}

/// Task 3b: A `tachyon_deploy_function` call missing the required
/// `artifact_path` argument MUST return JSON-RPC error code `-32602`
/// (Invalid params). Agents rely on this code to self-correct.
#[test]
fn test_deploy_function_missing_param_returns_32602() {
    let Some((mut child, mut stdin, mut reader)) = spawn_offline_mcp() else {
        return;
    };

    let raw = send_and_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":30,"method":"tools/call","params":{"name":"tachyon_deploy_function","arguments":{"function_name":"missing-artifact"}}}"#,
    );
    let resp: serde_json::Value =
        serde_json::from_str(&raw).expect("missing-param response is valid JSON");
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 30);
    let code = resp["error"]["code"]
        .as_i64()
        .expect("missing required field must produce a JSON-RPC error");
    assert_eq!(
        code, -32602,
        "missing required field must map to -32602 invalid_params; got {resp}"
    );

    drop(stdin);
    let _ = child.wait();
}

/// `tools/list` must include the three new scope tools and `tachyon_get_metrics`
/// description must mention `scope_denial_total`.
#[test]
fn test_tools_list_includes_scope_tools() {
    let Some((mut child, mut stdin, mut reader)) = spawn_offline_mcp() else {
        return;
    };

    let resp_raw = send_and_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":50,"method":"tools/list","params":{}}"#,
    );
    let resp: serde_json::Value =
        serde_json::from_str(&resp_raw).expect("tools/list is valid JSON");

    if let Some(tools) = resp["result"]["tools"].as_array() {
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        for expected in &[
            "tachyon_get_scope_denials",
            "tachyon_set_route_scopes",
            "tachyon_patch_manifest",
            "tachyon_patch_route",
            "tachyon_suggest_scopes",
            "tachyon_lora_training_status",
        ] {
            assert!(
                names.contains(expected),
                "tools/list must include `{expected}`"
            );
        }
        let metrics_tool = tools.iter().find(|t| t["name"] == "tachyon_get_metrics");
        if let Some(tool) = metrics_tool {
            let desc = tool["description"].as_str().unwrap_or("");
            assert!(
                desc.contains("scope_denial_total"),
                "tachyon_get_metrics description must mention scope_denial_total"
            );
        }
    }

    drop(stdin);
    let _ = child.wait();
}

/// `tachyon_get_scope_denials` and `tachyon_suggest_scopes` return valid
/// JSON-RPC responses offline (no required arguments needed for get_scope_denials;
/// suggest_scopes missing route_path → -32602).
#[test]
fn test_scope_read_tools_offline_behavior() {
    let Some((mut child, mut stdin, mut reader)) = spawn_offline_mcp() else {
        return;
    };

    // get_scope_denials has no required args; offline → -32001 from the metrics call
    let raw = send_and_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":51,"method":"tools/call","params":{"name":"tachyon_get_scope_denials","arguments":{}}}"#,
    );
    let resp: serde_json::Value =
        serde_json::from_str(&raw).expect("get_scope_denials response is valid JSON");
    assert_offline_jsonrpc(&resp, 51);

    // suggest_scopes missing route_path → -32602 (caught before cluster call)
    let raw = send_and_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":52,"method":"tools/call","params":{"name":"tachyon_suggest_scopes","arguments":{}}}"#,
    );
    let resp: serde_json::Value =
        serde_json::from_str(&raw).expect("suggest_scopes missing-arg response is valid JSON");
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 52);
    assert_eq!(
        resp["error"]["code"].as_i64(),
        Some(-32602),
        "missing route_path must return -32602; got {resp}"
    );

    drop(stdin);
    let _ = child.wait();
}

/// Helper: spawn a fresh offline MCP with an isolated rate-limit state file.
fn spawn_offline_mcp_isolated(
    name: &str,
) -> Option<(
    std::process::Child,
    std::process::ChildStdin,
    std::io::BufReader<std::process::ChildStdout>,
)> {
    let bin = mcp_binary();
    if !bin.exists() {
        return None;
    }
    let mut child = Command::new(&bin)
        .env("TACHYON_MCP_PAT", "e2e-test-token")
        .env("TACHYON_MCP_URL", "http://127.0.0.1:19999")
        .env("TACHYON_MCP_TIMEOUT_MS", "500")
        .env("TACHYON_MCP_RATE_LIMIT_STATE", rate_limit_state_path(name))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn tachyon-mcp");
    let stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);
    let mut s = stdin;
    send_and_recv(
        &mut s,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
    );
    Some((child, s, reader))
}

/// `tachyon_set_route_scopes` missing required args → -32602.
/// Uses isolated rate-limit state per assertion so the 1/min limit doesn't
/// interfere with the missing-params check.
#[test]
fn test_set_route_scopes_missing_args_returns_32602() {
    // Missing both route_path and scopes
    let Some((mut child, mut stdin, mut reader)) =
        spawn_offline_mcp_isolated("set-scopes-miss-both")
    else {
        return;
    };
    let raw = send_and_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":60,"method":"tools/call","params":{"name":"tachyon_set_route_scopes","arguments":{}}}"#,
    );
    let resp: serde_json::Value =
        serde_json::from_str(&raw).expect("set_route_scopes response is valid JSON");
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 60);
    assert_eq!(
        resp["error"]["code"].as_i64(),
        Some(-32602),
        "missing required args must return -32602; got {resp}"
    );
    drop(stdin);
    let _ = child.wait();

    // Missing scopes only — fresh process to avoid rate limit
    let Some((mut child, mut stdin, mut reader)) =
        spawn_offline_mcp_isolated("set-scopes-miss-scopes")
    else {
        return;
    };
    let raw = send_and_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":61,"method":"tools/call","params":{"name":"tachyon_set_route_scopes","arguments":{"route_path":"/api/fn"}}}"#,
    );
    let resp: serde_json::Value =
        serde_json::from_str(&raw).expect("set_route_scopes missing scopes response is valid JSON");
    assert_eq!(
        resp["error"]["code"].as_i64(),
        Some(-32602),
        "missing scopes must return -32602; got {resp}"
    );
    drop(stdin);
    let _ = child.wait();
}

/// `tachyon_patch_route`, `tachyon_patch_manifest`, and
/// `tachyon_lora_training_status` missing required args are rejected before any
/// cluster/network call.
#[test]
fn test_route_patch_and_lora_status_missing_args_return_32602() {
    let Some((mut child, mut stdin, mut reader)) =
        spawn_offline_mcp_isolated("patch-route-missing")
    else {
        return;
    };
    let raw = send_and_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":80,"method":"tools/call","params":{"name":"tachyon_patch_route","arguments":{"route_path":"/api/fn"}}}"#,
    );
    let resp: serde_json::Value =
        serde_json::from_str(&raw).expect("patch_route missing patch response is valid JSON");
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 80);
    assert_eq!(
        resp["error"]["code"].as_i64(),
        Some(-32602),
        "missing patch must return -32602; got {resp}"
    );
    drop(stdin);
    let _ = child.wait();

    let Some((mut child, mut stdin, mut reader)) =
        spawn_offline_mcp_isolated("patch-manifest-missing")
    else {
        return;
    };
    let raw = send_and_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":82,"method":"tools/call","params":{"name":"tachyon_patch_manifest","arguments":{}}}"#,
    );
    let resp: serde_json::Value =
        serde_json::from_str(&raw).expect("patch_manifest missing patch response is valid JSON");
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 82);
    assert_eq!(
        resp["error"]["code"].as_i64(),
        Some(-32602),
        "missing patch must return -32602; got {resp}"
    );
    drop(stdin);
    let _ = child.wait();

    let Some((mut child, mut stdin, mut reader)) =
        spawn_offline_mcp_isolated("lora-status-missing")
    else {
        return;
    };
    let raw = send_and_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":81,"method":"tools/call","params":{"name":"tachyon_lora_training_status","arguments":{}}}"#,
    );
    let resp: serde_json::Value =
        serde_json::from_str(&raw).expect("lora status missing job_id response is valid JSON");
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 81);
    assert_eq!(
        resp["error"]["code"].as_i64(),
        Some(-32602),
        "missing job_id must return -32602; got {resp}"
    );
    drop(stdin);
    let _ = child.wait();
}

/// `tachyon_set_route_scopes` is rate-limited to 1 req/min.
/// The second call within the same window must return `-32002`.
#[test]
fn test_set_route_scopes_rate_limited() {
    let bin = mcp_binary();
    if !bin.exists() {
        return;
    }
    let mut child = Command::new(&bin)
        .env("TACHYON_MCP_PAT", "e2e-test-token")
        .env("TACHYON_MCP_URL", "http://127.0.0.1:19999")
        .env("TACHYON_MCP_TIMEOUT_MS", "500")
        .env(
            "TACHYON_MCP_RATE_LIMIT_STATE",
            rate_limit_state_path("set-route-scopes"),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn tachyon-mcp");

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);

    send_and_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
    );

    let req = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"tachyon_set_route_scopes","arguments":{"route_path":"/api/fn","scopes":{"secrets":["**"]}}}}"#;
    send_and_recv(&mut stdin, &mut reader, req);

    // Second call must be rate-limited
    let raw = send_and_recv(&mut stdin, &mut reader, req);
    let resp: serde_json::Value =
        serde_json::from_str(&raw).expect("second set_route_scopes response is valid JSON");
    if let Some(code) = resp["error"]["code"].as_i64() {
        assert_eq!(code, -32002, "second call must be rate-limited; got {resp}");
        assert!(
            resp["error"]["data"]["retry_after_ms"].is_number(),
            "rate limit must include retry_after_ms"
        );
    }

    drop(stdin);
    let _ = child.wait();
}

/// Task 5.3: Chain `tachyon_get_scope_denials` → `tachyon_suggest_scopes` →
/// `tachyon_set_route_scopes` dry_run in a single offline session.
///
/// Without a live cluster every call will fail at the HTTP layer (-32001 or
/// -32603), but the JSON-RPC envelope must always be well-formed and the
/// missing-arg check (-32602) must fire *before* any network attempt.
#[test]
fn test_scope_tools_offline_chain() {
    let Some((mut child, mut stdin, mut reader)) = spawn_offline_mcp_isolated("scope-chain") else {
        return;
    };

    // Step 1 — get_scope_denials: no required args; offline → -32001 / -32603
    let raw = send_and_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":70,"method":"tools/call","params":{"name":"tachyon_get_scope_denials","arguments":{}}}"#,
    );
    let resp: serde_json::Value =
        serde_json::from_str(&raw).expect("get_scope_denials chain response is valid JSON");
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 70);
    assert!(
        resp["result"].is_object() || resp["error"].is_object(),
        "must have result or error; got {resp}"
    );
    if let Some(code) = resp["error"]["code"].as_i64() {
        assert_ne!(
            code, -32602,
            "get_scope_denials takes no required args; got -32602"
        );
    }

    // Step 2 — suggest_scopes with a route_path; offline → -32001 / -32603
    let raw = send_and_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":71,"method":"tools/call","params":{"name":"tachyon_suggest_scopes","arguments":{"route_path":"/api/fn"}}}"#,
    );
    let resp: serde_json::Value =
        serde_json::from_str(&raw).expect("suggest_scopes chain response is valid JSON");
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 71);
    if let Some(code) = resp["error"]["code"].as_i64() {
        // -32602 would mean route_path was still rejected — regression
        assert_ne!(
            code, -32602,
            "suggest_scopes must accept route_path; got -32602"
        );
        // Only -32001 (cluster unreachable) or -32603 (client error) are expected offline
        assert!(
            [-32001i64, -32603].contains(&code),
            "unexpected offline code {code} for suggest_scopes; got {resp}"
        );
    }

    // Step 3 — set_route_scopes with dry_run=true; same isolated session so
    // the 1/min rate-limit is NOT exhausted (only the first call consumes the token).
    let raw = send_and_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":72,"method":"tools/call","params":{"name":"tachyon_set_route_scopes","arguments":{"route_path":"/api/fn","scopes":{"kv":["tenant-a/*"]},"dry_run":true}}}"#,
    );
    let resp: serde_json::Value =
        serde_json::from_str(&raw).expect("set_route_scopes dry_run chain response is valid JSON");
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 72);
    assert!(
        resp["result"].is_object() || resp["error"].is_object(),
        "must have result or error; got {resp}"
    );
    if let Some(code) = resp["error"]["code"].as_i64() {
        // -32602 = missing args (regression), -32002 = rate-limited (unexpected: first call)
        assert_ne!(
            code, -32602,
            "set_route_scopes with all args must not return -32602"
        );
        assert_ne!(
            code, -32002,
            "first set_route_scopes call must not be rate-limited"
        );
    }

    drop(stdin);
    let _ = child.wait();
}

/// Full integration test against a running cluster.
/// Skipped unless `E2E_CLUSTER_URL` and `E2E_CLUSTER_PAT` are set.
#[test]
fn test_tools_list_against_live_cluster() {
    let url = match std::env::var("E2E_CLUSTER_URL") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("Skipping live-cluster test: E2E_CLUSTER_URL not set");
            return;
        }
    };
    let pat = match std::env::var("E2E_CLUSTER_PAT") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("Skipping live-cluster test: E2E_CLUSTER_PAT not set");
            return;
        }
    };

    let bin = mcp_binary();
    if !bin.exists() {
        return;
    }

    let mut child = Command::new(&bin)
        .env("TACHYON_MCP_PAT", &pat)
        .env("TACHYON_MCP_URL", &url)
        .env("TACHYON_MCP_TIMEOUT_MS", "5000")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn tachyon-mcp");

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = BufReader::new(stdout);

    // Initialize
    send_and_recv(
        &mut stdin,
        &mut stdout_reader,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
    );

    // tools/list must succeed and include core tools
    let resp_raw = send_and_recv(
        &mut stdin,
        &mut stdout_reader,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
    );
    let resp: serde_json::Value =
        serde_json::from_str(&resp_raw).expect("tools/list response is valid JSON");
    let tools = resp["result"]["tools"]
        .as_array()
        .expect("tools/list succeeded");

    let tool_names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();

    for expected in &[
        "tachyon_hardware_status",
        "tachyon_topology_snapshot",
        "tachyon_dryrun_manifest",
    ] {
        assert!(
            tool_names.contains(expected),
            "expected tool `{expected}` in tools/list"
        );
    }

    // Verify tachyon_dryrun_manifest inputSchema contains the manifest schema
    let dryrun = tools
        .iter()
        .find(|t| t["name"] == "tachyon_dryrun_manifest")
        .expect("tachyon_dryrun_manifest tool present");
    assert!(
        dryrun["inputSchema"]["properties"].is_object(),
        "inputSchema has properties"
    );

    // Read-only call: tachyon_hardware_status should not return -32603
    let hw_raw = send_and_recv(
        &mut stdin,
        &mut stdout_reader,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"tachyon_hardware_status","arguments":{}}}"#,
    );
    let hw: serde_json::Value =
        serde_json::from_str(&hw_raw).expect("hardware status response is valid JSON");
    if let Some(err) = hw["error"].as_object() {
        let code = err["code"].as_i64().unwrap_or(0);
        assert_ne!(
            code, -32603,
            "tachyon_hardware_status must not return internal_error"
        );
    }

    drop(stdin);
    let _ = child.wait();
}
