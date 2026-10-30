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

# TACH-26 (2026-10-29 audit): sdk/rust/wit/tachyon.wit is a real, tracked
# vendored copy of sdk/wit/tachyon.wit -- cargo package/publish sandbox the
# packaged crate to its own directory tree, so sdk/rust's own
# wit_bindgen::generate! can no longer reference ../wit (sdk/wit) directly.
# Unlike the curated-subset relationship above, this one must be an exact,
# byte-for-byte copy: diverging would mean the published Rust SDK generates
# different bindings than sdk/wit/tachyon.wit actually declares.
rust_vendored="sdk/rust/wit/tachyon.wit"
if [ ! -f "$rust_vendored" ]; then
  echo "::error file=$rust_vendored::missing -- sdk/rust's wit_bindgen::generate! needs its own vendored copy of $sdk (TACH-26)" >&2
  exit 1
fi
if ! diff -q "$sdk" "$rust_vendored" >/dev/null; then
  echo "::error file=$rust_vendored::has drifted from $sdk -- copy $sdk over $rust_vendored (TACH-26)" >&2
  diff "$sdk" "$rust_vendored" || true
  exit 1
fi

echo "sdk/rust/wit/tachyon.wit matches sdk/wit/tachyon.wit exactly."

# TACH-41 (2026-10-30 audit): sdk/go/wit/tachyon.wit is the same kind of
# real, tracked vendored copy as sdk/rust/wit/tachyon.wit above --
# wit-bindgen-go generate (sdk/go/generate.go) needs a local WIT file to
# read, and a copy that silently drifted from sdk/wit/tachyon.wit would mean
# the published Go SDK's bindings no longer match the interface sdk/wit's
# own curated world actually declares.
go_vendored="sdk/go/wit/tachyon.wit"
if [ ! -f "$go_vendored" ]; then
  echo "::error file=$go_vendored::missing -- sdk/go's wit-bindgen-go generate needs its own vendored copy of $sdk (TACH-41)" >&2
  exit 1
fi
if ! diff -q "$sdk" "$go_vendored" >/dev/null; then
  echo "::error file=$go_vendored::has drifted from $sdk -- copy $sdk over $go_vendored (TACH-41)" >&2
  diff "$sdk" "$go_vendored" || true
  exit 1
fi

echo "sdk/go/wit/tachyon.wit matches sdk/wit/tachyon.wit exactly."
