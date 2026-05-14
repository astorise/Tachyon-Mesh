# Technical Specification: Windows Onboarding & Schema Docs

## 1. Windows Downloader (`scripts/get-tachyon.ps1`)
Create a robust PowerShell script for downloading and unpacking the release.

```powershell
# scripts/get-tachyon.ps1
$ErrorActionPreference = "Stop"

Write-Host "🌀 Fetching latest Tachyon-Mesh release for Windows..." -ForegroundColor Cyan
$repo = "astorise/tachyon-mesh"

# Fetch latest release data
$releaseUrl = "[https://api.github.com/repos/$repo/releases/latest](https://api.github.com/repos/$repo/releases/latest)"
$release = Invoke-RestMethod -Uri $releaseUrl
$version = $release.tag_name
Write-Host "📦 Latest version: $version"

# Download the zip
$downloadUrl = "[https://github.com/$repo/releases/download/$version/tachyon-mesh-windows-amd64.zip](https://github.com/$repo/releases/download/$version/tachyon-mesh-windows-amd64.zip)"
Write-Host "⬇️ Downloading from $downloadUrl..."
Invoke-WebRequest -Uri $downloadUrl -OutFile "tachyon-mesh.zip"

# Extract and cleanup
Write-Host "📂 Extracting files..."
Expand-Archive -Path "tachyon-mesh.zip" -DestinationPath "." -Force
Remove-Item "tachyon-mesh.zip"

Write-Host "✅ Tachyon-Mesh downloaded successfully!" -ForegroundColor Green

# Print MCP Configuration instructions
$currentPath = (Get-Item .).FullName -replace '\\', '\\'
Write-Host "`n🤖 For AI Agents (Claude Desktop/Cursor):`nAdd this to your configuration:" -ForegroundColor Yellow
Write-Host @"
{
  "mcpServers": {
    "tachyon-mesh": {
      "command": "$currentPath\\tachyon-mcp.exe",
      "env": {
        "TACHYON_MCP_URL": "[http://127.0.0.1:8080](http://127.0.0.1:8080)",
        "TACHYON_MCP_PAT": "local-dev-token"
      }
    }
  }
}
"@
```

## 2. IDE Integration Documentation (`docs/ide-integration.md`)
Create a new file explaining how to consume the dynamic schema.

```markdown
# IDE Integration & Schema Validation

Tachyon-Mesh dynamically serves JSON schemas for its configuration files. You can bind these to your IDE to get real-time validation and autocompletion.

## 1. VS Code Configuration
Ensure your `core-host` is running (port 8080).

### For JSON files (e.g., `integrity.lock`)
Add the following to your workspace's `.vscode/settings.json`:

\`\`\`json
{
  "json.schemas": [
    {
      "fileMatch": ["*integrity.lock"],
      "url": "[http://127.0.0.1:8080/admin/schema/integrity-lock](http://127.0.0.1:8080/admin/schema/integrity-lock)"
    }
  ]
}
\`\`\`

### For YAML manifests (e.g., `deploy.yaml`)
If using the Red Hat YAML extension, add this to the top of your YAML file:
\`\`\`yaml
# yaml-language-server: $schema=[http://127.0.0.1:8080/admin/schema/manifest](http://127.0.0.1:8080/admin/schema/manifest)
\`\`\`

## 2. JetBrains (IntelliJ, RustRover)
1. Go to **Settings > Languages & Frameworks > Schemas and DTDs > JSON Schema Mappings**.
2. Add a new mapping, paste `http://127.0.0.1:8080/admin/schema/integrity-lock` as the Schema URL, and set the file path pattern to `integrity.lock`.
```

## 3. README Update
In the `README.md`, update the Quick Start section to show the Windows alternative:
```markdown
**Local Binary (Windows):**
\`\`\`powershell
irm [https://raw.githubusercontent.com/astorise/tachyon-mesh/main/scripts/get-tachyon.ps1](https://raw.githubusercontent.com/astorise/tachyon-mesh/main/scripts/get-tachyon.ps1) | iex
.\core-host.exe
\`\`\`
```
*Also, add a link to `docs/ide-integration.md` in the Developer Guide section.*