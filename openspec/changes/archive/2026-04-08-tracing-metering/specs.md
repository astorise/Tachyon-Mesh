# Specifications: Sampled Telemetry Pipeline

## 1. Global Configuration
The `integrity.lock` defines the sampling rate for the entire node.

    {
        "host": {
            "telemetry_sample_rate": 0.001,
            "enable_fuel_profiling": true
        }
    }

## 2. The Sampling Logic
When an HTTP request arrives, the `core-host` generates a random float between 0.0 and 1.0.
- If `random <= telemetry_sample_rate`: The host enables Wasmtime fuel consumption for this specific instance, records the start timestamp, and generates a Trace ID.
- If `random > telemetry_sample_rate`: The instance runs normally with zero telemetry overhead.

## 3. The Asynchronous Queue ("La Pile")
Sampled executions generate a heavy JSON object containing the Trace and Metrics.
- The host attempts to push this object to a bounded `tokio::sync::mpsc` channel.
- If the channel is full, the host uses `try_send`. If it fails, the metric is simply dropped (Fail-safe mechanism to prevent OOM or blocking).

## 4. The System FaaS
`system-faas-metering.wasm` acts as a background consumer. It reads batches of metrics from the channel and forwards them to the final destination (e.g., a Prometheus Pushgateway, an OpenTelemetry Collector, or a local log file) using the internal Mesh network.