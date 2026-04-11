use prost::Message;

mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "faas-guest",
    });

    export!(Component);
}

#[derive(Clone, PartialEq, Message)]
struct HelloRequest {
    #[prost(string, tag = "1")]
    name: String,
}

#[derive(Clone, PartialEq, Message)]
struct HelloResponse {
    #[prost(string, tag = "1")]
    message: String,
}

struct Component;

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        let request = match decode_grpc_message::<HelloRequest>(&req.body) {
            Ok(request) => request,
            Err(message) => return grpc_error("3", &message),
        };

        let response = HelloResponse {
            message: format!("Hello, {}!", default_name(&request.name)),
        };
        let body = match encode_grpc_message(&response) {
            Ok(body) => body,
            Err(message) => return grpc_error("13", &message),
        };

        bindings::exports::tachyon::mesh::handler::Response {
            status: 200,
            headers: vec![("content-type".to_owned(), "application/grpc".to_owned())],
            body,
            trailers: vec![("grpc-status".to_owned(), "0".to_owned())],
        }
    }
}

fn default_name(name: &str) -> &str {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        "world"
    } else {
        trimmed
    }
}

fn decode_grpc_message<T>(payload: &[u8]) -> Result<T, String>
where
    T: Message + Default,
{
    if payload.len() < 5 {
        return Err("gRPC payload must include the 5-byte frame prefix".to_owned());
    }

    if payload[0] != 0 {
        return Err("compressed gRPC frames are not supported".to_owned());
    }

    let message_len = u32::from_be_bytes([payload[1], payload[2], payload[3], payload[4]]) as usize;
    let framed = &payload[5..];
    if framed.len() != message_len {
        return Err(format!(
            "gRPC frame length mismatch: declared {message_len} bytes but received {}",
            framed.len()
        ));
    }

    T::decode(framed).map_err(|error| format!("failed to decode protobuf payload: {error}"))
}

fn encode_grpc_message<T>(message: &T) -> Result<Vec<u8>, String>
where
    T: Message,
{
    let mut payload = Vec::new();
    message
        .encode(&mut payload)
        .map_err(|error| format!("failed to encode protobuf payload: {error}"))?;

    let mut framed = Vec::with_capacity(payload.len() + 5);
    framed.push(0);
    framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    framed.extend_from_slice(&payload);
    Ok(framed)
}

fn grpc_error(status: &str, message: &str) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status: 200,
        headers: vec![("content-type".to_owned(), "application/grpc".to_owned())],
        body: Vec::new(),
        trailers: vec![
            ("grpc-status".to_owned(), status.to_owned()),
            ("grpc-message".to_owned(), message.to_owned()),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_decodes_framed_messages() {
        let body = encode_grpc_message(&HelloRequest {
            name: "Tachyon".to_owned(),
        })
        .expect("frame should encode");
        let decoded = decode_grpc_message::<HelloRequest>(&body).expect("frame should decode");

        assert_eq!(decoded.name, "Tachyon");
    }
}
