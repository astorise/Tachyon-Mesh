# Design: GitOps Configuration Architecture

## Component Responsibilities

### 1. `system-faas-config-api` (Brain)
- Exposes REST/gRPC/MCP endpoints on the admin port (e.g., 8443).
- Validates configuration payloads (JSON/YAML) against WIT schemas.
- Sends valid modification intents to the GitOps broker.

### 2. `system-faas-gitops-broker` (Muscle)
- Built using the `gix` (gitoxide) pure Rust crate for Wasm compatibility.
- Relies on a WASI preopened directory to persist the `.git` store locally.
- Handles atomic commits, branching, and merges in memory/local disk.
- Syncs the local repository to `system-faas-s3-proxy` asynchronously.

## Offline-First & Fast-Boot Resilience
On `core-host` startup, the GitOps broker instantly reads the local WASI configuration volume. It applies the last known good configuration in zero milliseconds without blocking the data-plane. An asynchronous fetch from S3 is spawned in the background.

## Multi-Environment Branching Model
- **Node Tagging**: Nodes enroll with environment tags (e.g., `--tag env=production`).
- **Pub/Sub Gossip**: The `gitops-broker` subscribes to gossip events matching its environment tag branch.
- **Promotion Flow**:
  1. UI pushes mutation to `dev` branch. `dev` nodes receive gossip and hot-reload.
  2. UI requests promotion. Broker executes `git merge dev -> staging`.
  3. Rollbacks execute `git reset --hard HEAD~1` on the target branch.

## Event-Driven Distribution
When a config is checked out/merged, the Wasm component emits a `ConfigUpdate` event. The host's Tokio broadcast channel routes this event to active background workers (Rate Limiter, Router, Telemetry) to swap their internal `Arc<State>` pointers atomically.
