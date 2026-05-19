# Quickstart: LLM Agents via MCP

Tachyon Mesh ships a [Model Context Protocol](https://modelcontextprotocol.io/) server (`tachyon-mcp`) that exposes the mesh control-plane as a set of callable tools. Any MCP-compatible client — Claude Desktop, Cursor, or a custom agent — can query cluster health, stream logs, validate manifests, and run chaos scenarios without writing a single line of API glue code.

---

## Prerequisites

- A running `core-host` instance (see [Quick Start](../README.md#-quick-start))
- A Personal Access Token (PAT) — generate one with `tachyon-ui` or via `POST /admin/security/pats`
- Rust toolchain installed (`cargo`)

---

## Configuration

### Claude Desktop

Add the following block to `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS) or `%APPDATA%\Claude\claude_desktop_config.json` (Windows):

```json
{
  "mcpServers": {
    "tachyon-mesh": {
      "command": "cargo",
      "args": ["run", "--manifest-path", "/path/to/tachyon-mesh/Cargo.toml", "--bin", "tachyon-mcp"],
      "env": {
        "TACHYON_MCP_URL": "http://127.0.0.1:8080",
        "TACHYON_MCP_PAT": "your-personal-access-token"
      }
    }
  }
}
```

> **Tip:** If you have already built the binary, replace `cargo run …` with the path to the compiled artifact:
> ```json
> { "command": "/path/to/tachyon-mcp", "args": [] }
> ```

### Cursor

Open **Settings → Features → MCP Servers → Add Server** and paste:

```json
{
  "name": "tachyon-mesh",
  "command": "cargo",
  "args": ["run", "--manifest-path", "/path/to/tachyon-mesh/Cargo.toml", "--bin", "tachyon-mcp"],
  "env": {
    "TACHYON_MCP_URL": "http://127.0.0.1:8080",
    "TACHYON_MCP_PAT": "your-personal-access-token"
  }
}
```

### Custom agent (stdio transport)

```bash
TACHYON_MCP_URL=http://127.0.0.1:8080 \
TACHYON_MCP_PAT=pat_... \
  cargo run --bin tachyon-mcp
```

The server speaks JSON-RPC 2.0 over stdin/stdout.

---

## Available tools

| Tool | Description |
|------|-------------|
| `tachyon_mesh_status` | Liveness and sealed-config version of the connected node |
| `tachyon_get_manifest` | Fetch the current sealed `IntegrityConfig` |
| `tachyon_validate_manifest` | Dry-run a manifest JSON against the host |
| `tachyon_get_metrics` | Error rate, p50/p99 latency, queue depth |
| `tachyon_tail_logs` | Fetch the last N lines of the audit log |
| `tachyon_get_shadow_diffs` | Shadow-proxy divergence reports |
| `tachyon_run_chaos_scenario` | Trigger a named chaos harness scenario |
| `tachyon_hardware_status` | Local RAM and accelerator inventory |
| `validate_faas_capabilities` | Check whether a hardware policy is admissible |

---

## Environment variables

| Variable | Required | Description |
|----------|----------|-------------|
| `TACHYON_MCP_URL` | Yes | Base URL of a running `core-host` (e.g. `http://127.0.0.1:8080`) |
| `TACHYON_MCP_PAT` | Yes | Personal Access Token with operator privileges |

## Available resources

| URI | Description |
| --- | --- |
| `hardware://local/status` | Local RAM and accelerator snapshot from the MCP host. |
| `hardware://mesh/cluster` | Enrolled-node, RAM, and GPU summary from the Tachyon node registry. |
| `hardware://mesh/{node_id}/status` | Per-node hardware capabilities for an enrolled mesh node. |

Both variables can also be passed as CLI arguments: `--url <url> --token <pat>`.

---

## Troubleshooting

- **`TACHYON_MCP_PAT` not set** — the server exits immediately with a descriptive error. Ensure the environment variable is present in the MCP host configuration.
- **Connection refused** — verify `core-host` is running and `TACHYON_MCP_URL` points to the correct address and port.
- **Rate limit exceeded** — each tool has a per-minute call cap. Wait 60 seconds or reduce agent call frequency.
