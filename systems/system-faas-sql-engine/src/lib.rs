mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

use serde::Serialize;

struct Component;

#[derive(Debug, Serialize)]
struct PushdownPlan {
    start_key: String,
    end_key: String,
    limit: u32,
    pushdown_filter_wasm: Vec<u8>,
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        let suffix = req.body.last().copied().unwrap_or_default();
        let plan = compile_suffix_pushdown_filter(suffix);
        match serde_json::to_vec(&plan) {
            Ok(body) => response(200, body),
            Err(error) => response(500, format!("failed to encode pushdown plan: {error}")),
        }
    }
}

fn compile_suffix_pushdown_filter(suffix: u8) -> PushdownPlan {
    PushdownPlan {
        start_key: String::new(),
        end_key: String::new(),
        limit: 1_000,
        // POC bytecode placeholder: the storage node treats this as a registry
        // key for the built-in suffix filter until dynamic JIT compilation lands.
        pushdown_filter_wasm: vec![b's', b'u', b'f', b'f', b'i', b'x', b':', suffix],
    }
}

fn response(
    status: u16,
    body: impl Into<Vec<u8>>,
) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status,
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body: body.into(),
        trailers: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_dummy_suffix_filter_payload() {
        let plan = compile_suffix_pushdown_filter(b'z');

        assert_eq!(plan.pushdown_filter_wasm, b"suffix:z");
        assert_eq!(plan.limit, 1_000);
    }
}
