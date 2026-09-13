## Why

The archived Candle-to-Magnetar cutover claimed completion while Tachyon still
used a local facade. The first corrective PR then proved real Magnetar
production execution, but it still placed model-family and loader/provider
knowledge directly in `core-host`. The final audit requires the architectural
boundary to be Component-centric: Tachyon transports, trusts, routes, and
invokes opaque inference Components; Magnetar and the Component own model
semantics.

## What Changes

- Replace Tachyon's model-specific Magnetar adapter boundary with a generic
  Magnetar inference Component adapter pinned through the vendored Magnetar
  submodule.
- Remove direct `core-host` dependencies on concrete Magnetar loader and
  Provider implementation crates.
- Keep Component provenance, host-controlled trust, placement constraints, QoS,
  admission, deadline policy, and streaming transport in Tachyon.
- Keep model format parsing, tokenizer/chat template handling, model instance
  lifecycle, execution planning, Provider construction, CUDA execution, and
  generation semantics behind Magnetar.
- Reject unsupported local tool-call and structured-output controls as invalid
  Component invocations instead of rewriting them into prompts in Tachyon core.
- Replace Candle-oriented canonical specs and GPU CI proof steps with Magnetar
  Component CPU/GPU checks and zero-test guards.

## Impact

- Affected specs: `ai-inference`, `github-actions`, `hardware-capabilities`,
  `memory-delegation`, `ai-orchestration`.
- Affected code: `core-host` local inference adapter, Magnetar vendored
  inference Component facade, AI inference tests, Cargo feature graph.
- Affected behavior: local `magnetar:` bindings are treated as inference
  Component artifacts; explicit CUDA placement remains fail-closed; unsupported
  local protocol controls are rejected rather than silently prompt-mutated.
