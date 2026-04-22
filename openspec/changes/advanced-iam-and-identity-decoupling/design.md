# Design: advanced-iam-and-identity-decoupling

## Overview
The change preserves the existing administrative connection bootstrap while replacing the single auth component with a two-stage pipeline:

1. AuthN validates a presented credential and returns an identity payload.
2. AuthZ evaluates the resolved identity against the requested action and resource.

## AuthN
- `system-faas-authn` owns JWT verification, recovery-code state, and PAT issuance.
- PATs are generated once in plaintext, hashed before persistence, and resolved by scanning the auth state store for matching hashes.
- The AuthN identity payload contains `subject`, `roles`, and `scopes`.

## AuthZ
- `system-faas-authz` is stateless and evaluates method/path decisions from the host.
- `admin` role remains a full-access shortcut.
- PAT scopes are matched against coarse route buckets such as asset deployment, model uploads, token management, and security recovery flows.

## Host integration
- `core-host/src/auth.rs` remains the entry point for admin middleware, but it instantiates two components instead of one.
- Both the HTTP/1.1 middleware and the HTTP/3 admin fast-path call the same authorize helper so the behavior stays consistent.

## Desktop integration
- `tachyon-client` exposes PAT issuance through the authenticated admin connection.
- `tachyon-ui` adds a `My Account` view for operator-scoped security actions without replacing the existing shared Identity view.
