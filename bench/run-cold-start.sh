#!/usr/bin/env bash
set -euo pipefail

RESULT_DIR="${RESULT_DIR:-bench/results/raw}"
COLD_START_URL="${COLD_START_URL:-http://127.0.0.1:8080/api/guest-example}"
COLD_START_LABEL="${COLD_START_LABEL:-with-instance-pre}"

# Single-shot latency sample: the caller (bench-regression.yml) is
# responsible for restarting the tachyon-host pod and waiting for readiness
# immediately before each invocation, so this always measures a genuine first
# invocation after a cache-clearing restart. Appends one JSON line per call so
# a CI loop can build up several samples for one variant's output file;
# remove the file first to start a fresh series.
mkdir -p "$RESULT_DIR"
output="$RESULT_DIR/cold-start-${COLD_START_LABEL}.jsonl"

latency_s=$(curl -s -o /dev/null -w '%{time_total}' "$COLD_START_URL")
printf '{"latency_s":%s}\n' "$latency_s" >>"$output"
