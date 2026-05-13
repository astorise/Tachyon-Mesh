#!/usr/bin/env bash
# Tachyon-Mesh — one-shot bootstrap script
# Usage: ./scripts/setup.sh [--skip-guests] [--skip-ui]
set -euo pipefail

SKIP_GUESTS=0
SKIP_UI=0
for arg in "$@"; do
  case "$arg" in
    --skip-guests) SKIP_GUESTS=1 ;;
    --skip-ui)     SKIP_UI=1 ;;
  esac
done

BOLD=$(tput bold 2>/dev/null || true)
RESET=$(tput sgr0 2>/dev/null || true)
GREEN=$(tput setaf 2 2>/dev/null || true)
CYAN=$(tput setaf 6 2>/dev/null || true)
RED=$(tput setaf 1 2>/dev/null || true)

info()  { echo "${CYAN}${BOLD}» $*${RESET}"; }
ok()    { echo "${GREEN}✔  $*${RESET}"; }
die()   { echo "${RED}${BOLD}✘  $*${RESET}" >&2; exit 1; }

# ── 1. Prerequisites ──────────────────────────────────────────────────────────
info "Checking prerequisites..."
command -v cargo  >/dev/null 2>&1 || die "Rust/Cargo is required but not installed. Install from https://rustup.rs"
command -v rustup >/dev/null 2>&1 || die "rustup is required but not installed. Install from https://rustup.rs"
command -v npm    >/dev/null 2>&1 || die "Node.js/npm is required but not installed. Install from https://nodejs.org"
ok "cargo $(cargo --version 2>&1 | head -1)"
ok "npm   $(npm --version 2>&1)"

# ── 2. WASM targets ───────────────────────────────────────────────────────────
info "Ensuring WASM targets are installed..."
rustup target add wasm32-wasip1 wasm32-wasip2
ok "WASM targets ready"

# ── 3. Build core binaries ────────────────────────────────────────────────────
info "Building core-host and tachyon-mcp (release)..."
cargo build --release --bin core-host --bin tachyon-mcp
ok "core-host and tachyon-mcp built"

# ── 4. Build FaaS guest artifacts ─────────────────────────────────────────────
if [[ "$SKIP_GUESTS" -eq 0 ]]; then
  info "Building standard FaaS guest artifacts..."
  bash scripts/build-guest-artifacts.sh
  ok "Guest artifacts built"
else
  echo "  (skipping guest artifact build — --skip-guests passed)"
fi

# ── 5. UI dependencies ────────────────────────────────────────────────────────
if [[ "$SKIP_UI" -eq 0 ]]; then
  info "Installing Tachyon-UI npm dependencies..."
  (cd tachyon-ui && npm install)
  ok "UI dependencies installed"
else
  echo "  (skipping UI install — --skip-ui passed)"
fi

# ── 6. Cross-layer validation ─────────────────────────────────────────────────
info "Running cross-layer validation..."
bash scripts/validate_cross_layer.sh
ok "Cross-layer validation passed"

# ── 7. Success banner ─────────────────────────────────────────────────────────
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MCP_BIN="${REPO_ROOT}/target/release/tachyon-mcp"

cat <<EOF

${GREEN}${BOLD}╔══════════════════════════════════════════════════════════╗
║        ✅  Tachyon-Mesh setup complete!                  ║
╚══════════════════════════════════════════════════════════╝${RESET}

${BOLD}Start the cluster:${RESET}
  Terminal 1 (Core):  ${CYAN}./target/release/core-host${RESET}
  Terminal 2 (UI):    ${CYAN}cd tachyon-ui && npm run tauri dev${RESET}

${BOLD}AI Agent integration (Claude Desktop / Cursor):${RESET}
Add to your MCP config:

${CYAN}{
  "mcpServers": {
    "tachyon-mesh": {
      "command": "${MCP_BIN}",
      "env": {
        "TACHYON_MCP_URL": "http://127.0.0.1:8080",
        "TACHYON_MCP_PAT": "local-dev-token"
      }
    }
  }
}${RESET}

For Claude Desktop: ${CYAN}~/Library/Application Support/Claude/claude_desktop_config.json${RESET}
For full MCP docs:  ${CYAN}docs/mcp-setup.md${RESET}

EOF
