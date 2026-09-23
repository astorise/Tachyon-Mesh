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

# Skips each #[cfg(test)]-attributed item individually — from the attribute
# line through its own matching closing brace (or, for a brace-less item
# like `#[cfg(test)] use foo;`, through that one line) — then resumes
# scanning production code for whatever follows. A file can carry any number
# of scattered #[cfg(test)] items (a few inline test-only helpers plus a
# trailing `mod tests { ... }`), not just one contiguous block at the end;
# stopping at the *first* one and never resuming (the previous behavior)
# silently exempted everything after it, including real production code.
check_production_prefix_absent() {
  local file="$1"
  local pattern="$2"
  local message="$3"
  local tmp
  tmp="$(mktemp)"
  awk '
    BEGIN { skip = 0; depth = 0 }
    {
      line = $0
      if (skip == 1) {
        opens = gsub(/\{/, "{", line)
        closes = gsub(/\}/, "}", line)
        depth += opens - closes
        if (depth <= 0) { skip = 0 }
        next
      }
      if (line ~ /^[[:space:]]*#\[cfg\(test\)\]/) {
        skip = 1
        depth = 0
        next
      }
      print
    }
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

check_production_prefix_absent \
  core-host/src/ai_inference.rs \
  'struct ModelMeta\b|declared_tool_call_metadata|read_declared_tool_call_parser' \
  'core-host must read a Component artifact sidecar as an opaque key/value bag, not extract one named tool-call-dialect field (TACH-01)'

check_absent \
  core-host/src/host_core/component_hosts.rs \
  'model_events|ModelUploaded|publish_model_uploaded|hot_models|hot_model_aliases|AI_MODELS_REGISTRY_TABLE|ai-models-registry|load_model|compute_detailed|compute_stream' \
  'component host bindings must expose Component/artifact contracts, not legacy model host APIs'

check_absent \
  core-host/src/host_core/component_hosts.rs \
  'GenerationError|ComponentGeneration|ai_inference::ToolCall\b|StreamPayload::Refusal|StreamPayload::ToolCall|tachyon\.refusal|tachyon\.tool_call|\btool_call_parser\s*:\s*Option<String>' \
  'component host bindings must not reintroduce typed chat-completion generation semantics or a named tool-call-dialect field in the registry record (TACH-01)'

check_absent \
  core-host/src/system_storage.rs \
  'model_events|ModelUploaded|publish_model_uploaded|AI_MODELS_REGISTRY_TABLE|ai-models-registry|modelPath|model_path' \
  'system storage must publish artifact/component events and registry rows'

check_production_prefix_absent \
  core-host/src/system_storage.rs \
  '\btool_call_parser\s*:\s*Option<String>|const OPENAI_SCHEME|TOOL_CALL_PARSER_METADATA_KEY|binding_tool_call_parser|registry_component_metadata' \
  'system storage must not reintroduce a named tool-call-dialect struct field, a hardcoded upstream URI scheme constant, or a dedicated tool-call-dialect metadata key/reader (TACH-01)'

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
  sdk/wit/tachyon.wit \
  'model-events|model-uploaded|model-file|model-path|publish-model|base-model|hot-models' \
  'sdk WIT must stay aligned with the Component-centric field names in wit/tachyon.wit (TACH-03; see also scripts/check_wit_sdk_sync.sh)'

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
  core-host/src/ai_inference/magnetar_runtime.rs \
  'GenerationStreamEvent|InferenceComponentOutput|InferenceComponentUsage|GenerationUsage|text_delta|\binvoke_payload\(|\binvoke_payload_streaming\(' \
  'Tachyon Magnetar adapter must call only invoke_payload_opaque/invoke_payload_streaming_opaque (astorise/Magnetar#89) and must never touch typed Magnetar generation vocabulary again (TACH-02, fully closed)'

check_absent \
  core-host/src/ai_inference/magnetar_runtime.rs \
  'tachyon_wire_tags|"prompt_tokens"|"generated_tokens"|"completion_tokens"|"finish_reason"|tachyon\.usage\.' \
  'Tachyon Magnetar adapter must relay Magnetar opaque tags exactly as received, with no renaming, filtering, or interpretation of token-usage or finish-reason keys (TACH-01, audit round 3)'

check_production_prefix_absent \
  core-host/src/ai_inference.rs \
  'tachyon_wire_tags|MOCK_LLM_RESPONSE|"prompt_tokens"|"generated_tokens"|"completion_tokens"|"finish_reason"|tachyon\.usage\.' \
  'core-host AI inference production code must not fabricate, rename, or interpret token-usage or finish-reason metadata, and the mock Component must not use LLM-flavored naming for its canned response (TACH-02, audit round 3)'

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
