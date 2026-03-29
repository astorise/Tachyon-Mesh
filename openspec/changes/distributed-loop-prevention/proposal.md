# Proposal: Change 017 - Distributed Loop Prevention & Hop Limits

## Context
While Wasmtime "Fuel" (Change 003) prevents a single FaaS function from executing an infinite CPU loop, it does not protect the system from distributed routing loops (e.g., FaaS A calls Legacy B, which calls FaaS C, which calls FaaS A). These distributed loops create request storms that quickly exhaust the host's asynchronous worker pool and memory.

## Objective
Implement a "Hop Limit" (or Call Depth) mechanism in the `core-host` API Gateway. Every request entering the mesh will be assigned a maximum number of allowed network/IPC hops. Every time a FaaS or Legacy container makes an outbound call through the Host, the Host decrements this counter. If the counter reaches zero, the Host immediately aborts the call chain and returns an HTTP 508 (Loop Detected).

## Scope
- Implement an Axum middleware in `core-host` that inspects incoming requests for a `X-Tachyon-Hop-Limit` header.
- If the header is missing, initialize it to a safe default (e.g., 10).
- If the header is present, parse it as an integer. If it is `0`, reject the request with HTTP `508 Loop Detected`.
- When the Host executes an outbound call on behalf of a guest (WASM -> Legacy, or Legacy -> WASM), it MUST inject the decremented `X-Tachyon-Hop-Limit` into the outgoing request headers.
- Write an integration test where FaaS A intentionally calls FaaS A to prove the circuit breaks.

## Success Metrics
- A single request can successfully traverse 3 different services without issue.
- A service chain that loops infinitely is stopped exactly at the 10th hop.
- The Host never crashes or runs out of memory during a routing loop attack.
- The standard HTTP `508 Loop Detected` status code is correctly returned to the caller.