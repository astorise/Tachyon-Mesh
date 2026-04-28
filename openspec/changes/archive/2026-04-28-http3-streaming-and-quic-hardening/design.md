# Design: Streaming Body Integration

## 1. Request Stream Bridging (`core-host/src/server_h3.rs`)
The function `handle_http3_request` must be refactored:
- **Current logic:** Loops over `stream.recv_data()` and appends to a `BytesMut`.
- **New logic:** Wrap the `h3::server::RequestStream` into a custom wrapper that implements `http_body::Body`. 
- Pass this streaming body directly into `axum::Request`.

## 2. QUIC Hardening (`build_quinn_server_config`)
Update the server configuration in `core-host/src/server_h3.rs`:
- `max_idle_timeout`: Set to `30s` to reap dead connections.
- `max_concurrent_bidi_streams`: Set a strict limit (e.g., `100`) per connection.
- `keep_alive_interval`: Configure to maintain healthy long-lived streams.

## 3. Memory Safety
- Ensure that the internal buffer used for chunking has a fixed size (e.g., 64KB) to maintain a constant memory footprint regardless of the total request size.