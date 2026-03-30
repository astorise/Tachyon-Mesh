# Design: Distributed Loop Prevention

## Overview
The host already supports guest-triggered outbound traffic through `MESH_FETCH:` bridge commands. The safest place to stop distributed loops is therefore at the HTTP boundary, where every inbound request can be assigned a hop budget and every host-driven outbound fetch can propagate the decremented remainder.

## Decisions

### Inbound enforcement
- Add a request middleware in `core-host` that reads `X-Tachyon-Hop-Limit`.
- Treat missing or invalid values as the default budget `10`.
- Reject `0` immediately with HTTP `508 Loop Detected` before guest execution starts.
- Store the parsed hop limit in request extensions so the handler can reuse it without reparsing headers.

### Outbound propagation
- Preserve the current `MESH_FETCH:` bridge model for legacy guests.
- When the host performs the outbound `reqwest` call, decrement the request hop limit and forward it via `X-Tachyon-Hop-Limit`.
- Allow relative targets such as `/api/guest-loop` by resolving them against the host listener address. This avoids hardcoding ports inside self-referential guests while keeping absolute URLs working for legacy-service calls.

### Loop regression coverage
- Add a `guest-loop` legacy guest that emits `MESH_FETCH:/api/guest-loop`.
- Add a host test that serves the app on an ephemeral port, invokes `/api/guest-loop`, and asserts the request chain terminates with HTTP `508`.

## Tradeoffs
- Invalid hop-limit headers are normalized to the safe default instead of rejected with `400`, which keeps the mesh resilient to malformed upstream callers without disabling the protection.
- Non-success mesh fetch responses still surface as gateway errors except for `508`, which is propagated so loop termination is observable by the original caller.
