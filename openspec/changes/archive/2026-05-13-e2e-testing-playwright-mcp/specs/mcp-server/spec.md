# Technical Specification: MCP Tool Testing

## 1. Test Script (`tachyon-mcp/tests/mcp_e2e_runner.rs`)
Since standard MCP operates over `stdio`, we need a script that spawns the `tachyon-mcp` binary, feeds it JSON-RPC strings via `stdin`, and asserts the `stdout`.

```rust
use std::process::{Command, Stdio};
use std::io::Write;

#[test]
fn test_mcp_all_tools_sanity() {
    let mut child = Command::new("cargo")
        .args(["run", "--bin", "tachyon-mcp"])
        .env("TACHYON_MCP_PAT", "test-token")
        .env("TACHYON_MCP_URL", "[http://127.0.0.1:8080](http://127.0.0.1:8080)")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to start MCP server");

    let stdin = child.stdin.as_mut().expect("Failed to open stdin");
    
    // 1. Test Tools List
    let init_req = r#"{"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}"#;
    stdin.write_all(init_req.as_bytes()).unwrap();
    stdin.write_all(b"\n").unwrap();
    
    // Read stdout line, parse JSON, assert tools are present...
    
    // 2. Test a read-only tool (e.g., tachyon_hardware_status)
    let call_req = r#"{"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "tachyon_hardware_status", "arguments": {}}}"#;
    stdin.write_all(call_req.as_bytes()).unwrap();
    stdin.write_all(b"\n").unwrap();
    
    // Read stdout line, verify no -32603 or -32602 error code is returned.
    
    child.kill().unwrap();
}
```

## 2. GitHub Actions Integration (`.github/workflows/ci.yml`)
Add a new job `e2e-tests` that ensures both `core-host` and `tachyon-mcp` interact properly in a clean environment.