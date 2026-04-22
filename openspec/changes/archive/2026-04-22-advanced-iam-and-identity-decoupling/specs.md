# Specifications: Decoupled IAM Architecture

## 1. AuthN vs AuthZ Capability Split
- **`wit/authn.wit`**: Handles credentials.
  - `validate-token(token: string) -> result<identity-payload, error>`
  - `issue-pat(name: string, scopes: list<string>, ttl-days: u32) -> result<string, error>`
- **`wit/authz.wit`**: Handles permissions.
  - `evaluate-policy(ident: identity-payload, action: string, resource: string) -> result<bool, error>`

## 2. Core Host Pipeline (`server_h3.rs`)
The security interceptor must sequence the FaaS calls:
1. Extract Bearer token.
2. Call `authn::validate-token`. If ok, returns an `identity`.
3. Call `authz::evaluate-policy` with the `identity`, the HTTP method (action), and the path (resource).
4. If both succeed, route the request.

## 3. UI Account Space
The `#view-account` will feature:
- A "Security" block for 2FA regeneration.
- A "Developer Tokens" block to create short-lived API keys (PATs) with specific scopes (e.g., `deploy:wasm`, `read:nodes`).