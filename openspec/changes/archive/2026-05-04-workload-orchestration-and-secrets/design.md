# Design: Workload & Secrets Data Model

## 1. The GitOps YAML Specification
This configuration declares the execution environments. Secrets are NEVER stored here; only references to the TDE keystore are allowed.

    api_version: workloads.tachyon.io/v1alpha1
    kind: WorkloadConfiguration
    metadata:
      name: edge-compute-fleet
      environment: production
      
    spec:
      # 1. SECRET DECLARATIONS (Resolvers)
      secrets_providers:
        - name: "stripe-api-key"
          backend: system-faas-tde # Internal encrypted store
          key_id: "stripe/prod_key"
        - name: "db-password"
          backend: external_vault # Could be HashiCorp Vault
          key_id: "kv/data/db/edge-proxy"

      # 2. WORKLOADS (FaaS, SmolVM, Legacy)
      workloads:
        # A standard WebAssembly FaaS function
        - name: "payment-processor"
          runtime: faas_wasm
          asset_ref: "payment-wasm-v1" # Links to Domain 8 (Air-Gapped PUSH)
          env:
            LOG_LEVEL: "info"
            CURRENCY: "EUR"
          secret_mounts:
            - env_var: "STRIPE_KEY"
              secret_ref: "stripe-api-key"

        # A highly isolated MicroVM for untrusted code
        - name: "untrusted-user-plugin"
          runtime: smolvm_microvm
          asset_ref: "user-plugin-v2"
          env:
            ISOLATION_MODE: "strict"

        # A bridge to a Legacy Docker Container running alongside the Mesh
        - name: "old-spring-boot-app"
          runtime: legacy_container
          endpoint: "127.0.0.1:8080" # Bypasses Wasm, acts as an L4/L7 bridge
          env:
            # We can still pass headers or context down to the legacy app
            MESH_INJECTED_HEADER: "true"

## 2. The WIT Contract (`wit/config-workloads.wit`)
The strict Wasm interface used by `system-faas-config-api`.

    interface config-workloads {
        enum runtime-type { faas-wasm, smolvm-microvm, legacy-container }
        enum secret-backend { system-faas-tde, external-vault }

        record secret-provider {
            name: string,
            backend: secret-backend,
            key-id: string,
        }

        record secret-mount {
            env-var: string,
            secret-ref: string,
        }

        record workload-spec {
            name: string,
            runtime: runtime-type,
            asset-ref: option<string>, // Nullable for legacy containers
            endpoint: option<string>,  // Only used by legacy containers
            env: list<tuple<string, string>>,
            secret-mounts: list<secret-mount>,
        }

        record workload-configuration {
            secrets: list<secret-provider>,
            workloads: list<workload-spec>,
        }

        /// Global validation (Ensures secret-refs actually exist in the provider list)
        validate-workload-config: func(config: workload-configuration) -> result<_, string>;

        /// CRUD for Tachyon-UI
        get-workload-config: func() -> result<workload-configuration, string>;
        apply-workload: func(spec: workload-spec) -> result<_, string>;
    }