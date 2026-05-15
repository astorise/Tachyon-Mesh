# Technical Specification: MCP E2E Mutator Coverage

## 1. Helper: Dummy WASM Generation
At the start of the test execution, ensure a temporary file is created to simulate a WASM artifact for the deploy test.

```rust
// tachyon-mcp/tests/mcp_e2e_runner.rs
use std::fs::File;
use std::io::Write;

fn create_dummy_wasm() -> String {
    let path = "/tmp/dummy_test_func.wasm";
    let mut file = File::create(path).unwrap();
    file.write_all(b"\\0asm\x01\0\0\0").unwrap(); // Minimal valid WASM magic header
    path.to_string()
}
```

## 2. FaaS Lifecycle Tests
Extend the test sequence to execute the deployment lifecycle.

```rust
// 1. Deploy Function
let dummy_path = create_dummy_wasm();
let deploy_req = format!(
    r#"{{"jsonrpc": "2.0", "id": 10, "method": "tools/call", "params": {{"name": "tachyon_deploy_function", "arguments": {{"function_name": "e2e-test-func", "artifact_path": "{}"}}}}}}"#,
    dummy_path
);
stdin.write_all(deploy_req.as_bytes()).unwrap();
// Read & assert response has no error

// 2. List Functions
let list_req = r#"{"jsonrpc": "2.0", "id": 11, "method": "tools/call", "params": {"name": "tachyon_list_functions", "arguments": {}}}"#;
stdin.write_all(list_req.as_bytes()).unwrap();
// Read & assert response contains "e2e-test-func"

// 3. Delete Function
let delete_req = r#"{"jsonrpc": "2.0", "id": 12, "method": "tools/call", "params": {"name": "tachyon_delete_function", "arguments": {"function_name": "e2e-test-func"}}}"#;
stdin.write_all(delete_req.as_bytes()).unwrap();
// Read & assert successful deletion
```

## 3. KV Store Tests
Extend the test sequence for the remaining KV tools.

```rust
// 1. KV Get (Assuming a previous test did a KV Put for key 'e2e-key')
let kv_get_req = r#"{"jsonrpc": "2.0", "id": 20, "method": "tools/call", "params": {"name": "tachyon_kv_get", "arguments": {"namespace": "default", "key": "e2e-key"}}}"#;
stdin.write_all(kv_get_req.as_bytes()).unwrap();
// Read & assert response

// 2. KV Delete
let kv_del_req = r#"{"jsonrpc": "2.0", "id": 21, "method": "tools/call", "params": {"name": "tachyon_kv_delete", "arguments": {"namespace": "default", "key": "e2e-key"}}}"#;
stdin.write_all(kv_del_req.as_bytes()).unwrap();
// Read & assert response
```

## 4. Error Code Assertion (`-32602`)
Test the schema validation boundaries.

```rust
// Missing required argument 'artifact_path'
let bad_req = r#"{"jsonrpc": "2.0", "id": 30, "method": "tools/call", "params": {"name": "tachyon_deploy_function", "arguments": {"function_name": "bad-func"}}}"#;
stdin.write_all(bad_req.as_bytes()).unwrap();
// Read response
// assert_eq!(response["error"]["code"], -32602);
```