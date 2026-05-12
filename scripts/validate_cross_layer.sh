#!/usr/bin/env bash
set -euo pipefail

client_file="tachyon-client/src/lib.rs"
router_file="core-host/src/host_core/app_runtime.rs"
ui_cargo="tachyon-ui/Cargo.toml"
ui_main="tachyon-ui/src/main.rs"

failures=0

while IFS= read -r entry; do
    constant="${entry%% *}"
    path="${entry#* }"
    if [[ -z "${constant}" || -z "${path}" || "${constant}" == "${path}" ]]; then
        continue
    fi

    if grep -Fq "\"${path}\"" "${router_file}" || grep -Fq "\"${path}/" "${router_file}"; then
        continue
    fi

    echo "missing core-host route for ${constant}: ${path}" >&2
    failures=$((failures + 1))
done < <(sed -n 's/.*const \(ADMIN_[A-Z0-9_]*_PATH\):.*= "\([^"]*\)".*/\1 \2/p' "${client_file}")

if grep -q 'tauri-plugin-stronghold' "${ui_cargo}" && ! grep -q 'Stronghold::' "${ui_main}"; then
    echo "tauri-plugin-stronghold is declared, but no Stronghold:: API usage was found in ${ui_main}" >&2
    failures=$((failures + 1))
fi

if [[ "${failures}" -ne 0 ]]; then
    echo "${failures} cross-layer validation failure(s) found." >&2
    exit 1
fi

echo "cross-layer validation passed"
