# Proposal: Change 027 - Pluggable Auth Middleware & Credential Graph Validation

## Context
Hardcoding authentication (JWT) and authorization (Zanzibar) into the `core-host` violates our modular architecture. Instead, Auth should be implemented as a pluggable "System FaaS" middleware. Furthermore, to prevent runtime permission errors in deeply nested FaaS dependency chains (e.g., FaaS A calls FaaS B), we must validate the required "credential scopes" at deployment time, just as we do for SemVer dependencies.

## Objective
1. Introduce the concept of a `middleware` module in the `RouteConfig`.
2. Update the `integrity.lock` schema so targets can declare required `credentials` (scopes/permissions) and dependencies can map them.
3. Extend the Host's Startup Graph Validator (from Change 025) to ensure that if Target B requires credential `c2`, Target A (which depends on B) must explicitly acknowledge and possess the right to propagate `c2`.
4. Create a `system-faas-auth` guest that intercepts the HTTP request, validates the JWT, queries Zanzibar (or any external PDP), and either aborts the request or passes it to the business FaaS.

## Scope
- Update `tachyon-cli` to support `middlewares` and `credentials` arrays.
- Extend the `core-host` Startup Validator to perform static analysis of the credential delegation graph.
- Implement the Axum middleware logic to execute the `middleware` WASM module *before* the target WASM module.
- Build the `system-faas-auth` module using standard HTTP/WIT capabilities.

## Success Metrics
- If FaaS A depends on FaaS B, and B requires `c2`, but A does not declare `c2` in its delegated credentials, the `core-host` panics at startup.
- A request to a protected route first triggers `system-faas-auth`. If it returns a `403 Forbidden`, the execution chain stops instantly, and the business FaaS is never loaded into memory.