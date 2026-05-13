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

fn mcp_binary() -> std::path::PathBuf {
    // Prefer a pre-built release binary; fall back to the debug build.
    let release = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("target/release/tachyon-mcp");
    let debug = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
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
    let resp: serde_json::Value = serde_json::from_str(&resp_raw).unwrap();
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
    let hw: serde_json::Value = serde_json::from_str(&hw_raw).unwrap();
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
