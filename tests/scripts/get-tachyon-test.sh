#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd "$(dirname "$0")/../.." && pwd)
test_dir=$(mktemp -d)
trap 'rm -rf "$test_dir"' EXIT
mkdir -p "$test_dir/bin" "$test_dir/payload"
printf '#!/bin/sh\necho core\n' > "$test_dir/payload/core-host"
printf '#!/bin/sh\necho mcp\n' > "$test_dir/payload/tachyon-mcp"
chmod +x "$test_dir/payload/"*
tar -czf "$test_dir/archive.tar.gz" -C "$test_dir/payload" core-host tachyon-mcp
sha256sum "$test_dir/archive.tar.gz" | cut -d ' ' -f 1 > "$test_dir/checksum"
cat > "$test_dir/bin/curl" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
output=""
url=""
while (($#)); do
  case "$1" in
    -o) output=$2; shift 2 ;;
    -w) shift 2 ;;
    -*) shift ;;
    *) url=$1; shift ;;
  esac
done
printf '%s\n' "$url" >> "$TEST_DIR/urls"
case "$url" in
  *.sha256) cp "$TEST_DIR/checksum" "$output" ;;
  *.tar.gz) cp "$TEST_DIR/archive.tar.gz" "$output" ;;
  *) echo "Unexpected URL: $url" >&2; exit 1 ;;
esac
printf 200
STUB
chmod +x "$test_dir/bin/curl"
export TEST_DIR=$test_dir PATH="$test_dir/bin:$PATH"

# Exercise the actual installer and extraction with three separated arguments.
bash "$repo_root/scripts/get-tachyon.sh" --version v1.0.0 --dir "$test_dir/install one" --variant ai > "$test_dir/output"
test -x "$test_dir/install one/core-host"
test -x "$test_dir/install one/tachyon-mcp"
expected_url='https://github.com/astorise/tachyon-mesh/releases/download/v1.0.0/tachyon-mesh-v1.0.0-linux-x86_64-ai.tar.gz'
[[ $(head -n 1 "$test_dir/urls") == "$expected_url" ]]

# Mixed forms and repeated options: the last occurrence must win.
: > "$test_dir/urls"
bash "$repo_root/scripts/get-tachyon.sh" --variant=fips --version=v2.3.4 --dir "$test_dir/install two" --variant http3 > "$test_dir/output"
test -x "$test_dir/install two/core-host"
[[ $(head -n 1 "$test_dir/urls") == *'/v2.3.4/tachyon-mesh-v2.3.4-linux-x86_64-http3.tar.gz' ]]

for args in '--version' '--dir=' '--variant --version' '--unknown value'; do
  : > "$test_dir/urls"
  # Intentional splitting: each case contains literal, space-delimited options.
  if bash "$repo_root/scripts/get-tachyon.sh" $args > "$test_dir/output" 2>&1; then
    echo "Installer unexpectedly accepted: $args" >&2
    exit 1
  fi
  [[ ! -s "$test_dir/urls" ]] || { echo "Invalid options reached network: $args" >&2; exit 1; }
done
echo 'Installer argument tests passed.'
