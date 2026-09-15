# get-tachyon.ps1 — Zero-build installer for Tachyon-Mesh on Windows
# Usage:
#   irm https://raw.githubusercontent.com/astorise/tachyon-mesh/main/scripts/get-tachyon.ps1 | iex
#   .\scripts\get-tachyon.ps1 [-Version v1.2.3] [-Dir C:\Tools\tachyon]
param(
    [string]$Version = "",
    [string]$Dir     = "."
)
$ErrorActionPreference = "Stop"

function Write-Info  { param($msg) Write-Host "  $msg" -ForegroundColor Cyan }
function Write-Ok    { param($msg) Write-Host "OK  $msg" -ForegroundColor Green }
function Write-Fail  { param($msg) Write-Host "FAIL $msg" -ForegroundColor Red; exit 1 }

$Repo = "astorise/tachyon-mesh"

# ── 1. Resolve version ────────────────────────────────────────────────────────
if ($Version -eq "") {
    Write-Info "Fetching latest release tag from GitHub..."
    try {
        $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -UseBasicParsing
        $Version = $release.tag_name
    } catch {
        Write-Fail "Could not fetch latest release: $_`nIs the repo public and reachable?"
    }
}
Write-Ok "Version: $Version"

# ── 2. Build download URL ─────────────────────────────────────────────────────
# Matches the artifact produced by publish-server-binaries in release.yml:
#   tachyon-mesh-{VERSION}-windows-x86_64.zip
$VersionNoV = $Version -replace '^v', ''
$ZipName    = "tachyon-mesh-${VersionNoV}-windows-x86_64.zip"
$DownloadUrl    = "https://github.com/$Repo/releases/download/$Version/$ZipName"
$ChecksumUrl    = "$DownloadUrl.sha256"

Write-Info "Downloading $ZipName..."

# ── 3. Download ───────────────────────────────────────────────────────────────
$TmpZip = Join-Path $env:TEMP "tachyon-mesh-$([System.IO.Path]::GetRandomFileName()).zip"
try {
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $TmpZip -UseBasicParsing
} catch {
    $status = $_.Exception.Response.StatusCode.value__
    Write-Host ""
    Write-Fail "Download failed (HTTP $status).`n  URL: $DownloadUrl`n`n  If no release exists yet, build from source:`n    git clone https://github.com/$Repo && cd tachyon-mesh && .\scripts\setup.ps1"
}
Write-Ok "Downloaded ($('{0:N1} MB' -f ((Get-Item $TmpZip).Length / 1MB)))"

# ── 3b. Verify SHA-256 checksum ───────────────────────────────────────────────
Write-Info "Verifying SHA-256 checksum..."
$TmpChksum = Join-Path $env:TEMP "tachyon-mesh-$([System.IO.Path]::GetRandomFileName()).sha256"
try {
    Invoke-WebRequest -Uri $ChecksumUrl -OutFile $TmpChksum -UseBasicParsing
} catch {
    Remove-Item $TmpZip -Force -ErrorAction SilentlyContinue
    Write-Fail "Could not fetch checksum file. Aborting for safety.`n  URL: $ChecksumUrl"
}
$ExpectedHash = (Get-Content $TmpChksum -Raw).Trim().ToUpper()
$ActualHash   = (Get-FileHash -Path $TmpZip -Algorithm SHA256).Hash.ToUpper()
Remove-Item $TmpChksum -Force -ErrorAction SilentlyContinue
if ($ActualHash -ne $ExpectedHash) {
    Remove-Item $TmpZip -Force -ErrorAction SilentlyContinue
    Write-Fail "SHA-256 checksum mismatch — the archive is corrupt or tampered with.`n  Expected: $ExpectedHash`n  Got:      $ActualHash"
}
Write-Ok "Checksum verified"

# ── 3c. Verify cosign signature (optional — proves the archive came from
# this repo's release workflow, not just that it matches a checksum served
# from the same place) ─────────────────────────────────────────────────────
$Cosign = Get-Command cosign -ErrorAction SilentlyContinue
if ($Cosign) {
    Write-Info "Verifying cosign signature..."
    $BundleUrl = "$DownloadUrl.bundle"
    $TmpBundle = Join-Path $env:TEMP "tachyon-mesh-$([System.IO.Path]::GetRandomFileName()).bundle"
    try {
        Invoke-WebRequest -Uri $BundleUrl -OutFile $TmpBundle -UseBasicParsing
    } catch {
        Remove-Item $TmpZip -Force -ErrorAction SilentlyContinue
        Write-Fail "cosign is installed but the signature bundle could not be fetched.`n  URL: $BundleUrl`n  Re-run without cosign on PATH to skip this check, or verify manually once the bundle is available."
    }
    & cosign verify-blob `
        --bundle $TmpBundle `
        --certificate-identity-regexp "^https://github\.com/$Repo/\.github/workflows/release\.yml@" `
        --certificate-oidc-issuer "https://token.actions.githubusercontent.com" `
        $TmpZip
    if ($LASTEXITCODE -ne 0) {
        Remove-Item $TmpZip, $TmpBundle -Force -ErrorAction SilentlyContinue
        Write-Fail "cosign signature verification failed — the archive does not match a signature from this repo's release workflow."
    }
    Remove-Item $TmpBundle -Force -ErrorAction SilentlyContinue
    Write-Ok "Signature verified"
} else {
    Write-Info "cosign not found on PATH — skipping signature verification (SHA-256 checksum above still applies). Install cosign (https://docs.sigstore.dev/cosign/installation/) to also verify the archive was signed by this repo's release workflow."
}

# ── 4. Extract ────────────────────────────────────────────────────────────────
Write-Info "Extracting to $Dir..."
New-Item -ItemType Directory -Path $Dir -Force | Out-Null
Expand-Archive -Path $TmpZip -DestinationPath $Dir -Force
Remove-Item $TmpZip -Force
Write-Ok "Extracted core-host.exe and tachyon-mcp.exe"

# ── 5. Success banner ─────────────────────────────────────────────────────────
$AbsDir = (Resolve-Path $Dir).Path
$McpBin = Join-Path $AbsDir "tachyon-mcp.exe"

Write-Host ""
Write-Host "╔══════════════════════════════════════════════════════════╗" -ForegroundColor Green
Write-Host "║   OK  Tachyon-Mesh $Version ready!                      " -ForegroundColor Green
Write-Host "╚══════════════════════════════════════════════════════════╝" -ForegroundColor Green
Write-Host ""
Write-Host "Start the mesh:" -ForegroundColor White
Write-Host "  $AbsDir\core-host.exe" -ForegroundColor Cyan
Write-Host ""
Write-Host "MCP config for Claude Desktop / Cursor:" -ForegroundColor White
$escaped = $McpBin -replace '\\', '\\'
Write-Host @"
{
  "mcpServers": {
    "tachyon-mesh": {
      "command": "$escaped",
      "env": {
        "TACHYON_MCP_URL": "http://127.0.0.1:8080",
        "TACHYON_MCP_PAT": "YOUR_PAT"
      }
    }
  }
}
"@ -ForegroundColor Cyan
Write-Host ""
Write-Host "Docs: https://github.com/$Repo" -ForegroundColor Cyan
