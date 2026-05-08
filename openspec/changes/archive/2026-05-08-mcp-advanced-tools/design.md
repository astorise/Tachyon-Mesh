# Design: Advanced MCP Tools

## Context

`tachyon-mcp` already exposes workspace inspection, sealing, apply, and local hardware tools. Agentic operations also need observability and resilience controls that can be called through the same MCP surface.

## Decisions

- Keep `tachyon_dryrun_manifest` local-only. It parses raw config payloads or sealed manifest `configPayload` fields and returns a validation report without touching overlay files, `integrity.lock`, or remote admin APIs.
- Add small typed tachyon-client bindings for `/admin/metrics`, `/admin/logs`, `/admin/shadow/diffs`, and `/admin/chaos/scenarios`.
- Return recent logs as normal MCP tool content and include `notifications/message` JSON-RPC payloads in `structuredContent` so clients can render notification streams without blocking the stdio request loop.
- Surface admin HTTP failures directly, including status code and response body, rather than hiding unavailable host capabilities.

## Risks

- Host endpoint schemas may evolve. The MCP boundary uses typed client structs so schema drift fails clearly at the binding layer.
- Full unbounded log streaming over stdio can block the MCP request loop. The first implementation keeps the call bounded and notification-compatible; follow mode is captured in the response contract for clients that continue polling.
