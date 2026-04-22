mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

const EXPECTED_HASH_HEADER: &str = "x-tachyon-expected-sha256";

struct Component;

#[derive(Serialize)]
struct AssetUploadResponse {
    asset_uri: String,
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        if !req.method.eq_ignore_ascii_case("POST") {
            return response(405, "Method Not Allowed");
        }
        if req.body.is_empty() {
            return response(400, "asset body must not be empty");
        }

        let hash = sha256_hash(&req.body);
        if let Some(expected_hash) = header_value(&req.headers, EXPECTED_HASH_HEADER) {
            if expected_hash.trim() != hash {
                return response(
                    400,
                    format!(
                        "asset checksum mismatch: expected `{}`, computed `{hash}`",
                        expected_hash.trim()
                    ),
                );
            }
        }

        if let Err(error) = persist_asset(&hash, &req.body) {
            return response(500, error);
        }

        response_json(
            200,
            &AssetUploadResponse {
                asset_uri: format!("tachyon://{hash}"),
            },
        )
    }
}

fn header_value(headers: &[(String, String)], expected: &str) -> Option<String> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(expected))
        .map(|(_, value)| value.clone())
}

fn persist_asset(hash: &str, body: &[u8]) -> Result<(), String> {
    let path = asset_path(hash);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create registry directory `{}`: {error}",
                parent.display()
            )
        })?;
    }
    fs::write(&path, body)
        .map_err(|error| format!("failed to write asset `{}`: {error}", path.display()))
}

fn asset_path(hash: &str) -> PathBuf {
    Path::new("assets").join(format!("{}.wasm", hash.trim_start_matches("sha256:")))
}

fn sha256_hash(body: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(body)))
}

fn response(
    status: u16,
    body: impl Into<Vec<u8>>,
) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status,
        headers: Vec::new(),
        body: body.into(),
        trailers: Vec::new(),
    }
}

fn response_json<T>(status: u16, payload: &T) -> bindings::exports::tachyon::mesh::handler::Response
where
    T: Serialize,
{
    match serde_json::to_vec(payload) {
        Ok(body) => bindings::exports::tachyon::mesh::handler::Response {
            status,
            headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            body,
            trailers: Vec::new(),
        },
        Err(error) => response(
            500,
            format!("failed to serialize response payload: {error}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::exports::tachyon::mesh::handler::Guest;

    #[test]
    fn upload_rejects_empty_body() {
        let response =
            Component::handle_request(bindings::exports::tachyon::mesh::handler::Request {
                method: "POST".to_owned(),
                uri: "/admin/assets".to_owned(),
                headers: Vec::new(),
                body: Vec::new(),
                trailers: Vec::new(),
            });

        assert_eq!(response.status, 400);
    }

    #[test]
    fn upload_validates_expected_hash_header() {
        let response =
            Component::handle_request(bindings::exports::tachyon::mesh::handler::Request {
                method: "POST".to_owned(),
                uri: "/admin/assets".to_owned(),
                headers: vec![(
                    EXPECTED_HASH_HEADER.to_owned(),
                    "sha256:deadbeef".to_owned(),
                )],
                body: b"hello".to_vec(),
                trailers: Vec::new(),
            });

        assert_eq!(response.status, 400);
    }
}
