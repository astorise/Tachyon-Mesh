#!/usr/bin/env bash
# Builds system FaaS WASM modules that are gated behind core-host cargo features,
# then copies the resulting .wasm files to a staging directory so the Dockerfile
# runtime stage can include them with a single COPY instruction.
#
# Usage (called from the Dockerfile rust-builder / wasm-builder stage):
#   CARGO_FEATURES="ai-inference,s3-persistence" bash scripts/build-feature-system-faas.sh <staging-dir>
set -euo pipefail

STAGING_DIR="${1:-/workspace/staging-modules}"
FEATURES="${CARGO_FEATURES:-}"
TARGET=wasm32-wasip2
RELEASE_DIR="target/${TARGET}/release"

mkdir -p "${STAGING_DIR}"

has_feature() {
    printf '%s' "${FEATURES}" | grep -qE "(^|,)${1}(,|$)"
}

build_and_stage() {
    local pkg="$1"
    local wasm="$2"
    cargo build -p "${pkg}" --target "${TARGET}" --release
    cp "${RELEASE_DIR}/${wasm}" "${STAGING_DIR}/"
}

# ai-inference ─────────────────────────────────────────────────────────────────
# The OpenAI adapter and model registry are now the `guest-openai` user FaaS
# example (built via build-guest-artifacts.sh); only the broker and vector-search
# remain feature-gated system components here.
if has_feature "ai-inference"; then
    build_and_stage system-faas-vector-search  system_faas_vector_search.wasm
fi

# s3-persistence ───────────────────────────────────────────────────────────────
if has_feature "s3-persistence"; then
    build_and_stage system-faas-s3-proxy       system_faas_s3_proxy.wasm
    build_and_stage system-faas-storage-broker system_faas_storage_broker.wasm
fi

# websockets ───────────────────────────────────────────────────────────────────
if has_feature "websockets"; then
    build_and_stage system-faas-media-server system_faas_media_server.wasm
fi

# experimental ─────────────────────────────────────────────────────────────────
# system-faas-tde is crate-type rlib (not a deployable WASM module).
if has_feature "experimental"; then
    build_and_stage system-faas-tee-runtime tee_runtime.wasm
fi
