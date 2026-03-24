# Proposal: Change 004 - FaaS-Native Observability & Proc-Macro

## Context
Standard observability SDKs (like OpenTelemetry OTLP exporters) rely on heavy asynchronous network I/O. If developers embed these SDKs inside their WASM guest functions, it will drastically increase the binary size (>10MB) and ruin the cold start latency. We need a FaaS-native approach where the guest is completely network-agnostic regarding telemetry.

## Objective
Implement a procedural macro `#[faas_handler]` that developers can apply to their guest entrypoints. This macro will automatically initialize a lightweight, JSON-based structured logger that outputs directly to `stdout` (which is already piped to the Host via WASI). The Host will intercept this stdout, parse the JSON, and integrate it into its own telemetry pipeline.

## Scope
- Create a new procedural macro crate `faas-sdk`.
- Implement `#[faas_handler]` using `syn` and `quote`.
- Update the `guest-example` to use this macro and the `tracing` crate.
- Update `core-host` to parse the JSON output from the guest's WASI `MemoryWritePipe` and forward it to the host's own `tracing` subsystem.

## Out of Scope
- Full OpenTelemetry/OTLP backend integration (for this iteration, the Host will just parse and echo the guest's structured logs to the terminal).
- Distributed Trace Context propagation (TraceId injection will be added in a future iteration).

## Success Metrics
- A developer can write a function annotated with `#[faas_handler]` and use standard `tracing::info!()`.
- The compiled guest WASM binary remains small (< 2MB).
- The Host captures the guest's execution logs and prints them to the console, clearly identifying them as originating from the FaaS guest.