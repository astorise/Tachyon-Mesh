#!/usr/bin/env bash
# TACH-06 (2026-09-24 audit): the README used to hand-carry the vendored
# Magnetar commit SHA as prose, decoupled from the actual `vendor/Magnetar`
# gitlink — a round-6 audit found it had drifted stale (README claimed
# `ee7ef9e`, the real gitlink was already two commits ahead at `9db481d`).
# This guard fails CI the moment that happens again, rather than relying on
# someone noticing during a future audit.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

readonly readme_path="README.md"
readonly readme_pattern='Magnetar `([0-9a-f]{40})`'

if [ ! -d vendor/Magnetar/.git ] && [ ! -f vendor/Magnetar/.git ]; then
  echo "::error file=vendor/Magnetar::vendor/Magnetar submodule is not checked out; run 'git submodule update --init --recursive' before validating the pin doc" >&2
  exit 1
fi

actual_sha=$(git -C vendor/Magnetar rev-parse HEAD)

readme_sha=$(grep -oE "$readme_pattern" "$readme_path" | head -1 | grep -oE '[0-9a-f]{40}' || true)

if [ -z "$readme_sha" ]; then
  echo "::error file=${readme_path}::could not find a 'Magnetar \`<40-hex-char sha>\`' reference to validate against vendor/Magnetar's actual pin" >&2
  exit 1
fi

if [ "$readme_sha" != "$actual_sha" ]; then
  echo "::error file=${readme_path}::README documents Magnetar at \`${readme_sha}\`, but vendor/Magnetar is actually pinned at \`${actual_sha}\` — update the README's SHA alongside any vendor/Magnetar repin (TACH-06)" >&2
  exit 1
fi

echo "Magnetar pin documentation validation passed (${actual_sha})."
