# system-faas-openapi Specification

## Purpose
Provides interactive API documentation for the Tachyon Mesh admin API via a WASM component that bundles Swagger UI assets and proxies the OpenAPI schema from the core-host runtime.

## Requirements

### Requirement: system-faas-openapi MUST serve Swagger UI from an embedded asset
The `system-faas-openapi` WASM component SHALL embed the Swagger UI HTML using `include_str!` so that the `core-host` binary contains zero separate UI asset files.

#### Scenario: Documentation page is served without filesystem access
- **WHEN** a request arrives at `GET /admin/docs`
- **THEN** the component returns a 200 response with `content-type: text/html; charset=utf-8`
- **AND** the HTML body is the embedded Swagger UI pointing at `/admin/schema/openapi.json`
- **AND** no file is read from disk at runtime

### Requirement: system-faas-openapi MUST proxy the OpenAPI schema
The component SHALL forward `GET /admin/schema/openapi.json` to the core-host loopback via `outbound_http::send_request`, returning the utoipa-generated OpenAPI 3.1 JSON.

#### Scenario: Schema request is proxied to core-host
- **WHEN** a request arrives at `GET /admin/schema/openapi.json`
- **THEN** the component calls `outbound_http::send_request("GET", "http://127.0.0.1:8080/admin/schema/openapi.json", ...)`
- **AND** the response body and status are forwarded back to the caller

#### Scenario: Upstream failure is surfaced as 502
- **WHEN** the outbound HTTP call fails
- **THEN** the component returns status 502 with an error message body

### Requirement: system-faas-openapi MUST use the system-faas-guest WIT world
The component SHALL export `handler::handle-request` from the `tachyon:mesh@1.0.0` `system-faas-guest` world, consistent with all other system FaaS components.

#### Scenario: Component is loadable by the core-host WASM runtime
- **GIVEN** the component is compiled to `wasm32-wasip2`
- **WHEN** core-host attempts to load it via the `system-faas-guest` interface
- **THEN** the component satisfies all required exports without panicking on load
