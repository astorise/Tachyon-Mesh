# Implementation Tasks
- [x] Define the System FaaS OTLP adapter contract using the existing `system-faas-guest` WIT world.
- [x] Bootstrap `systems/system-faas-otel` in Rust, compiling as a component-friendly `cdylib`.
- [x] Normalize host telemetry JSON/NDJSON batches into span-like OTLP bridge records.
- [x] Verify `core-host/Cargo.toml` remains free of direct `opentelemetry` exporter dependencies.
