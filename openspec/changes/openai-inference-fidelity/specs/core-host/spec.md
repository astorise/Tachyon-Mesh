## ADDED Requirements

### Requirement: The host provides an incremental body-flush streaming transport

When a request carries `Accept: text/event-stream`, the host SHALL execute the
FaaS guest on a dedicated thread with a `tachyon:mesh/response-body` streaming
sink pre-installed in the execution context. The guest acquires the sink via
`get-streaming-response`, commits status and headers with `begin`, and flushes
body bytes with `write`; each write is forwarded to the client immediately via an
axum `Body::from_stream` backed by a bounded channel, so the client receives
headers and first bytes before generation completes. When the guest never calls
`begin`, the execution SHALL fall back to a buffered response whose headers and
body are forwarded through the same channel pair after `handle-request` returns,
so a non-streaming guest under a streaming request still responds correctly. The
transport SHALL acquire the route's volume leases and concurrency permit exactly
as the buffered path does, and SHALL wire the same scope-gated interfaces into
the streaming linker (including `kv-partition` and `graph` under the `kv` scope).

#### Scenario: Streaming response flushes chunks in real time

- **GIVEN** a guest that calls `get-streaming-response` and `begin`
- **WHEN** the guest writes body chunks via `write`
- **THEN** each chunk is forwarded to the connected HTTP client immediately
- **AND** the client receives the committed headers before any body bytes

#### Scenario: Buffered fallback under a streaming request

- **GIVEN** a request carrying `Accept: text/event-stream`
- **WHEN** the guest returns a buffered `handler::response` without calling `begin`
- **THEN** the host forwards the buffered status, headers, and body through the
  same channel pair
- **AND** the awaiting transport never blocks waiting for headers

#### Scenario: Streaming linker matches the buffered authorization model

- **WHEN** the streaming execution path builds its component linker
- **THEN** it wires the same interfaces as the buffered path, each gated on the
  route's deployment scope (e.g. `kv-partition` and `graph` only when the route
  grants `kv`)
