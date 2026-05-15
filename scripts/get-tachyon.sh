#!/usr/bin/env bash
# get-tachyon.sh — zero-build installer for Tachyon-Mesh
# Downloads the pre-compiled core-host and tachyon-mcp binaries from the
# latest GitHub release and extracts them into the current directory.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/astorise/tachyon-mesh/main/scripts/get-tachyon.sh | bash
#   bash scripts/get-tachyon.sh [--version v1.2.3] [--dir /usr/local/bin]
set -euo pipefail

REPO="astorise/tachyon-mesh"
TARGET_DIR="."
VERSION=""

for arg in "$@"; do
  case "$arg" in
    --version=*) VERSION="${arg#*=}" ;;
    --version)   shift; VERSION="${1:-}" ;;
    --dir=*)     TARGET_DIR="${arg#*=}" ;;
    --dir)       shift; TARGET_DIR="${1:-}" ;;
  esac
done

BOLD=$(tput bold 2>/dev/null || true)
RESET=$(tput sgr0 2>/dev/null || true)
CYAN=$(tput setaf 6 2>/dev/null || true)
GREEN=$(tput setaf 2 2>/dev/null || true)
RED=$(tput setaf 1 2>/dev/null || true)

info() { echo "${CYAN}${BOLD}» $*${RESET}"; }
ok()   { echo "${GREEN}✔  $*${RESET}"; }
die()  { echo "${RED}${BOLD}✘  $*${RESET}" >&2; exit 1; }

# ── Prerequisites ─────────────────────────────────────────────────────────────
command -v curl >/dev/null 2>&1 || die "curl is required but not installed."
command -v tar  >/dev/null 2>&1 || die "tar is required but not installed."

# ── Resolve version ───────────────────────────────────────────────────────────
if [[ -z "$VERSION" ]]; then
  info "Fetching latest release tag from GitHub..."
  VERSION=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep '"tag_name":' \
    | sed -E 's/.*"([^"]+)".*/\1/')
  [[ -n "$VERSION" ]] || die "Could not determine latest release. Is the repo public?"
fi
ok "Version: ${VERSION}"

# ── Detect platform ───────────────────────────────────────────────────────────
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)
case "$ARCH" in
  x86_64)           ARCH="x86_64" ;;
  aarch64|arm64)    ARCH="aarch64" ;;
  *) die "Unsupported architecture: $ARCH" ;;
esac

case "$OS" in
  linux|darwin) ;;
  *) die "Unsupported OS: $OS. Use scripts/setup.ps1 on Windows." ;;
esac

TARBALL="tachyon-mesh-${VERSION}-${OS}-${ARCH}.tar.gz"
URL="https://github.com/${REPO}/releases/download/${VERSION}/${TARBALL}"
CHECKSUM_URL="${URL}.sha256"
ok "Platform: ${OS}/${ARCH}"

# ── Download ──────────────────────────────────────────────────────────────────
info "Downloading ${TARBALL}..."
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

HTTP_CODE=$(curl -fsSL -w "%{http_code}" -o "${TMP}/${TARBALL}" "$URL" 2>&1) || true
if [[ "$HTTP_CODE" != "200" ]]; then
  die "Download failed (HTTP ${HTTP_CODE}).
  URL: ${URL}

  If no release exists yet, build from source:
    git clone https://github.com/${REPO}.git && cd tachyon-mesh && ./scripts/setup.sh"
fi
ok "Downloaded ($(du -sh "${TMP}/${TARBALL}" | cut -f1))"

# ── Verify SHA-256 checksum ───────────────────────────────────────────────────
info "Verifying SHA-256 checksum..."
CHKSUM_CODE=$(curl -fsSL -w "%{http_code}" -o "${TMP}/${TARBALL}.sha256" "$CHECKSUM_URL" 2>&1) || true
if [[ "$CHKSUM_CODE" != "200" ]]; then
  die "Could not fetch checksum file (HTTP ${CHKSUM_CODE}). Aborting for safety."
fi
(cd "$TMP" && echo "$(cat "${TARBALL}.sha256")  ${TARBALL}" | sha256sum -c --status) \
  || die "SHA-256 checksum mismatch — the archive is corrupt or tampered with."
ok "Checksum verified"

# ── Extract ───────────────────────────────────────────────────────────────────
info "Extracting to ${TARGET_DIR}..."
mkdir -p "$TARGET_DIR"
tar -xzf "${TMP}/${TARBALL}" -C "$TARGET_DIR"
chmod +x "${TARGET_DIR}/core-host" "${TARGET_DIR}/tachyon-mcp" 2>/dev/null || true
ok "Extracted core-host and tachyon-mcp"

# ── Success banner ────────────────────────────────────────────────────────────
TARGET_ABS=$(cd "$TARGET_DIR" && pwd)
cat <<EOF

${GREEN}${BOLD}╔══════════════════════════════════════════════════════════╗
║        ✅  Tachyon-Mesh ${VERSION} ready!
╚══════════════════════════════════════════════════════════╝${RESET}

${BOLD}Start the mesh:${RESET}
  ${CYAN}${TARGET_ABS}/core-host${RESET}

${BOLD}MCP config for Claude Desktop / Cursor:${RESET}
${CYAN}{
  "mcpServers": {
    "tachyon-mesh": {
      "command": "${TARGET_ABS}/tachyon-mcp",
      "env": {
        "TACHYON_MCP_URL": "http://127.0.0.1:8080",
        "TACHYON_MCP_PAT": "YOUR_PAT"
      }
    }
  }
}${RESET}

For docs: ${CYAN}https://github.com/${REPO}${RESET}

EOF
