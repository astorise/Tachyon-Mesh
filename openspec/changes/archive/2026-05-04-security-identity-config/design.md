# Design: Security & Identity Data Model

## 1. The GitOps YAML Specification
This file configures the Zero-Trust posture and limits. It maps directly to the routes defined in the Traffic Management pillar.

    api_version: security.tachyon.io/v1alpha1
    kind: SecurityConfiguration
    metadata:
      name: edge-global-security
      environment: production
      
    spec:
      # 1. AUTHENTICATION (Who is the user?)
      authentication:
        providers:
          - name: corporate-oidc
            type: JWT
            issuer: "https://auth.corporate.local"
            jwks_uri: "https://auth.corporate.local/.well-known/jwks.json"
            identity_extraction:
              subject: "claims.sub"
              tenant: "claims.tenant_id"
              roles: "claims.groups"

      # 2. AUTHORIZATION (What can they do?)
      authorization:
        policies:
          - name: admin-api-access
            target_route_refs: ["api-v1-routing"] # Refers to Pilier 1 Route
            action: ALLOW
            conditions:
              required_roles: ["admin", "sys-op"]
          
          - name: block-legacy
            target_route_refs: ["legacy-db-proxy"]
            action: DENY
            conditions:
              untrusted_network: true

      # 3. RATE LIMITING (How much can they do?)
      rate_limits:
        - name: global-inference-quota
          target_route_refs: ["api-v1-routing"]
          algorithm: distributed_crdt
          scope: identity_tenant # Limits per tenant extracted from JWT
          limits:
            requests_per_second: 100
            burst: 20
          # Action to take when quota is exceeded
          on_exceed:
            status_code: 429
            headers:
              "X-RateLimit-Exceeded": "True"

## 2. The WIT Contract (`wit/config-security.wit`)
This contract is used by the `system-faas-config-api` to validate intents and expose CRUD operations.

    interface config-security {
        /// Enums for validation
        enum auth-type { jwt, mtls, api-key }
        enum action-type { allow, deny, custom-challenge }
        enum limit-algorithm { token-bucket, distributed-crdt }
        enum limit-scope { global, identity-tenant, identity-subject, ip-address }

        /// Authentication
        record identity-extractor {
            subject: string,
            tenant: option<string>,
            roles: option<string>,
        }

        record auth-provider {
            name: string,
            provider-type: auth-type,
            issuer-url: option<string>,
            jwks-url: option<string>,
            extractor: identity-extractor,
        }

        /// Authorization
        record rbac-condition {
            required-roles: list<string>,
        }

        record authz-policy {
            name: string,
            target-route-refs: list<string>,
            action: action-type,
            conditions: rbac-condition,
        }

        /// Rate Limiting
        record limit-threshold {
            requests-per-second: u32,
            burst: u32,
        }

        record rate-limit-policy {
            name: string,
            target-route-refs: list<string>,
            algo: limit-algorithm,
            scope: limit-scope,
            threshold: limit-threshold,
        }

        /// Root Payload
        record security-configuration {
            providers: list<auth-provider>,
            policies: list<authz-policy>,
            rate-limits: list<rate-limit-policy>,
        }

        /// Global validation
        validate-security-config: func(config: security-configuration) -> result<_, string>;

        /// CRUD Operations exposed for UI/MCP
        get-security-config: func() -> result<security-configuration, string>;
        apply-rate-limit: func(limit: rate-limit-policy) -> result<_, string>;
        delete-rate-limit: func(name: string) -> result<_, string>;
    }