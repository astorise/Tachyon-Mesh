# Proposal: Decoupled OTLP Tracing

## Why
`core-host` already keeps tracing lightweight by emitting sampled request telemetry as JSON lines and avoiding direct `opentelemetry` dependencies. Operators still need an optional OTLP bridge that can be sealed as a System FaaS instead of forcing every deployment to carry exporter code in the host binary.

## What Changes
- Add `system-faas-otel`, a System FaaS component that accepts the existing telemetry JSON batch format.
- Normalize sampled Tachyon request records into span-like NDJSON suitable for an OTLP bridge or downstream collector adapter.
- Forward batches to a sealed OTLP endpoint through the existing `outbound-http` host import when configured, otherwise append them to a sealed volume for air-gapped collection.
- Document the OpenSpec requirements for keeping `core-host` free of direct OpenTelemetry exporter dependencies.
