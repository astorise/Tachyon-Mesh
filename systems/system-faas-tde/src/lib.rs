use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use serde::{Deserialize, Serialize};

mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "control-plane-faas",
    });

    export!(Component);
}

const TDE_KEY_HEX_ENV: &str = "TDE_KEY_HEX";

struct Component;

impl bindings::Guest for Component {
    fn on_tick() {}
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        match (req.method.as_str(), request_path(&req.uri).as_str()) {
            ("POST", "/encrypt_chunk") | ("POST", "/encrypt") => {
                transform(req.body, Operation::Encrypt)
            }
            ("POST", "/decrypt_chunk") | ("POST", "/decrypt") => {
                transform(req.body, Operation::Decrypt)
            }
            _ => response(404, b"unknown tde endpoint".to_vec()),
        }
    }
}

fn transform(
    body: Vec<u8>,
    operation: Operation,
) -> bindings::exports::tachyon::mesh::handler::Response {
    let request = match serde_json::from_slice::<ChunkRequest>(&body) {
        Ok(request) => request,
        Err(error) => return response(400, format!("invalid TDE request: {error}").into_bytes()),
    };
    let result = match operation {
        Operation::Encrypt => encrypt_chunk(&request.data, request.nonce),
        Operation::Decrypt => decrypt_chunk(&request.data, request.nonce),
    };
    match result {
        Ok(data) => json_response(200, &ChunkResponse { data }),
        Err(error) => response(400, error.into_bytes()),
    }
}

pub fn encrypt_chunk(data: &[u8], nonce: u64) -> Result<Vec<u8>, String> {
    let nonce = nonce_bytes(nonce);
    cipher()
        .encrypt(Nonce::from_slice(&nonce), data)
        .map_err(|_| "failed to encrypt TDE chunk with AES-256-GCM".to_owned())
}

pub fn decrypt_chunk(data: &[u8], nonce: u64) -> Result<Vec<u8>, String> {
    let nonce = nonce_bytes(nonce);
    cipher()
        .decrypt(Nonce::from_slice(&nonce), data)
        .map_err(|_| "failed to decrypt TDE chunk or authenticate ciphertext".to_owned())
}

fn cipher() -> Aes256Gcm {
    Aes256Gcm::new((&key_bytes()).into())
}

fn key_bytes() -> [u8; 32] {
    std::env::var(TDE_KEY_HEX_ENV)
        .ok()
        .and_then(|value| decode_hex_32(value.trim()).ok())
        .unwrap_or([0x42; 32])
}

fn nonce_bytes(value: u64) -> [u8; 12] {
    let mut nonce = [0_u8; 12];
    nonce[4..].copy_from_slice(&value.to_be_bytes());
    nonce
}

fn decode_hex_32(value: &str) -> Result<[u8; 32], String> {
    if value.len() != 64 {
        return Err("TDE key must be 64 hexadecimal characters".to_owned());
    }
    let mut out = [0_u8; 32];
    for (index, chunk) in value.as_bytes().chunks(2).enumerate() {
        let pair = std::str::from_utf8(chunk).map_err(|error| error.to_string())?;
        out[index] = u8::from_str_radix(pair, 16).map_err(|error| error.to_string())?;
    }
    Ok(out)
}

fn request_path(uri: &str) -> String {
    let path = uri.split('?').next().unwrap_or(uri).trim();
    if path.is_empty() || path == "/" {
        "/".to_owned()
    } else if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    }
}

fn json_response<T: Serialize>(
    status: u16,
    value: &T,
) -> bindings::exports::tachyon::mesh::handler::Response {
    match serde_json::to_vec(value) {
        Ok(body) => bindings::exports::tachyon::mesh::handler::Response {
            status,
            headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            body,
            trailers: Vec::new(),
        },
        Err(error) => response(
            500,
            format!("failed to encode TDE response: {error}").into_bytes(),
        ),
    }
}

fn response(status: u16, body: Vec<u8>) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status,
        headers: Vec::new(),
        body,
        trailers: Vec::new(),
    }
}

#[derive(Clone, Copy)]
enum Operation {
    Encrypt,
    Decrypt,
}

#[derive(Debug, Deserialize)]
struct ChunkRequest {
    nonce: u64,
    data: Vec<u8>,
}

#[derive(Debug, Serialize)]
struct ChunkResponse {
    data: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::{decrypt_chunk, encrypt_chunk};

    #[test]
    fn aes_gcm_chunk_round_trips_and_authenticates() {
        let plaintext = b"patient-record: secret";
        let ciphertext = encrypt_chunk(plaintext, 7).expect("encryption should succeed");

        assert_ne!(ciphertext, plaintext);
        assert_eq!(
            decrypt_chunk(&ciphertext, 7).expect("decryption should succeed"),
            plaintext
        );

        let mut tampered = ciphertext;
        tampered[0] ^= 0x01;
        assert!(decrypt_chunk(&tampered, 7).is_err());
    }
}
