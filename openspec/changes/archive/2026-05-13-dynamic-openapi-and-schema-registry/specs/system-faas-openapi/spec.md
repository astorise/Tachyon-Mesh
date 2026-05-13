# Technical Specification: OpenAPI System FaaS

## 1. New Component Creation
Create a new crate `systems/system-faas-openapi` targeting `wasm32-wasip2`.

## 2. Dynamic Schema Serving
The component acts as a smart proxy and documentation server.

```rust
use tachyon_sdk::prelude::*;

#[tachyon_function]
async fn handle_request(req: Request) -> Response {
    match req.uri().path() {
        "/admin/schema/openapi.json" => {
            // 1. Fetch base schema via host call
            let mut base_schema = host::get_base_openapi();
            
            // 2. (Future Scope) Query local FaaS registry to merge dynamic user routes
            
            Response::builder()
                .header("Content-Type", "application/json")
                .body(base_schema)
                .build()
        },
        "/admin/docs" => {
            // Serve embedded Swagger UI HTML pointing to the JSON route
            let swagger_html = include_str!("swagger-ui.html");
            Response::builder()
                .header("Content-Type", "text/html")
                .body(swagger_html)
                .build()
        },
        _ => Response::builder().status(404).build(),
    }
}
```

*Note: Bundling the HTML inside the WASM artifact using `include_str!` guarantees that the `core-host` binary has zero UI assets inside it.*