# Tachyon-Mesh — one-shot bootstrap script (Windows / PowerShell)
# Usage: .\scripts\setup.ps1 [-SkipGuests] [-SkipUI]
param(
    [switch]$SkipGuests,
    [switch]$SkipUI
)
$ErrorActionPreference = "Stop"

function Write-Info  { param($msg) Write-Host "» $msg" -ForegroundColor Cyan }
function Write-Ok    { param($msg) Write-Host "✔  $msg" -ForegroundColor Green }
function Write-Fail  { param($msg) Write-Host "✘  $msg" -ForegroundColor Red; exit 1 }

# ── 1. Prerequisites ──────────────────────────────────────────────────────────
Write-Info "Checking prerequisites..."
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Fail "Rust/Cargo is required but not installed. Install from https://rustup.rs"
}
if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) {
    Write-Fail "rustup is required. Install from https://rustup.rs"
}
if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {
    Write-Fail "Node.js/npm is required but not installed. Install from https://nodejs.org"
}
Write-Ok "cargo $(cargo --version)"
Write-Ok "npm   $(npm --version)"

# ── 2. WASM targets ───────────────────────────────────────────────────────────
Write-Info "Ensuring WASM targets are installed..."
rustup target add wasm32-wasip1 wasm32-wasip2
Write-Ok "WASM targets ready"

# ── 3. Build core binaries ────────────────────────────────────────────────────
Write-Info "Building core-host and tachyon-mcp (release)..."
cargo build --release --bin core-host --bin tachyon-mcp
Write-Ok "core-host and tachyon-mcp built"

# ── 4. FaaS guest artifacts ───────────────────────────────────────────────────
if (-not $SkipGuests) {
    Write-Info "Building standard FaaS guest artifacts..."
    # Prefer bash (Git Bash / WSL); fall back to PowerShell line-by-line execution
    if (Get-Command bash -ErrorAction SilentlyContinue) {
        bash scripts/build-guest-artifacts.sh
    } else {
        Write-Host "  bash not found — skipping guest build. Run 'bash scripts/build-guest-artifacts.sh' in WSL/Git Bash." -ForegroundColor Yellow
    }
    Write-Ok "Guest artifact step complete"
} else {
    Write-Host "  (skipping guest artifact build — -SkipGuests passed)"
}

# ── 5. UI dependencies ────────────────────────────────────────────────────────
if (-not $SkipUI) {
    Write-Info "Installing Tachyon-UI npm dependencies..."
    Push-Location tachyon-ui
    npm install
    Pop-Location
    Write-Ok "UI dependencies installed"
} else {
    Write-Host "  (skipping UI install — -SkipUI passed)"
}

# ── 6. Cross-layer validation ─────────────────────────────────────────────────
Write-Info "Running cross-layer validation..."
if (Get-Command bash -ErrorAction SilentlyContinue) {
    bash scripts/validate_cross_layer.sh
    Write-Ok "Cross-layer validation passed"
} else {
    Write-Host "  bash not found — skipping cross-layer validation. Run in WSL/Git Bash." -ForegroundColor Yellow
}

# ── 7. Success banner ─────────────────────────────────────────────────────────
$RepoRoot = (Resolve-Path "$PSScriptRoot\..").Path
$McpBin   = Join-Path $RepoRoot "target\release\tachyon-mcp.exe"

Write-Host ""
Write-Host "╔══════════════════════════════════════════════════════════╗" -ForegroundColor Green
Write-Host "║        ✅  Tachyon-Mesh setup complete!                  ║" -ForegroundColor Green
Write-Host "╚══════════════════════════════════════════════════════════╝" -ForegroundColor Green
Write-Host ""
Write-Host "Start the cluster:" -ForegroundColor White
Write-Host "  Terminal 1 (Core):  .\target\release\core-host.exe" -ForegroundColor Cyan
Write-Host "  Terminal 2 (UI):    cd tachyon-ui; npm run tauri dev" -ForegroundColor Cyan
Write-Host ""
Write-Host "AI Agent integration (Claude Desktop / Cursor):" -ForegroundColor White
Write-Host "Add to your MCP config:" -ForegroundColor White
Write-Host @"
{
  "mcpServers": {
    "tachyon-mesh": {
      "command": "$McpBin",
      "env": {
        "TACHYON_MCP_URL": "http://127.0.0.1:8080",
        "TACHYON_MCP_PAT": "local-dev-token"
      }
    }
  }
}
"@ -ForegroundColor Cyan
Write-Host "For full MCP docs: docs/mcp-setup.md" -ForegroundColor Cyan
Write-Host ""
