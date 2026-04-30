# Design: Local Interop Harness

## 1. gRPC Conformance Harness
We will use the official `grpc/interop` Docker image.

**Setup:**
- The CI spawns `core-host` in the background.
- It loads `examples/guest-grpc` (which implements the standard gRPC echo/streaming testing service).

**Execution:**
```yaml
- name: Run gRPC Interop Tests
  run: |
    docker run --network host grpc/go:latest \
      go run interop/client/client.go \
      --server_host=127.0.0.1 --server_port=443 \
      --use_tls=true --test_case=all
```
If any test case (e.g., `empty_unary`, `large_unary`, `client_streaming`, `cancel_after_begin`) fails, the CI fails.

## 2. HTTP/3 (QUIC) Conformance Harness
We will integrate with the `quic-interop-runner` framework.
Since Tachyon Mesh uses `quinn` under the hood, we inherit its stability, but our *wrapper* (ALPN negotiation, 0-RTT handling) must be validated.

**Execution:**
We will create a lightweight `docker-compose.yml` in `tests/conformance/` that runs the standard `quic-interop-runner` Python script targeting a Tachyon Mesh container. It will specifically test:
- Handshake & TLS 1.3
- 0-RTT resumption
- Connection Migration (crucial for Edge/Mobile clients)
- Stream multiplexing limits