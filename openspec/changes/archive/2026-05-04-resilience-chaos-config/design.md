# Design: Resilience & Chaos Data Model

## 1. The GitOps YAML Specification
This file defines reliability policies and attaches them to existing Routes (from Pillar 1).

    api_version: resilience.tachyon.io/v1alpha1
    kind: ResilienceConfiguration
    metadata:
      name: edge-global-resilience
      environment: production
      
    spec:
      policies:
        # 1. Standard Reliability (Retries & Timeouts)
        - name: standard-api-resilience
          target_route_refs: ["api-v1-routing"]
          timeouts:
            request_timeout_ms: 5000
            idle_timeout_ms: 30000
          retries:
            max_attempts: 3
            per_try_timeout_ms: 2000
            retry_on: ["5xx", "gateway-error", "connect-failure"]

        # 2. Shadow Traffic (Mirroring traffic to a V2 without impacting V1)
        - name: shadow-ai-v2
          target_route_refs: ["api-v1-routing"]
          shadow_traffic:
            target_group_ref: "ai-inference-wasm-v2"
            percentage: 15 # 15% of traffic is mirrored asynchronously

        # 3. Chaos Engineering (Injecting faults to test client resilience)
        - name: chaos-db-latency
          target_route_refs: ["legacy-db-proxy"]
          chaos_injection:
            percentage: 5 # Affects 5% of traffic
            fault:
              type: LATENCY
              delay_ms: 2500

## 2. The WIT Contract (`wit/config-resilience.wit`)
This interface is used by the `system-faas-config-api` to safely parse and apply these intents via the event bus.

    interface config-resilience {
        record timeout-policy {
            request-timeout-ms: u32,
            idle-timeout-ms: u32,
        }

        record retry-policy {
            max-attempts: u8,
            per-try-timeout-ms: u32,
            retry-on: list<string>,
        }

        record shadow-policy {
            target-group-ref: string,
            percentage: u8, // 0-100
        }

        enum fault-type { latency, abort }

        record chaos-fault {
            fault-type: fault-type,
            delay-ms: option<u32>,
            abort-status: option<u16>,
        }

        record chaos-policy {
            percentage: u8,
            fault: chaos-fault,
        }

        record resilience-policy {
            name: string,
            target-route-refs: list<string>,
            timeouts: option<timeout-policy>,
            retries: option<retry-policy>,
            shadow: option<shadow-policy>,
            chaos: option<chaos-policy>,
        }

        record resilience-configuration {
            policies: list<resilience-policy>,
        }

        /// Global validation (Zero-Panic)
        validate-resilience-config: func(config: resilience-configuration) -> result<_, string>;

        /// CRUD Operations exposed for Tachyon-UI / MCP
        get-resilience-config: func() -> result<resilience-configuration, string>;
        apply-resilience-policy: func(policy: resilience-policy) -> result<_, string>;
        delete-resilience-policy: func(name: string) -> result<_, string>;
    }