# Proposal: Security & Identity Configuration Schema

## Context
Following the Routing configuration schema, the control plane needs a declarative model for Security and Identity. Tachyon Mesh handles Zero-Trust verification and distributed rate-limiting natively at the Edge.

## Problem
Hardcoding security policies or rate-limiting thresholds in Wasm modules limits the flexibility of the Mesh. We need a unified GitOps schema that allows Tachyon-UI to dynamically update JWT providers, RBAC rules, and distributed CRDT quotas without restarting the data-plane.

## Solution
Create the `config-security.wit` contract and its GitOps YAML equivalent. The schema is divided into:
1. **Authentication (Authn)**: Identity Providers (OIDC/JWT) and claim extraction (e.g., Tenant ID).
2. **Authorization (Authz)**: RBAC/ABAC rules mapping identities to allowed Routes.
3. **Quotas & Rate Limiting**: Distributed limits mapped to extracted identities (CRDT algorithm).