# Design: Control Plane RBAC Data Model

## 1. The GitOps YAML Specification
This configuration secures the UI and MCP server operations.

    api_version: rbac.tachyon.io/v1alpha1
    kind: ControlPlaneAccess
    metadata:
      name: edge-admin-policies
      environment: production
      
    spec:
      # 1. ROLE DEFINITIONS (What actions are allowed?)
      roles:
        - name: "network-admin"
          permissions:
            - domains: ["config-routing", "config-resilience"]
              actions: ["CREATE", "READ", "UPDATE", "DELETE"]
            - domains: ["config-topology", "config-hardware"]
              actions: ["READ"] # Can view but not modify infrastructure

        - name: "ai-orchestrator"
          permissions:
            - domains: ["config-ai", "config-cache", "config-assets"]
              actions: ["CREATE", "READ", "UPDATE", "DELETE"]

        - name: "tenant-developer"
          permissions:
            - domains: ["config-routing", "config-workloads"]
              actions: ["CREATE", "READ", "UPDATE", "DELETE"]
              
      # 2. ROLE BINDINGS & ACLs (Who gets the role, and where can they apply it?)
      bindings:
        - name: "bind-infra-team"
          role_ref: "network-admin"
          subjects:
            - kind: oidc_group
              name: "tachyon-net-ops"
          # No resource_selector means global scope
          
        - name: "bind-dev-team-finance"
          role_ref: "tenant-developer"
          subjects:
            - kind: oidc_group
              name: "finance-devs"
          # 3. GRANULAR ACL: Can only modify resources tagged for their specific tenant fleet
          resource_selectors:
            match_labels:
              tenant: "finance"

## 2. The WIT Contract (`wit/config-rbac.wit`)
The strict Wasm interface used by `system-faas-authz` to intercept and validate calls to `system-faas-config-api`.

    interface config-rbac {
        enum action { create, read, update, delete }
        enum subject-kind { user, oidc-group, service-account }

        record permission {
            domains: list<string>, // e.g., "config-routing", "config-ai"
            actions: list<action>,
        }

        record role {
            name: string,
            permissions: list<permission>,
        }

        record subject {
            kind: subject-kind,
            name: string,
        }

        record label-selector {
            match-labels: list<tuple<string, string>>,
        }

        record role-binding {
            name: string,
            role-ref: string,
            subjects: list<subject>,
            resource-selectors: option<label-selector>,
        }

        record rbac-configuration {
            roles: list<role>,
            bindings: list<role-binding>,
        }

        /// Validation (Ensures roles referenced in bindings actually exist)
        validate-rbac-config: func(config: rbac-configuration) -> result<_, string>;

        /// Authorization Hook: Evaluates if a given subject can perform an action on a domain payload
        evaluate-access: func(subject-claims: list<tuple<string, string>>, action: action, domain: string, resource-labels: list<tuple<string, string>>) -> result<bool, string>;
    }