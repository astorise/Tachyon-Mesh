#!/usr/bin/env bash
# sdk/wit/tachyon.wit is a deliberately curated subset of wit/tachyon.wit --
# it intentionally omits interfaces like custom-metrics, artifact-events,
# graph, and response-body, and composes its own worlds. That curation is
# legitimate and this script does not flag it.
#
# What is not legitimate is a *shared* interface silently drifting between
# the two copies. A Tachyon integration audit (2026-09-18, TACH-03) found
# exactly that: sdk/wit/tachyon.wit's telemetry-reader and training
# interfaces still used `hot-models`/`base-model` after the canonical
# wit/tachyon.wit had moved to `hot-inference-components`/
# `base-component-ref`, because nothing checked the two stayed in sync.
#
# This script diffs every interface name present in both files, body for
# body, and fails on any difference.
set -euo pipefail

canonical="wit/tachyon.wit"
sdk="sdk/wit/tachyon.wit"

extract_interface() {
  local file="$1" name="$2"
  # Comments are free to differ between the canonical file and the SDK's
  # curated copy (each documents its own audience); only structure -- record
  # fields, function signatures, types -- has to match, so comment-only and
  # blank lines are stripped before comparing.
  awk -v target="interface $name {" '
    $0 == target { found=1 }
    found { print }
    found && /^}/ { exit }
  ' "$file" | grep -Ev '^\s*(///?.*)?$'
}

# Every interface name declared in both files today. If a future change adds
# a new interface to both files, add its name here too -- an interface this
# script does not know about is not checked.
shared_interfaces=(
  handler udp-handler websocket telemetry-reader scaling-metrics vector
  training storage-broker outbound-http outbox-store bridge-controller
  routing-control secrets-vault kv-partition
)

failures=0
for name in "${shared_interfaces[@]}"; do
  canonical_body="$(extract_interface "$canonical" "$name")"
  sdk_body="$(extract_interface "$sdk" "$name")"
  if [ -z "$canonical_body" ]; then
    echo "::error file=$canonical::interface '$name' not found; update shared_interfaces in $0"
    failures=$((failures + 1))
    continue
  fi
  if [ -z "$sdk_body" ]; then
    echo "::error file=$sdk::interface '$name' not found; update shared_interfaces in $0"
    failures=$((failures + 1))
    continue
  fi
  if [ "$canonical_body" != "$sdk_body" ]; then
    echo "::error file=$sdk::interface '$name' has drifted from $canonical"
    diff <(echo "$canonical_body") <(echo "$sdk_body") || true
    failures=$((failures + 1))
  fi
done

if [ "$failures" -ne 0 ]; then
  echo "sdk/wit/tachyon.wit drifted from wit/tachyon.wit in $failures shared interface(s)."
  exit 1
fi

echo "sdk/wit/tachyon.wit matches wit/tachyon.wit for every shared interface."
