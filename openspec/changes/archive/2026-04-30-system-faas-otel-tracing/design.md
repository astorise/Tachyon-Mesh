# Design: System FaaS OTLP Adapter

`core-host` remains the low-overhead producer of sampled request telemetry. The host emits JSON lines through the existing async telemetry worker and keeps OpenTelemetry SDK/exporter dependencies out of `core-host/Cargo.toml`.

`system-faas-otel` is a sealed system route that accepts POST batches in the same JSON or NDJSON shape emitted by the host. The component normalizes those records into span-like NDJSON. If the request includes `x-tachyon-otlp-endpoint`, the component forwards the normalized batch through the existing `outbound-http` WIT import. Without that header, it writes the normalized batch to `/app/data/otel-spans.ndjson`, allowing air-gapped deployments to collect traces from a sealed volume.

The adapter intentionally uses the existing `system-faas-guest` world and `outbound-http` import so this change does not require a new host linker surface.
