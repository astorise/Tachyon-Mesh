# Proposal: Identity-Aware Rate Limiting

## Context
Our current Rate Limiter protects against DDoS by tracking requests per IP address (`rate-limiter-oom-protection`). In a Multi-Tenant enterprise environment, multiple users might share an outbound IP (NAT), or a single malicious tenant might use a botnet (many IPs) to exhaust their tier limit. We must enforce quotas based on the authenticated Identity (`CallerIdentityClaims`).

## Proposed Solution
Extend the Rate Limiting rule schema to define a `scope`.
- `scope: "ip"` (Default, Layer 4 protection)
- `scope: "tenant"` (Extracts the `tenant_id` from the JWT/mTLS)
- `scope: "token"` (Extracts the specific `client_id` or `jti`)

The Global CRDT map will use a composite key: `"{scope}:{identifier}:{route}"` (e.g., `tenant:acme-corp:/api/ai`).