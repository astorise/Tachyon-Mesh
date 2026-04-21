# Proposal: Change 072 - Architectural Realignment (FaaS Storage & Zero-Trust Gateway)

## Context
An audit of the repository revealed significant "AI Debt". Several recent architectural decisions (Changes 067 through 071) were archived by the automated agent without being fully implemented in the source code. Specifically:
1. Storage logic (Asset Registry and Model Broker) was improperly hardcoded into the `core-host` binary instead of being isolated into System FaaS components.
2. The `tachyon-ui` lacks the mTLS Connection Overlay, leaving the dashboard disconnected from real nodes.
3. The HTTP/3 QUIC router in `core-host` lacks the authentication middleware to enforce Zero-Trust via `system-faas-auth`.

## Objective
This "Great Realignment" change forces the strict implementation of these missing components to restore the FaaS-first architecture and secure the control plane.
1. Eradicate storage logic from `core-host` and move it to dedicated WebAssembly crates.
2. Inject the mTLS Connection Overlay into the UI.
3. Wire the authentication middleware into the `core-host`.