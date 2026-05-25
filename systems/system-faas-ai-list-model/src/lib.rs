mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "control-plane-faas",
    });

    export!(Component);
}

use serde::{Deserialize, Serialize};
use std::cell::RefCell;

const MODELS_TABLE: &str = "ai-models-registry";
const ROUTE_REGISTER: &str = "/internal/ai-list-model/register";
const ROUTE_LIST: &str = "/internal/ai-list-model/list-models";
const ROUTE_DEREGISTER_PREFIX: &str = "/internal/ai-list-model/deregister/";

struct Component;

thread_local! {
    static MODELS_CACHE: RefCell<Option<Vec<ModelInfo>>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub alias: String,
    pub engine: String,
    pub vram_required_mb: u64,
    pub status: String,
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        let result = route_request(&req.method, route_path(&req.uri), &req.body);
        match result {
            Ok((status, body)) => json_response(status, body),
            Err(error) => json_response(400, error.into_bytes()),
        }
    }
}

impl bindings::Guest for Component {
    fn on_tick() {}
}

fn route_request(method: &str, path: &str, body: &[u8]) -> Result<(u16, Vec<u8>), String> {
    if method.eq_ignore_ascii_case("POST") && path == ROUTE_REGISTER {
        let info: ModelInfo = serde_json::from_slice(body)
            .map_err(|e| format!("invalid model registration payload: {e}"))?;
        let table = bindings::tachyon::mesh::kv_partition::Table::new(MODELS_TABLE);
        let value = serde_json::to_vec(&info)
            .map_err(|e| format!("failed to encode model info: {e}"))?;
        table
            .set(&info.alias, &value)
            .map_err(|e| format!("model registry write failed: {e}"))?;
        invalidate_cache();
        return Ok((201, b"model registered".to_vec()));
    }

    if method.eq_ignore_ascii_case("DELETE") && path.starts_with(ROUTE_DEREGISTER_PREFIX) {
        let alias = path.trim_start_matches(ROUTE_DEREGISTER_PREFIX);
        let table = bindings::tachyon::mesh::kv_partition::Table::new(MODELS_TABLE);
        table
            .delete(alias)
            .map_err(|e| format!("model registry delete failed: {e}"))?;
        invalidate_cache();
        return Ok((204, Vec::new()));
    }

    if method.eq_ignore_ascii_case("GET") && path == ROUTE_LIST {
        let models = list_models()?;
        let body = serde_json::to_vec(&models)
            .map_err(|e| format!("failed to encode model list: {e}"))?;
        return Ok((200, body));
    }

    Err(format!("unsupported ai-list-model route `{method} {path}`"))
}

fn list_models() -> Result<Vec<ModelInfo>, String> {
    if let Some(cached) = MODELS_CACHE.with(|c| c.borrow().clone()) {
        return Ok(cached);
    }
    let table = bindings::tachyon::mesh::kv_partition::Table::new(MODELS_TABLE);
    let rows = table
        .get_range("", "\u{10ffff}", 10_000, 0)
        .map_err(|e| format!("model registry scan failed: {e}"))?;
    let models = rows
        .into_iter()
        .filter_map(|(_, v)| serde_json::from_slice::<ModelInfo>(&v).ok())
        .collect::<Vec<_>>();
    MODELS_CACHE.with(|c| *c.borrow_mut() = Some(models.clone()));
    Ok(models)
}

fn invalidate_cache() {
    MODELS_CACHE.with(|c| *c.borrow_mut() = None);
}

fn route_path(uri: &str) -> &str {
    uri.split_once('?').map(|(p, _)| p).unwrap_or(uri)
}

fn json_response(
    status: u16,
    body: Vec<u8>,
) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status,
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body,
        trailers: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_register_rejects_malformed_payload() {
        let result = route_request("POST", ROUTE_REGISTER, b"not json");
        assert!(result.is_err());
    }

    #[test]
    fn route_list_key_is_stable() {
        assert_eq!(ROUTE_LIST, "/internal/ai-list-model/list-models");
    }

    #[test]
    fn deregister_prefix_strips_correctly() {
        let path = "/internal/ai-list-model/deregister/my-model";
        let alias = path.trim_start_matches(ROUTE_DEREGISTER_PREFIX);
        assert_eq!(alias, "my-model");
    }
}
