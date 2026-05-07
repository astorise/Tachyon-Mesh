# Proposal: Compute & Observability Dashboards

## 1. Context
Managing unified workloads (Wasm, SmolVM, Legacy) and gathering telemetry via OTLP are core features of Tachyon. We need UI panels to configure these domains (4 and 12).

## 2. Solution
Implement `<tachyon-workloads-panel>` to select execution engines and bind secrets, and `<tachyon-observability-panel>` to configure OTLP export endpoints.