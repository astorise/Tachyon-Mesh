#!/usr/bin/env bash
set -euo pipefail

failures=0

check_absent() {
  local file="$1"
  local pattern="$2"
  local message="$3"
  if grep -En "$pattern" "$file" >/tmp/tachyon-ai-boundary-match.txt; then
    echo "::error file=$file::$message"
    cat /tmp/tachyon-ai-boundary-match.txt
    failures=$((failures + 1))
  fi
}

check_production_prefix_absent() {
  local file="$1"
  local pattern="$2"
  local message="$3"
  local tmp
  tmp="$(mktemp)"
  awk '
    /^[[:space:]]*#\[cfg\(test\)\]/ { stop = 1 }
    stop != 1 { print }
  ' "$file" > "$tmp"
  if grep -En "$pattern" "$tmp" >/tmp/tachyon-ai-boundary-match.txt; then
    echo "::error file=$file::$message"
    cat /tmp/tachyon-ai-boundary-match.txt
    failures=$((failures + 1))
  fi
  rm -f "$tmp"
}

check_production_prefix_absent \
  core-host/src/ai_inference.rs \
  'qwen|qwen_coder|mistral|llama|gemma|gguf|safetensors|onnx|tokenizer\.json|config\.json|ProductionQwenLoadedModel|HuggingFace(Ingestor|Tokenizer)' \
  'core-host AI inference production code must stay model-family, tokenizer, and model-format agnostic'

check_absent \
  core-host/src/ai_inference/magnetar_runtime.rs \
  'qwen|qwen_coder|mistral|llama|gemma|gguf|safetensors|onnx|tokenizer\.json|config\.json|ProductionQwenLoadedModel|HuggingFace(Ingestor|Tokenizer)|ModelTrustStore|TACHYON_MODEL_TRUST_STORE|tachyon-model' \
  'Tachyon Magnetar adapter must expose only Component/artifact contracts'

check_absent \
  core-host/src/system_storage.rs \
  'declared_model_format|TACHYON_MODEL_TRUST_STORE|\.tachyon-model\.json' \
  'system storage must not reintroduce model-format sidecar parsing'

check_absent \
  core-host/src/host_core/domain_types.rs \
  'paged_attention|cuda_graph_decode|flashinfer_attention|prefill_chunk_tokens|speculative_draft|stage_layer_ranges|expert_device_map|pipeline_depth' \
  'core domain types must not carry model-execution engine knobs'

check_absent \
  vendor/Magnetar/inference-components/src/lib.rs \
  'DEFAULT_COMPONENT|register_default_component|include_bytes!' \
  'generic Magnetar inference Component adapter must not select a compiled-in default Component'

if [ "$failures" -ne 0 ]; then
  echo "AI Component boundary validation failed with $failures finding(s)."
  exit 1
fi

echo "AI Component boundary validation passed."
