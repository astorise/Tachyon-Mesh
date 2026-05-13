# Technical Specification: Operator Onboarding

## 1. The Installer Script (`scripts/get-tachyon.sh`)
Create a bash script that hits the GitHub API to find the latest release and downloads the pre-compiled binaries.

```bash
#!/usr/bin/env bash
set -e

REPO="astorise/tachyon-mesh"
echo "🌀 Fetching latest Tachyon-Mesh release..."

# Fetch latest release data from GitHub API
LATEST_RELEASE=$(curl -s [https://api.github.com/repos/$REPO/releases/latest](https://api.github.com/repos/$REPO/releases/latest) | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')
echo "📦 Latest version: $LATEST_RELEASE"

# Determine OS and Architecture
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

# Map arch to rust targets (simplified example)
if [ "$ARCH" = "x86_64" ]; then ARCH="x86_64"; fi
if [ "$ARCH" = "aarch64" ] || [ "$ARCH" = "arm64" ]; then ARCH="aarch64"; fi

# Download the tarball
URL="[https://github.com/$REPO/releases/download/$LATEST_RELEASE/tachyon-mesh-$LATEST_RELEASE-$OS-$ARCH.tar.gz](https://github.com/$REPO/releases/download/$LATEST_RELEASE/tachyon-mesh-$LATEST_RELEASE-$OS-$ARCH.tar.gz)"
echo "⬇️ Downloading from $URL..."
curl -L -o tachyon-mesh.tar.gz "$URL"

# Extract and cleanup
tar -xzf tachyon-mesh.tar.gz
rm tachyon-mesh.tar.gz

echo "✅ Tachyon-Mesh downloaded successfully!"
echo "Run './core-host' to start the mesh, or './tachyon-mcp' for the Agent server."
```

## 2. README Architecture
Update the root `README.md` to prioritize the operator paths.

```markdown
## 🚀 Quick Start

### Path A: For Operators (Zero-Build)
The fastest way to run Tachyon-Mesh on your machine.

**Local Binary:**
\`\`\`bash
curl -fsSL [https://raw.githubusercontent.com/astorise/tachyon-mesh/main/scripts/get-tachyon.sh](https://raw.githubusercontent.com/astorise/tachyon-mesh/main/scripts/get-tachyon.sh) | bash
./core-host
\`\`\`

**Kubernetes:**
\`\`\`bash
kubectl apply -f [https://raw.githubusercontent.com/astorise/tachyon-mesh/main/manifests/deploy.yaml](https://raw.githubusercontent.com/astorise/tachyon-mesh/main/manifests/deploy.yaml)
\`\`\`

### Path B: For Contributors (Build from Source)
If you want to modify the core engine or UI.
\`\`\`bash
git clone [https://github.com/astorise/tachyon-mesh.git](https://github.com/astorise/tachyon-mesh.git)
cd tachyon-mesh
./scripts/setup.sh
\`\`\`
```