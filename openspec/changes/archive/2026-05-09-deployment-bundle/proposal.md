# Title: Smart Deployment Pipeline: Bundling, Server-Side Resolution, and Interactive Feedback

## Problem Statement
Following the UI/MCP audit closure, the deployment pipeline needs an architectural overhaul to support composite functional domains (like the Pulsar project). 
1. Configurations and their WebAssembly dependencies must be deployed atomically.
2. In a multi-tenant Edge cluster, the client cannot safely resolve global dependencies or generate the final cryptographic lockfile locally.
3. If the cluster dynamically resolves a SemVer dependency to a better cached version, overriding the user's bundled asset silently, it creates unpredictable deployments and frustrates operators.

## Objective
Implement a "Smart Deployment Pipeline":
1. **Client Bundling:** The client (UI/MCP) builds a `.tar.gz` bundle containing a declarative `manifest.yaml` (with SemVer dependencies) and local WASM assets.
2. **Server-Side Resolution:** The Tachyon host receives the bundle, validates it via a system FaaS, resolves SemVer constraints against its registry, and generates the final `integrity.lock`.
3. **Interactive "Plan & Confirm":** If the host detects a conflict (e.g., the bundled asset is shadowed by a better version in the cluster cache), it halts and returns an HTTP 428 error. The UI intercepts this and prompts the user to either force the local version or use the optimized cluster version.