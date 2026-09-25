## Why

The archived Candle-to-Magnetar cutover claimed completion while Tachyon still
used a local facade. The first corrective PR then proved real Magnetar
production execution, but it still placed model-family and loader/provider
knowledge directly in `core-host`. The final audit requires the architectural
boundary to be Component-centric: Tachyon transports, trusts, routes, and
invokes opaque inference Components; Magnetar and the Component own model
semantics.

## What Changes

- Replace Tachyon's Component-specific Magnetar adapter boundary with a generic
  Magnetar inference Component adapter pinned through the vendored Magnetar
  submodule.
- Resolve the inference Component as an explicit `*.component.wasm` artifact
  from the Tachyon-staged binding root and pass that artifact to Magnetar;
  the generic adapter no longer selects a compiled-in default Component.
- Remove direct `core-host` dependencies on concrete Magnetar loader and
  Provider implementation crates.
- Keep Component provenance, host-controlled trust, placement constraints, QoS,
  admission, deadline policy, and streaming transport in Tachyon.
- Keep model format parsing, tokenizer/chat template handling, model instance
  lifecycle, execution planning, Provider construction, CUDA execution, and
  generation semantics behind Magnetar.
- Reject unsupported local tool-call and structured-output controls as invalid
  Component invocations instead of rewriting them into prompts in Tachyon core.
- Reject historical local adapter bindings before runtime construction; adapter
  behavior must be implemented by the selected Component/Magnetar contract.
- Replace Candle-oriented canonical specs and GPU CI proof steps with Magnetar
  Component CPU/GPU checks, zero-test guards, and an architecture guard that
  prevents model knowledge from re-entering production `core-host` paths.

## Impact

- Affected specs: `ai-inference`, `github-actions`, `hardware-capabilities`,
  `memory-delegation`, `ai-orchestration`.
- Affected code: `core-host` local inference adapter, Magnetar vendored
  inference Component facade, AI inference tests, Cargo feature graph.
- Affected behavior: local `magnetar:` bindings are treated as inference
  Component artifacts; explicit CUDA placement remains fail-closed; unsupported
  local protocol controls are rejected rather than silently prompt-mutated.
