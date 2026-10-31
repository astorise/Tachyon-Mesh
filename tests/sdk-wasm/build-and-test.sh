#!/usr/bin/env bash
# Builds real Python and Go SDK consumers and runs them in core-host's router.
set -euo pipefail
repo_root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$repo_root"
for tool in componentize-py tinygo wasm-tools wit-bindgen-go go cargo; do
  command -v "$tool" >/dev/null || { echo "Missing required tool: $tool" >&2; exit 1; }
done
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export PYTHONDONTWRITEBYTECODE=1 GOWORK=off
mkdir -p "$work/python-wit" "$work/python" "$work/python-sdk" "$work/go-wit/deps/cli" "$work/go" target/wasm32-wasip2/debug

# An SDK guest needs only KV and the Tachyon handler. Generating from the full
# faas-guest world would import unrelated capabilities and fail a KV-only scope.
cp sdk/wit/tachyon.wit "$work/python-wit/tachyon.wit"
printf '\nworld audit-kv-faas {\n    import kv-partition;\n    export handler;\n}\n' >> "$work/python-wit/tachyon.wit"
componentize-py -d "$work/python-wit" -w audit-kv-faas bindings "$work/python"
cp tests/sdk-wasm/python_guest.py "$work/python/app.py"
cp -R sdk/python/tachyon_mesh_sdk "$work/python-sdk/"
componentize-py -d "$work/python-wit" -w audit-kv-faas componentize \
  -p "$work/python" -p "$work/python-sdk" app \
  -o "$repo_root/target/wasm32-wasip2/debug/sdk_python_audit.wasm"

# TinyGo's WASIp2 runtime also imports wasi:cli and its dependencies. Its
# default command world does not include Tachyon KV or the handler; compose
# all of them into the component world passed to TinyGo's embed step.
cp sdk/wit/tachyon.wit "$work/go-wit/tachyon.wit"
tinygo_root=$(tinygo env TINYGOROOT)
cp "$tinygo_root"/lib/wasi-cli/wit/*.wit "$work/go-wit/deps/cli/"
cp -R "$tinygo_root"/lib/wasi-cli/wit/deps/* "$work/go-wit/deps/"
printf '\nworld audit-kv-faas {\n    include wasi:cli/command@0.2.0;\n    import kv-partition;\n    export handler;\n}\n' >> "$work/go-wit/tachyon.wit"
wit-bindgen-go generate -w faas-guest -o "$work/go/gen" \
  -p example.com/tachyon-sdk-audit/gen sdk/go/wit
cp tests/sdk-wasm/go_guest.go "$work/go/main.go"
cp sdk/go/go.sum "$work/go/go.sum"
(
  cd "$work/go"
  go mod init example.com/tachyon-sdk-audit
  go mod edit -go=1.23.0 -require=github.com/astorise/Tachyon-Mesh/sdk/go@v0.0.0 \
    -require=go.bytecodealliance.org/cm@v0.3.0 \
    -replace="github.com/astorise/Tachyon-Mesh/sdk/go=$repo_root/sdk/go"
  GOFLAGS=-buildvcs=false tinygo build -target=wasip2 \
    -wit-package "$work/go-wit" -wit-world audit-kv-faas \
    -o "$repo_root/target/wasm32-wasip2/debug/sdk_go_audit.wasm" .
)

# These are explicitly ignored by the general suite because they require the
# generated artifacts. Fail if a renamed or missing test would execute zero tests.
run_required_test() {
  local name=$1
  local full_name="host_core::tests::http_router::${name}"
  cargo test --locked -p core-host --bin core-host "$full_name" -- --ignored --exact | tee "$work/${name}.log"
  grep -Fx "test ${full_name} ... ok" "$work/${name}.log" >/dev/null || {
    echo "Required SDK test did not execute successfully: ${full_name}" >&2
    return 1
  }
}
run_required_test python_sdk_kv_component_round_trip
run_required_test go_sdk_kv_component_round_trip
