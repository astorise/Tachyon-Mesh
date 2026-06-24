## 1. Reference Contract and Dependency Spike

- [x] 1.1 Inventory reusable Qwen 3.5, hybrid-attention, sparse-MoE, FP8, and cache primitives in Candle 0.10 and the `astorise/candle` fork
- [x] 1.2 Select a trusted Qwen 3.5 reference implementation and document the exact linear-attention, rotary, routing, and shared-expert equations
- [x] 1.3 Add a reproducible exporter for small golden configuration, tensor, intermediate-state, and logits fixtures without downloading production model artifacts in CI
- [x] 1.4 Record the accepted ModelOpt producer version and mixed-precision metadata contract for the first compatibility profile

## 2. Architecture Detection and Validation

- [x] 2.1 Add an architecture descriptor and registry for text-generation backends
- [x] 2.2 Implement normalized recognition of `Qwen3_5MoeForConditionalGeneration`, `qwen3_5_moe`, and `qwen3_5_moe_text` metadata
- [x] 2.3 Validate the hybrid layer schedule, dimensions, rotary settings, expert counts, top-k routing, shared-expert settings, and unsupported MTP or vision requirements
- [x] 2.4 Validate required tensor names, shapes, shard-index entries, and per-operator quantization assignments with actionable errors
- [x] 2.5 Add fail-closed compatibility-profile versioning for sibling checkpoints with compatible semantics but different layer or expert counts

## 3. Hybrid Decoder Runtime

- [x] 3.1 Implement token embedding, RMS normalization, residual flow, final normalization, and LM-head projection for the Qwen 3.5 text tower
- [x] 3.2 Implement full-attention layers with grouped KV heads, causal masking, partial rotary dimensions, and autoregressive KV-cache updates
- [x] 3.3 Implement linear-attention layers with the required gated projections, convolutional state, recurrent state, and token-by-token updates
- [x] 3.4 Dispatch each decoder layer according to the declared ordered hybrid layer schedule
- [x] 3.5 Add deterministic single-token and multi-token golden parity tests for full-attention, linear-attention, and complete decoder layers

## 4. Sparse MoE Execution

- [x] 4.1 Implement router logits, deterministic top-k expert selection, routing-weight normalization, and output aggregation
- [x] 4.2 Implement routed expert gate, up, activation, and down projections using configurable expert counts
- [x] 4.3 Implement shared-expert execution and aggregation with routed expert outputs
- [x] 4.4 Resolve selected expert tensors lazily across safetensors shards and report layer, expert, and component on contract failures
- [x] 4.5 Add golden routing and expert-output tests, including ties, multiple tokens, and missing tensor components

## 5. Mixed FP8 and NVFP4 Operators

- [x] 5.1 Introduce a quantized-linear operator contract that dispatches dense, FP8, and W4A16 NVFP4 storage from validated metadata
- [x] 5.2 Implement the FP8 projection path with block scales, input scales, dtype validation, and deterministic fallback tests
- [x] 5.3 Integrate packed W4A16 NVFP4 expert, shared-expert, and LM-head projections with tensor and block scales
- [x] 5.4 Bound fallback dequantization to the active operator or layer window and reject execution before configured memory limits are exceeded
- [x] 5.5 Gate native FP8 and NVFP4 CUDA/CUTLASS paths on compiled kernels, runtime support, GPU capability, alignment, and shape constraints
- [x] 5.6 Add mixed dense/FP8/NVFP4 forward-graph parity tests and unsupported quantization diagnostics

## 6. Decode State and Memory Profiles

- [x] 6.1 Extend decode state to track full-attention KV caches separately from linear-attention convolutional and recurrent state
- [x] 6.2 Account for both state types in performance and layer-wise-streaming memory plans
- [x] 6.3 Page only the active layer, selected routed experts, shared expert, and required state in layer-wise-streaming mode
- [x] 6.4 Add bounded expert and layer working-set caches with observable hit, miss, transfer, and eviction diagnostics
- [x] 6.5 Add tests proving inactive experts remain off accelerator memory and state survives layer paging across multiple decode tokens

## 7. Generation and OpenAI Integration

- [x] 7.1 Connect the Qwen 3.5 backend to the existing tokenizer, chat template, sampling, stop-sequence, and bounded generation pipeline
- [x] 7.2 Implement incremental streaming through the existing OpenAI response path and verify streamed fragments reconstruct deterministic buffered output
- [x] 7.3 Reject image content and unsupported modalities explicitly while retaining text-only requests
- [x] 7.4 Preserve the existing ONNX, Llama, mock, and unsupported-NVFP4 backend boundaries in regression tests
- [x] 7.5 Add model-broker compatibility metadata and diagnostics that distinguish weight format support from architecture runtime support

## 8. Production Checkpoint Qualification

- [x] 8.1 Add an opt-in local probe for `models/nvidia--Qwen3.6-35B-A3B-NVFP4` that validates metadata and tensors without eager model densification
- [x] 8.2 Add an opt-in deterministic prompt test that produces non-mock text through buffered and streaming runtime paths
- [x] 8.3 Measure host memory, accelerator memory, first-token latency, decode latency, and expert paging for each supported memory profile
- [x] 8.4 Require native production capabilities when fallback execution would exceed configured memory limits and verify the resulting diagnostic
- [ ] 8.5 Bind the qualified model to the deployed OpenAI route and smoke-test `https://ai.tachyon-mesh.wsl/v1/chat/completions`

## 9. Documentation and Release Guardrails

- [x] 9.1 Document the initial Qwen 3.5 MoE compatibility profile, supported quantization assignments, hardware requirements, and known exclusions
- [x] 9.2 Document how to add another compatible ModelOpt checkpoint without treating all NVFP4 models as architecture-compatible
- [x] 9.3 Document fixture regeneration, installed-checkpoint qualification, and failure-diagnosis procedures
- [x] 9.4 Update architecture support documentation and link the focused implementation to GitHub issue #228 and multimodal issue #238
