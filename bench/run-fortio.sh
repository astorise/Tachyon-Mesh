#!/usr/bin/env bash
set -euo pipefail

RESULT_DIR="${RESULT_DIR:-bench/results/raw}"
DURATION="${DURATION:-60s}"
CONNECTIONS="${CONNECTIONS:-64}"
RATES="${RATES:-1000 10000}"
TARGETS="${TARGETS:-tachyon:http://echo.tachyon-bench.svc.cluster.local istio:http://echo.istio-bench.svc.cluster.local linkerd:http://echo.linkerd-bench.svc.cluster.local}"

if ! command -v fortio >/dev/null 2>&1; then
  echo "fortio is required" >&2
  exit 127
fi

mkdir -p "$RESULT_DIR"

for target in $TARGETS; do
  name="${target%%:*}"
  url="${target#*:}"
  for qps in $RATES; do
    output="$RESULT_DIR/${name}-${qps}qps.json"
    fortio load \
      -json "$output" \
      -qps "$qps" \
      -c "$CONNECTIONS" \
      -t "$DURATION" \
      "$url"
  done
done
