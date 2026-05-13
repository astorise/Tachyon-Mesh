# Technical Specification: The Bootstrap Script

## 1. Setup Script Creation (`scripts/setup.sh`)
Create a robust, idempotent bash script.

```bash
#!/usr/bin/env bash
set -e

echo "🌀 Bootstrapping Tachyon-Mesh environment..."

# 1. Check prerequisites
command -v cargo >/dev/null 2>&1 || { echo >&2 "❌ Rust/Cargo is required but not installed. Aborting."; exit 1; }
command -v npm >/dev/null 2>&1 || { echo >&2 "❌ Node.js/npm is required but not installed. Aborting."; exit 1; }

# 2. WASM Target
echo "📦 Ensuring wasm32-wasip2 target is installed..."
rustup target add wasm32-wasip2

# 3. Build Core
echo "🏗️ Building Core Host & MCP Server..."
cargo build --release --bin core-host
cargo build --release --bin tachyon-mcp

# 4. Build Default Guests
echo "🧩 Building standard FaaS guests..."
./scripts/build-guest-artifacts.sh examples/guest-example
# Assume the script generates/updates integrity.lock

# 5. UI Setup
echo "🎨 Installing Tachyon-UI dependencies..."
cd tachyon-ui
npm install
cd ..

# 6. Final Instructions
cat << EOF

✅ Setup Complete! Tachyon-Mesh is ready.

Terminal 1 (Core):
  ./target/release/core-host

Terminal 2 (UI):
  cd tachyon-ui && npm run tauri dev

🤖 For AI Agents (Claude Desktop/Cursor):
Add this to your configuration:
{
  "mcpServers": {
    "tachyon-mesh": {
      "command": "$(pwd)/target/release/tachyon-mcp",
      "env": {
        "TACHYON_MCP_URL": "[http://127.0.0.1:8080](http://127.0.0.1:8080)",
        "TACHYON_MCP_PAT": "local-dev-token"
      }
    }
  }
}
EOF
```