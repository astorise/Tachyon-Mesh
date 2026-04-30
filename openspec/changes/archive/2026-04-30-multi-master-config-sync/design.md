# Design: Gossip-Triggered Synchronization

## 1. Config Versioning
The `integrity.lock` is extended to include a mandatory metadata block:
```json
{
  "metadata": {
    "version": 104,
    "timestamp": "2026-04-27T10:00:00Z",
    "signature": "..."
  }
}
```

## 2. Sync Workflow


### Step A: Reception (Node A)
- Node A receives the new configuration via the UI.
- It validates the signature and the version.
- It triggers a **Hot-Reload** locally.

### Step B: Notification (Gossip)
- Node A sends a message through `system-faas-gossip`: `topic: tachyon.config.sync, data: { version: 104, checksum: "sha256..." }`.

### Step C: Reconciliation (Node B, C...)
- Other nodes receive the gossip message.
- They check: `if message.version > local.version`.
- They initiate a secure P2P stream via `system-faas-mesh-overlay` to Node A to download the manifest and any new `.wasm` files.
- Once downloaded, they trigger their own **Hot-Reload**.

## 3. Tachyon-UI (Tachyon Studio)
The UI is updated to support a "Node List". If the connection to Node A fails, it automatically tries Node B or C to send the configuration.