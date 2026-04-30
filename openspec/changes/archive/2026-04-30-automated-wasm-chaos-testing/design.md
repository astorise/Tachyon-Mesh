# Design: The Chaos Test Suite (`core-host/tests/chaos_test.rs`)

```rust
#[tokio::test]
async fn test_infinite_loop_resilience() {
    let (node, client) = setup_test_node_with_guest("guest-malicious").await;
    
    // Trigger the infinite loop endpoint
    let res = client.get("/attack/infinite-loop").send().await;
    
    // Assert the host caught the Wasmtime Fuel exhaustion
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(res.body().contains("Wasm trap: out of fuel"));
    
    // Assert the host is still alive and fast
    let health = client.get("/health").send().await;
    assert_eq!(health.status(), StatusCode::OK);
}
```