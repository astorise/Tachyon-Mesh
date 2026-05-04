# Design: Traffic Management Data Model

## 1. The GitOps YAML Specification
This is the human-readable and UI-generated representation of the routing state. It is stored by the GitOps broker.

    api_version: routing.tachyon.io/v1alpha1
    kind: TrafficConfiguration
    metadata:
      name: edge-main-routing
      environment: production

    spec:
      gateways:
        - name: public-https
          protocol: HTTPS
          bind_address: 0.0.0.0:443
          tls:
            mode: terminate
            cert_resolver: system-faas-cert-manager

        - name: fastpath-tcp
          protocol: TCP
          bind_address: 0.0.0.0:8080
          acceleration: ebpf

      routes:
        - name: api-v1-routing
          gateway_refs: ["public-https"]
          type: HTTP
          rules:
            - match:
                host: "api.tachyon.local"
                path: { prefix: "/v1/inference" }
              action:
                route_to:
                  target_ref: "ai-inference-wasm"
                  weight: 100

      target_groups:
        - name: ai-inference-wasm
          type: wasm_component
          component_id: "system-faas-model-broker"

## 2. The WIT Contract (`wit/config-routing.wit`)
This contract is used by `system-faas-config-api` to safely validate payloads and expose CRUD operations to the UI via MCP.

    interface config-routing {
        enum protocol { tcp, udp, http, https, grpc, uds }
        enum accel-mode { userspace, ebpf }

        record gateway-config {
            name: string,
            proto: protocol,
            bind-address: string,
            accel: accel-mode,
        }

        record route-config {
            name: string,
            gateway-refs: list<string>,
            // routing rules simplified for brevity...
        }

        record traffic-configuration {
            gateways: list<gateway-config>,
            routes: list<route-config>,
        }

        /// Global validation
        validate-traffic-config: func(config: traffic-configuration) -> result<_, string>;

        /// CRUD Operations exposed for Tachyon-UI / MCP Server
        get-config: func() -> result<traffic-configuration, string>;

        /// Upsert: Creates a new route or updates an existing one in the array
        apply-route: func(route: route-config) -> result<_, string>;

        /// Deletes a specific route from the array by name
        delete-route: func(name: string) -> result<_, string>;
    }
