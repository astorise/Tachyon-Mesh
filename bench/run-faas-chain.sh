#!/usr/bin/env bash
set -euo pipefail

RESULT_DIR="${RESULT_DIR:-bench/results/raw}"
DURATION="${DURATION:-30s}"
CONNECTIONS="${CONNECTIONS:-16}"
QPS="${QPS:-200}"
CHAIN_URL="${CHAIN_URL:-http://127.0.0.1:8080/api/guest-chain-a}"
CHAIN_LABEL="${CHAIN_LABEL:-in-process}"

if ! command -v fortio >/dev/null 2>&1; then
  echo "fortio is required" >&2
  exit 127
fi

mkdir -p "$RESULT_DIR"
fortio load \
  -json "$RESULT_DIR/faas-chain-${CHAIN_LABEL}.json" \
  -qps "$QPS" \
  -c "$CONNECTIONS" \
  -t "$DURATION" \
  "$CHAIN_URL"
