# Design: Fleet Profiles & Node Selectors Data Model

## 1. The GitOps YAML Specification
This file defines logical groupings of nodes and binds other configuration resources to them.

    api_version: fleet.tachyon.io/v1alpha1
    kind: FleetProfile
    metadata:
      name: europe-production-gateways
      environment: production
      
    spec:
      # 1. NODE SELECTORS (Who belongs to this fleet?)
      selector:
        match_labels:
          env: "production"
          role: "api-gateway"
          region: "eu-*" # Supports glob pattern matching
        exclude_labels:
          maintenance: "true"

      # 2. CONFIGURATION BINDINGS (What applies to them?)
      bindings:
        routing_refs: ["edge-main-routing"] # Refers to config-routing
        security_refs: ["edge-global-security"] # Refers to config-security
        hardware_refs: ["ebpf-optimized-network"] # Refers to config-hardware
        
      # 3. ROLLOUT STRATEGY (How updates are applied to this fleet)
      rollout:
        strategy: rolling_update
        max_unavailable: "10%" # Ensures 90% of the fleet remains active during config reloads

## 2. The WIT Contract (`wit/config-fleet.wit`)
The strict Wasm interface used by `system-faas-config-api` to compute fleet topology.

    interface config-fleet {
        enum rollout-strategy { rolling-update, blue-green, all-at-once }

        record label-selector {
            match-labels: list<tuple<string, string>>,
            exclude-labels: option<list<tuple<string, string>>>,
        }

        record rollout-policy {
            strategy: rollout-strategy,
            max-unavailable-percentage: u8,
        }

        record config-bindings {
            routing-refs: list<string>,
            security-refs: list<string>,
            resilience-refs: list<string>,
            ops-refs: list<string>,
            storage-refs: list<string>,
            hardware-refs: list<string>,
        }

        record fleet-profile {
            name: string,
            selector: label-selector,
            bindings: config-bindings,
            rollout: option<rollout-policy>,
        }

        record fleet-configuration {
            profiles: list<fleet-profile>,
        }

        /// Validation (Ensures referenced configs actually exist in the GitOps store)
        validate-fleet-config: func(config: fleet-configuration) -> result<_, string>;

        /// Evaluates if a given set of node tags matches a specific profile
        matches-profile: func(node-tags: list<tuple<string, string>>, profile-name: string) -> result<bool, string>;

        /// CRUD Operations for Tachyon-UI / MCP
        get-fleet-config: func() -> result<fleet-configuration, string>;
        apply-fleet-profile: func(profile: fleet-profile) -> result<_, string>;
    }