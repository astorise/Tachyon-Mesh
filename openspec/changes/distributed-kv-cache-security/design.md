# Design: Distributed KV Cache Data Model

## 1. The GitOps YAML Specification
This configuration declares how the AI inference KV cache is stored, distributed, and secured.

    api_version: cache.tachyon.io/v1alpha1
    kind: DistributedCache
    metadata:
      name: global-llm-kv-cache
      environment: production
      
    spec:
      # 1. KV CACHE TOPOLOGY
      topology:
        target_deployment_ref: "llama-3-core" # Links to Domain 10
        distribution:
          mode: distributed_gossip # Shares cache state with neighboring Edge nodes
          sync_interval_ms: 100
        eviction:
          policy: lru # Least Recently Used
          max_total_memory_mb: 4096
          max_ttl_seconds: 3600 # Clear context after 1 hour of inactivity

      # 2. ISOLATION & SECURITY
      security:
        tenant_isolation: true # Strict separation based on Domain 2 extracted Identity
        encryption:
          mode: transparent_data_encryption (tde)
          key_rotation_hours: 24
          hardware_backend: auto_detect # Uses TEE/TPM if available to secure keys

## 2. The WIT Contract (`wit/config-cache.wit`)
The strict Wasm interface used by `system-faas-config-api`.

    interface config-cache {
        enum distribution-mode { local-only, distributed-gossip }
        enum eviction-policy { lru, lfu, fifo }
        enum encryption-mode { none, transparent-data-encryption }

        record cache-topology {
            target-deployment-ref: string,
            dist-mode: distribution-mode,
            sync-interval-ms: u32,
            eviction-mode: eviction-policy,
            max-memory-mb: u32,
            max-ttl-seconds: u32,
        }

        record cache-security {
            tenant-isolation: bool,
            enc-mode: encryption-mode,
            key-rotation-hours: u32,
            hardware-backend: string,
        }

        record distributed-cache-config {
            name: string,
            topology: cache-topology,
            security: cache-security,
        }

        record cache-configuration {
            caches: list<distributed-cache-config>,
        }

        /// Global validation
        validate-cache-config: func(config: cache-configuration) -> result<_, string>;

        /// CRUD for Tachyon-UI
        get-cache-config: func() -> result<cache-configuration, string>;
        apply-cache-config: func(config: distributed-cache-config) -> result<_, string>;
    }