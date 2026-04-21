# Proposal: Change 067-068 - Identity-Aware Connection Management

## Context
Tachyon Mesh requires a strictly Zero-Trust administrative interface. The current `tachyon-ui` relies on static mocked data and lacks the ability to connect to real edge nodes. Furthermore, the core router (`core-host`) currently has no mechanism to cryptographically verify who is connecting or what permissions they hold. To solve this, we must build a secure connection pipeline from the UI down to a pluggable WebAssembly authentication module.

## Objective
1. **The Gateway (UI/Client):** Implement a connection screen in `tachyon-ui` that requires a Mutual TLS (mTLS) profile bundle and an admin token to establish communication with any node.
2. **The Enforcer (Core Host):** Implement an API Middleware in the `core-host` that intercepts all administrative requests.
3. **The Brain (System FaaS):** Create `system-faas-auth`, a WebAssembly component responsible for parsing the token, validating signatures (OIDC/JWT), and enforcing Role-Based Access Control (RBAC).

## Scope
- Modify `tachyon-client` to support mTLS client certificates via `rustls` or `reqwest`.
- Implement the Connection Overlay in `tachyon-ui` using Tailwind and GSAP.
- Create the `system-faas-auth` crate implementing a new `tachyon:identity/auth` WIT interface.
- Wire the HTTP router in `core-host` to invoke `system-faas-auth` before serving data.