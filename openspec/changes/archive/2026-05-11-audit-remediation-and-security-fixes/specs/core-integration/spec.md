# Specification: Core Host Phantom Endpoints Implementation

The UI and MCP tools advertise endpoints that currently return 404s because they are missing from `core-host/src/host_core/app_runtime.rs`.

## 1. Metrics Endpoint
* **Route:** `GET /admin/metrics`
* **Implementation:** Connect to the `telemetry::TelemetrySnapshot` manager. Aggregate current CPU, memory, and invocation metrics across active component hosts. Return the serialized `TelemetrySnapshot`.

## 2. Shadow Diffs Endpoint
* **Route:** `GET /admin/shadow/diffs`
* **Implementation:** Tap into the event stream from `system-faas-shadow-proxy`. Return a paginated list of recent structural or payload diffs between the primary execution and the shadow execution.

## 3. Chaos Scenarios Endpoint
* **Route:** `POST /admin/chaos/scenarios`
* **Implementation:** Hook into the chaos harness (as currently tested in `chaos_test.rs`). Accept a payload defining the failure mode (e.g., node isolation, simulated latency, memory pressure) and inject it into the active test overlay.