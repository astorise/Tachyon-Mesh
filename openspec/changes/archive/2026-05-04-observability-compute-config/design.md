# Design: Observability & Compute Data Model

## 1. The GitOps YAML Specification
This file controls the telemetry output and resource bounding for the execution engine.

    api_version: compute.tachyon.io/v1alpha1
    kind: ObservabilityAndCompute
    metadata:
      name: edge-global-ops
      environment: production
      
    spec:
      # 1. OBSERVABILITY (Logs & Traces)
      telemetry:
        logs:
          global_level: INFO
          component_overrides:
            "system-faas-authz": DEBUG # Hot-reloadable debug for troubleshooting
            "guest-legacy-proxy": WARN
        traces:
          otlp_endpoint: "http://otel-collector.infra.local:4317"
          sampling_rate_percentage: 5 # Only trace 5% of requests to save bandwidth
          propagate_w3c_context: true

      # 2. COMPUTE QUOTAS (MicroVM & Memory Governor)
      compute_quotas:
        - target_group_ref: "ai-inference-wasm" # Links to Pillar 1 TargetGroup
          limits:
            max_memory_mb: 2048
            cpu_shares: 1024
          scaling:
            min_instances: 1
            max_instances: 5
            scale_to_zero_after_seconds: 300
            
        - target_group_ref: "tcp-echo-guest"
          limits:
            max_memory_mb: 16 # Lightweight networking component
            cpu_shares: 128

## 2. The WIT Contract (`wit/config-observability.wit`)
This interface allows `system-faas-config-api` to validate and propagate telemetry and resource boundaries safely.

    interface config-observability {
        enum log-level { trace, debug, info, warn, error, fatal }

        record log-policy {
            global-level: log-level,
            component-overrides: list<tuple<string, log-level>>,
        }

        record trace-policy {
            otlp-endpoint: option<string>,
            sampling-rate-percentage: u8,
            propagate-w3c-context: bool,
        }

        record telemetry-config {
            logs: log-policy,
            traces: trace-policy,
        }

        record resource-limits {
            max-memory-mb: u32,
            cpu-shares: u32,
        }

        record scaling-policy {
            min-instances: u32,
            max-instances: u32,
            scale-to-zero-after-seconds: option<u32>,
        }

        record compute-quota {
            target-group-ref: string,
            limits: resource-limits,
            scaling: option<scaling-policy>,
        }

        record ops-configuration {
            telemetry: telemetry-config,
            quotas: list<compute-quota>,
        }

        /// Global validation
        validate-ops-config: func(config: ops-configuration) -> result<_, string>;

        /// CRUD Operations for Tachyon-UI / MCP
        get-ops-config: func() -> result<ops-configuration, string>;
        apply-compute-quota: func(quota: compute-quota) -> result<_, string>;
        update-telemetry: func(config: telemetry-config) -> result<_, string>;
    }