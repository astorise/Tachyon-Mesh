# Specifications: Zero-Overhead Resiliency Architecture

## 1. Feature Flag Definition (`core-host/Cargo.toml`)
The resiliency engine MUST be completely disabled by default.

    [features]
    default = []
    resiliency = ["tower"]

    [dependencies]
    tower = { version = "0.4", features = ["retry", "timeout", "limit"], optional = true }

## 2. Schema Update v8 (`integrity.lock`)
Add an optional resiliency object to the Target definition. The CLI will always generate this if requested, but the Host will only act on it if compiled with the feature.

    {
        "targets": [
            {
                "name": "payment-api",
                "module": "payment.wasm",
                "resiliency": {
                    "timeout_ms": 2000,
                    "retries": {
                        "max_attempts": 3,
                        "retry_on": [503, 504]
                    }
                }
            }
        ]
    }

## 3. Conditional Axum Routing (`core-host`)
The `faas_handler` or the Axum Router construction MUST use conditional compilation to inject the layers.

    #[cfg(feature = "resiliency")]
    fn apply_resiliency_layers(router: Router, config: &TargetConfig) -> Router {
        // Extract policies from config.resiliency
        // Apply tower::timeout::TimeoutLayer
        // Apply tower::retry::RetryLayer
        router.layer(...)
    }

    #[cfg(not(feature = "resiliency"))]
    fn apply_resiliency_layers(router: Router, _config: &TargetConfig) -> Router {
        // Return the raw router with ZERO overhead
        router
    }

## 4. Timeout and Retry Mechanics (When Enabled)
- **Timeout:** Enforced at the Tower level. If the Wasmtime execution Future exceeds `timeout_ms`, Tokio drops the Future. The Wasmtime engine reclaims the memory, and Axum returns `504 Gateway Timeout`.
- **Retry:** A custom `tower::retry::Policy` inspects the HTTP Response. If the status code matches the `retry_on` array, it returns `true` and the request is cloned and re-executed up to `max_attempts`.