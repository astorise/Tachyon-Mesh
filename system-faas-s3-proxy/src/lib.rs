mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

const REAL_S3_BUCKET_ENV: &str = "REAL_S3_BUCKET";
const TARGET_ROUTE_ENV: &str = "TARGET_ROUTE";
const BUFFER_ROUTE_ENV: &str = "BUFFER_ROUTE";
const S3_AUTHORIZATION_ENV: &str = "S3_AUTHORIZATION";
const OBJECT_KEY_HEADER: &str = "x-tachyon-object-key";
const ORIGINAL_ROUTE_HEADER: &str = "x-tachyon-original-route";
const DEFAULT_BUFFER_ROUTE: &str = "/system/buffer";

struct Component;

#[derive(Debug, Serialize)]
struct UploadEvent {
    bucket: String,
    key: String,
    size_bytes: u64,
    content_type: String,
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        if req.method != "PUT" {
            return response(405, "unsupported method");
        }

        match proxy_upload(req) {
            Ok(event) => json_response(200, &event),
            Err(error) => response(502, error),
        }
    }
}

fn proxy_upload(
    req: bindings::exports::tachyon::mesh::handler::Request,
) -> Result<UploadEvent, String> {
    let bucket = required_env(REAL_S3_BUCKET_ENV)?;
    let target_route = normalize_route(&required_env(TARGET_ROUTE_ENV)?)?;
    let buffer_route = normalize_route(&env_or_default(BUFFER_ROUTE_ENV, DEFAULT_BUFFER_ROUTE))?;
    let object_key = object_key(&req.headers);
    let upload_url = upload_url(&bucket, &object_key);
    let content_type = header_value(&req.headers, "content-type")
        .unwrap_or("application/octet-stream")
        .to_owned();

    let upload_response = bindings::tachyon::mesh::outbound_http::send_request(
        "PUT",
        &upload_url,
        &upload_headers(&req.headers),
        &req.body,
    )?;
    if !(200..300).contains(&upload_response.status) {
        return Err(format!(
            "upstream storage returned HTTP {}: {}",
            upload_response.status,
            String::from_utf8_lossy(&upload_response.body)
        ));
    }

    let event = UploadEvent {
        bucket,
        key: object_key,
        size_bytes: req.body.len() as u64,
        content_type,
    };
    enqueue_event(&target_route, &buffer_route, &event)?;
    Ok(event)
}

fn enqueue_event(
    target_route: &str,
    buffer_route: &str,
    event: &UploadEvent,
) -> Result<(), String> {
    let body = serde_json::to_vec(event)
        .map_err(|error| format!("failed to encode upload event: {error}"))?;
    let response = bindings::tachyon::mesh::outbound_http::send_request(
        "POST",
        &format!("http://mesh{buffer_route}"),
        &[
            ("content-type".to_owned(), "application/json".to_owned()),
            (ORIGINAL_ROUTE_HEADER.to_owned(), target_route.to_owned()),
            ("x-priority".to_owned(), "high".to_owned()),
        ],
        &body,
    )?;
    if response.status >= 400 {
        return Err(format!(
            "buffer route returned HTTP {}: {}",
            response.status,
            String::from_utf8_lossy(&response.body)
        ));
    }
    Ok(())
}

fn upload_headers(request_headers: &[(String, String)]) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    if let Some(content_type) = header_value(request_headers, "content-type") {
        headers.push(("content-type".to_owned(), content_type.to_owned()));
    }
    if let Some(content_md5) = header_value(request_headers, "content-md5") {
        headers.push(("content-md5".to_owned(), content_md5.to_owned()));
    }

    let auth = std::env::var(S3_AUTHORIZATION_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            header_value(request_headers, "authorization")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        });
    if let Some(auth) = auth {
        headers.push(("authorization".to_owned(), auth));
    }
    headers
}

fn object_key(headers: &[(String, String)]) -> String {
    header_value(headers, OBJECT_KEY_HEADER)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("upload-{}.bin", unix_time_ms()))
}

fn upload_url(bucket: &str, object_key: &str) -> String {
    format!(
        "{}/{}",
        bucket.trim_end_matches('/'),
        object_key.trim_start_matches('/')
    )
}

fn required_env(name: &str) -> Result<String, String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("missing required environment variable `{name}`"))
}

fn env_or_default(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_owned())
}

fn normalize_route(route: &str) -> Result<String, String> {
    let trimmed = route.trim();
    if trimmed.is_empty() {
        return Err("route must not be empty".to_owned());
    }
    Ok(if trimmed.starts_with('/') {
        trimmed.to_owned()
    } else {
        format!("/{trimmed}")
    })
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn response(
    status: u16,
    body: impl Into<Vec<u8>>,
) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status,
        headers: vec![],
        body: body.into(),
        trailers: vec![],
    }
}

fn json_response<T: Serialize>(
    status: u16,
    body: &T,
) -> bindings::exports::tachyon::mesh::handler::Response {
    match serde_json::to_vec(body) {
        Ok(body) => bindings::exports::tachyon::mesh::handler::Response {
            status,
            headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            body,
            trailers: vec![],
        },
        Err(error) => response(500, format!("failed to encode upload response: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_route_adds_leading_slash() {
        assert_eq!(
            normalize_route("api/uploads").expect("route should normalize"),
            "/api/uploads"
        );
    }

    #[test]
    fn upload_headers_prefer_explicit_env_authorization() {
        std::env::set_var(S3_AUTHORIZATION_ENV, "Bearer proxy");
        let headers = upload_headers(&[
            (
                "content-type".to_owned(),
                "application/octet-stream".to_owned(),
            ),
            ("authorization".to_owned(), "Bearer caller".to_owned()),
        ]);
        assert_eq!(
            headers,
            vec![
                (
                    "content-type".to_owned(),
                    "application/octet-stream".to_owned()
                ),
                ("authorization".to_owned(), "Bearer proxy".to_owned()),
            ]
        );
        std::env::remove_var(S3_AUTHORIZATION_ENV);
    }

    #[test]
    fn object_key_prefers_explicit_header() {
        assert_eq!(
            object_key(&[(OBJECT_KEY_HEADER.to_owned(), "uploads/demo.txt".to_owned())]),
            "uploads/demo.txt"
        );
    }
}
