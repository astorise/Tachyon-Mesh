# Proposal: Automated Chaos Testing Harness

## Context
Tachyon Mesh relies on WebAssembly's isolation to safely execute untrusted multi-tenant code. We have an example FaaS (`guest-malicious`) designed to crash the host (infinite loops, memory exhaustion, panic). However, it is a manual test. If a regression occurs in our `wasmtime` fuel consumption or memory governor, we might not realize it until a production outage.

## Proposed Solution
Create a CI-driven Chaos Harness. The test suite will intentionally deploy `guest-malicious`, fire HTTP requests triggering specific attacks, and assert that:
1. The host **does not crash**.
2. The specific request yields a `500 Internal Server Error` or `408 Request Timeout`.
3. Subsequent valid requests to other endpoints continue to succeed with < 1ms latency (proving no thread locking).