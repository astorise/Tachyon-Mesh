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
  'qwen|qwen_coder|mistral|llama|gemma|gguf|safetensors|onnx|tokenizer\.json|config\.json|ProductionQwenLoadedModel|HuggingFace(Ingestor|Tokenizer)|ModelRuntime|LoadedModel|IntegrityModelBinding|ModelDevice|LoadedAcceleratorModel|UpstreamAdmission|UPSTREAM_SCHEME|upstream_openai|AI_MODELS_REGISTRY_TABLE|ai-models-registry|hot_models|hot_model_aliases|load_accelerator_model|compute_accelerator_prompt|stream_accelerator_prompt|resolve_accelerator_model' \
  'core-host AI inference production code must stay Component-centric and model-family, tokenizer, provider, and upstream-protocol agnostic'

check_production_prefix_absent \
  core-host/src/ai_inference.rs \
  'GenerationError|ComponentGeneration|StreamEvent::Refusal|StreamEvent::ToolCall|struct ToolCall\b|\bfinish_reason\s*:|\bprompt_tokens\s*:\s*u32|\bcompletion_tokens\s*:\s*u32' \
  'core-host AI inference production code must not reintroduce typed chat-completion generation semantics (token usage, tool calls, finish reasons, refusals) as core concepts'

check_absent \
  core-host/src/host_core/component_hosts.rs \
  'model_events|ModelUploaded|publish_model_uploaded|hot_models|hot_model_aliases|AI_MODELS_REGISTRY_TABLE|ai-models-registry|load_model|compute_detailed|compute_stream' \
  'component host bindings must expose Component/artifact contracts, not legacy model host APIs'

check_absent \
  core-host/src/host_core/component_hosts.rs \
  'GenerationError|ComponentGeneration|ai_inference::ToolCall\b|StreamPayload::Refusal|StreamPayload::ToolCall|tachyon\.refusal|tachyon\.tool_call' \
  'component host bindings must not reintroduce typed chat-completion generation semantics or dialect-specific metadata tags'

check_absent \
  core-host/src/system_storage.rs \
  'model_events|ModelUploaded|publish_model_uploaded|AI_MODELS_REGISTRY_TABLE|ai-models-registry|modelPath|model_path' \
  'system storage must publish artifact/component events and registry rows'

check_production_prefix_absent \
  core-host/src/system_storage.rs \
  '\btool_call_parser\s*:\s*Option<String>|const OPENAI_SCHEME' \
  'system storage must not reintroduce a named tool-call-dialect struct field or a hardcoded upstream URI scheme constant'

check_absent \
  core-host/src/host_core/admin_plane.rs \
  '/admin/models|/admin/kv-cache|/admin/lora|Lora|LoRA|model_ref' \
  'admin-plane core routes must use Artifact/Component/training contracts'

check_absent \
  core-host/src/host_core/kv_cache.rs \
  '/api/kv-cache|/admin/kv-cache|model_ref|modelRef|model_is_hot|kv_cache_evict_model|model `' \
  'component cache must not expose model-keyed core semantics'

check_absent \
  core-host/src/host_core/app_runtime.rs \
  'Lora|LoRA|lora_|sanitize_lora|adapter_path|adapter_dir|tachyon\.mock-lora|\.safetensors' \
  'core training queue must not expose LoRA-specific execution semantics'

check_absent \
  wit/tachyon.wit \
  'model-events|model-uploaded|model-file|model-path|publish-model|base-model|hot-models' \
  'central Tachyon WIT must not expose Component-native upload or training contracts'

check_absent \
  wit/accelerator/accelerator-cpu.wit \
  'load-model|model-id|compute-detailed|compute-stream|generation-request|generation-error|token-usage|tool-call' \
  'accelerator CPU WIT must expose opaque Component invocation bytes'

check_absent \
  wit/accelerator/accelerator-gpu.wit \
  'load-model|model-id|compute-detailed|generation-request|generation-error|token-usage|tool-call' \
  'accelerator GPU WIT must expose opaque Component invocation bytes'

check_absent \
  wit/accelerator/accelerator-npu.wit \
  'load-model|model-id|compute-detailed|generation-request|generation-error|token-usage|tool-call' \
  'accelerator NPU WIT must expose opaque Component invocation bytes'

check_absent \
  wit/accelerator/accelerator-tpu.wit \
  'load-model|model-id|compute-detailed|generation-request|generation-error|token-usage|tool-call' \
  'accelerator TPU WIT must expose opaque Component invocation bytes'

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

check_production_prefix_absent \
  vendor/Magnetar/inference-components/src/lib.rs \
  'DEFAULT_COMPONENT|register_default_component|include_bytes!' \
  'generic Magnetar inference Component adapter must not select a compiled-in default Component'

if [ "$failures" -ne 0 ]; then
  echo "AI Component boundary validation failed with $failures finding(s)."
  exit 1
fi

echo "AI Component boundary validation passed."
