mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "background-system-faas",
    });

    export!(Component);
}

use serde::{Deserialize, Serialize};

const QUEUE_URL_ENV: &str = "QUEUE_URL";
const TARGET_ROUTE_ENV: &str = "TARGET_ROUTE";
const RECEIVE_WAIT_TIME_SECONDS: u32 = 20;

struct Component;

impl bindings::Guest for Component {
    fn on_tick() {
        if let Err(error) = poll_once() {
            eprintln!("system-faas-sqs tick failed: {error}");
        }
    }
}

fn poll_once() -> Result<(), String> {
    let queue_url = required_env(QUEUE_URL_ENV)?;
    let target_route = normalize_target_route(&required_env(TARGET_ROUTE_ENV)?)?;
    let receive_request = serde_json::to_vec(&ReceiveRequest {
        wait_time_seconds: RECEIVE_WAIT_TIME_SECONDS,
    })
    .map_err(|error| format!("failed to encode receive request: {error}"))?;
    let receive_response = bindings::tachyon::mesh::outbound_http::send_request(
        "POST",
        &queue_receive_url(&queue_url),
        &json_headers(),
        &receive_request,
    )?;
    if receive_response.status >= 400 {
        return Err(format!(
            "queue receive request failed with status {}",
            receive_response.status
        ));
    }

    let messages = decode_messages(&receive_response.body)?;
    for message in messages {
        let dispatch_response = bindings::tachyon::mesh::outbound_http::send_request(
            "POST",
            &format!("http://mesh{target_route}"),
            &json_headers(),
            message.body.as_bytes(),
        )?;
        if dispatch_response.status == 200 {
            let delete_request = serde_json::to_vec(&DeleteRequest {
                receipt_handle: message.receipt_handle.clone(),
            })
            .map_err(|error| format!("failed to encode delete request: {error}"))?;
            let _ = bindings::tachyon::mesh::outbound_http::send_request(
                "POST",
                &queue_delete_url(&queue_url),
                &json_headers(),
                &delete_request,
            )?;
        }
    }

    Ok(())
}

fn required_env(name: &str) -> Result<String, String> {
    std::env::var(name)
        .map(|value| value.trim().to_owned())
        .map_err(|_| format!("missing required environment variable `{name}`"))?
        .pipe(|value| {
            if value.is_empty() {
                Err(format!("environment variable `{name}` must not be empty"))
            } else {
                Ok(value)
            }
        })
}

fn normalize_target_route(route: &str) -> Result<String, String> {
    let trimmed = route.trim();
    if trimmed.is_empty() {
        return Err("TARGET_ROUTE must not be empty".to_owned());
    }
    Ok(if trimmed.starts_with('/') {
        trimmed.to_owned()
    } else {
        format!("/{trimmed}")
    })
}

fn queue_receive_url(base_url: &str) -> String {
    format!("{}/receive", base_url.trim_end_matches('/'))
}

fn queue_delete_url(base_url: &str) -> String {
    format!("{}/delete", base_url.trim_end_matches('/'))
}

fn json_headers() -> Vec<(String, String)> {
    vec![("content-type".to_owned(), "application/json".to_owned())]
}

fn decode_messages(body: &[u8]) -> Result<Vec<QueueMessage>, String> {
    if body.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_slice::<ReceiveResponse>(body)
        .map(|response| response.messages)
        .map_err(|error| format!("failed to parse queue response: {error}"))
}

#[derive(Serialize)]
struct ReceiveRequest {
    wait_time_seconds: u32,
}

#[derive(Serialize)]
struct DeleteRequest {
    receipt_handle: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct ReceiveResponse {
    #[serde(default)]
    messages: Vec<QueueMessage>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct QueueMessage {
    body: String,
    receipt_handle: String,
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}

impl<T> Pipe for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_target_route_adds_leading_slash() {
        assert_eq!(
            normalize_target_route("api/worker").expect("route should normalize"),
            "/api/worker"
        );
        assert_eq!(
            normalize_target_route("/api/worker").expect("route should normalize"),
            "/api/worker"
        );
    }

    #[test]
    fn queue_urls_trim_trailing_slash() {
        assert_eq!(
            queue_receive_url("http://queue.local/example/"),
            "http://queue.local/example/receive"
        );
        assert_eq!(
            queue_delete_url("http://queue.local/example/"),
            "http://queue.local/example/delete"
        );
    }

    #[test]
    fn decode_messages_accepts_empty_body() {
        assert!(decode_messages(&[])
            .expect("empty response should succeed")
            .is_empty());
    }

    #[test]
    fn decode_messages_parses_receipt_handles() {
        let messages =
            decode_messages(br#"{"messages":[{"body":"force-ok","receipt_handle":"abc-123"}]}"#)
                .expect("queue response should parse");

        assert_eq!(
            messages,
            vec![QueueMessage {
                body: "force-ok".to_owned(),
                receipt_handle: "abc-123".to_owned(),
            }]
        );
    }
}
