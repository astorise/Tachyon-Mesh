# Technical Specification: Verified Onboarding

## 1. Bash Script Hardening (`scripts/get-tachyon.sh`)
Add logic to fetch the checksum file and verify the tarball before extraction.

```bash
# ... after downloading $URL into tachyon-mesh.tar.gz

echo "🛡️ Fetching checksum for verification..."
CHECKSUM_URL="[https://github.com/$REPO/releases/download/$LATEST_RELEASE/tachyon-mesh-$LATEST_RELEASE-$OS-$ARCH.tar.gz.sha256](https://github.com/$REPO/releases/download/$LATEST_RELEASE/tachyon-mesh-$LATEST_RELEASE-$OS-$ARCH.tar.gz.sha256)"
curl -sL -o tachyon-mesh.tar.gz.sha256 "$CHECKSUM_URL"

echo "🔍 Verifying integrity..."
if ! sha256sum -c tachyon-mesh.tar.gz.sha256 > /dev/null 2>&1; then
    echo "❌ CRITICAL: Checksum verification failed. The downloaded file may be corrupted or compromised."
    exit 1
fi
echo "✅ Integrity verified."

# ... proceed with extraction
```

## 2. PowerShell Script Hardening (`scripts/get-tachyon.ps1`)
Mirror the bash verification logic in PowerShell.

```powershell
# ... after downloading the zip file

Write-Host "🛡️ Fetching checksum for verification..." -ForegroundColor Cyan
$checksumUrl = "[https://github.com/$repo/releases/download/$version/tachyon-mesh-windows-amd64.zip.sha256](https://github.com/$repo/releases/download/$version/tachyon-mesh-windows-amd64.zip.sha256)"
Invoke-WebRequest -Uri $checksumUrl -OutFile "tachyon-mesh.zip.sha256"

Write-Host "🔍 Verifying integrity..." -ForegroundColor Cyan
# Parse the expected hash from the file (assumes standard sha256sum format: HASH  FILENAME)
$expectedHash = (Get-Content "tachyon-mesh.zip.sha256" | Select-String -Pattern "^([a-fA-F0-9]{64})").Matches.Groups[1].Value

$computedHash = (Get-FileHash -Path "tachyon-mesh.zip" -Algorithm SHA256).Hash.ToLower()

if ($computedHash -ne $expectedHash.ToLower()) {
    Write-Host "❌ CRITICAL: Checksum verification failed. Expected $expectedHash, got $computedHash." -ForegroundColor Red
    exit 1
}
Write-Host "✅ Integrity verified." -ForegroundColor Green

# ... proceed with Expand-Archive
```