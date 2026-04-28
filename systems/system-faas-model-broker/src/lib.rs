mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

const MODEL_CHUNK_BYTES: usize = 5 * 1024 * 1024;
const INIT_PATH: &str = "/admin/models/init";
const UPLOAD_PREFIX: &str = "/admin/models/upload/";
const COMMIT_PREFIX: &str = "/admin/models/commit/";
const ABORT_PREFIX: &str = "/admin/models/abort/";

struct Component;

#[derive(Debug, Deserialize)]
struct InitUploadRequest {
    expected_hash: String,
    size_bytes: u64,
}

#[derive(Debug, Serialize)]
struct InitUploadResponse {
    upload_id: String,
}

#[derive(Debug, Serialize)]
struct CommitUploadResponse {
    model_path: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PendingUpload {
    expected_hash: String,
    size_bytes: u64,
    bytes_received: u64,
    last_part: u64,
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        let result = if req.method.eq_ignore_ascii_case("POST") && route_path(&req.uri) == INIT_PATH
        {
            init_upload(&req.body)
                .and_then(|upload_id| response_json(200, &InitUploadResponse { upload_id }))
        } else if req.method.eq_ignore_ascii_case("PUT")
            && route_path(&req.uri).starts_with(UPLOAD_PREFIX)
        {
            append_chunk(&req.uri, &req.body).map(|_| response(202, "Accepted"))
        } else if req.method.eq_ignore_ascii_case("POST")
            && route_path(&req.uri).starts_with(COMMIT_PREFIX)
        {
            commit_upload(&req.uri)
                .and_then(|model_path| response_json(200, &CommitUploadResponse { model_path }))
        } else if (req.method.eq_ignore_ascii_case("POST")
            || req.method.eq_ignore_ascii_case("DELETE"))
            && route_path(&req.uri).starts_with(ABORT_PREFIX)
        {
            abort_upload(&req.uri).map(|_| response(204, ""))
        } else {
            Ok(response(405, "Method Not Allowed"))
        };

        match result {
            Ok(response) => response,
            Err(error) => map_error_response(error),
        }
    }
}

fn init_upload(body: &[u8]) -> Result<String, String> {
    ensure_dirs()?;
    let payload: InitUploadRequest = serde_json::from_slice(body)
        .map_err(|error| format!("failed to decode model-init payload: {error}"))?;
    validate_hash(&payload.expected_hash)?;
    if payload.size_bytes == 0 {
        return Err("model upload size must be greater than zero".to_owned());
    }

    let upload_id = Uuid::new_v4().to_string();
    fs::write(staging_path(&upload_id), [])
        .map_err(|error| format!("failed to initialize model staging file: {error}"))?;
    save_pending_upload(
        &upload_id,
        &PendingUpload {
            expected_hash: payload.expected_hash,
            size_bytes: payload.size_bytes,
            bytes_received: 0,
            last_part: 0,
        },
    )?;

    Ok(upload_id)
}

fn append_chunk(uri: &str, chunk: &[u8]) -> Result<(), String> {
    ensure_dirs()?;
    if chunk.is_empty() {
        return Err("model upload chunk must not be empty".to_owned());
    }
    if chunk.len() > MODEL_CHUNK_BYTES {
        return Err(format!(
            "model upload chunk exceeds the 5 MiB protocol limit ({} bytes)",
            chunk.len()
        ));
    }

    let upload_id = upload_id_from_uri(uri, UPLOAD_PREFIX)?;
    let part = query_u64(uri, "part")?;
    let mut pending = load_pending_upload(&upload_id)?;
    if part != pending.last_part + 1 {
        return Err(format!(
            "model upload `{upload_id}` expected part {}, received {part}",
            pending.last_part + 1
        ));
    }
    let new_total = pending.bytes_received.saturating_add(chunk.len() as u64);
    if new_total > pending.size_bytes {
        return Err(format!(
            "model upload `{upload_id}` exceeds the declared size of {} bytes",
            pending.size_bytes
        ));
    }

    let mut file = OpenOptions::new()
        .append(true)
        .open(staging_path(&upload_id))
        .map_err(|error| format!("failed to open model staging file for `{upload_id}`: {error}"))?;
    file.write_all(chunk)
        .map_err(|error| format!("failed to append chunk for `{upload_id}`: {error}"))?;
    file.flush()
        .map_err(|error| format!("failed to flush chunk for `{upload_id}`: {error}"))?;

    pending.bytes_received = new_total;
    pending.last_part = part;
    save_pending_upload(&upload_id, &pending)?;
    Ok(())
}

fn commit_upload(uri: &str) -> Result<String, String> {
    ensure_dirs()?;
    let upload_id = upload_id_from_uri(uri, COMMIT_PREFIX)?;
    let pending = load_pending_upload(&upload_id)?;
    if pending.bytes_received != pending.size_bytes {
        return Err(format!(
            "model upload `{upload_id}` expected {} bytes but received {}",
            pending.size_bytes, pending.bytes_received
        ));
    }

    let staging_path = staging_path(&upload_id);
    let computed_hash = hash_file(&staging_path)?;
    if computed_hash != pending.expected_hash {
        // Hash mismatch means the staged content is unusable. Delete the .part and the
        // metadata so the upload slot is freed and the broker never accidentally
        // promotes a corrupted file to the final model name.
        cleanup_staging(&upload_id);
        return Err(format!(
            "model upload `{upload_id}` hash mismatch: expected `{}`, computed `{computed_hash}`",
            pending.expected_hash
        ));
    }

    let model_path = models_dir().join(format!(
        "{}.gguf",
        pending.expected_hash.trim_start_matches("sha256:")
    ));
    if model_path.exists() {
        fs::remove_file(&model_path).map_err(|error| {
            format!(
                "failed to replace existing model `{}`: {error}",
                model_path.display()
            )
        })?;
    }
    fs::rename(&staging_path, &model_path).map_err(|error| {
        format!(
            "failed to finalize model upload from `{}` to `{}`: {error}",
            staging_path.display(),
            model_path.display()
        )
    })?;
    let metadata_path = metadata_path(&upload_id);
    if metadata_path.exists() {
        fs::remove_file(&metadata_path).map_err(|error| {
            format!(
                "failed to remove upload metadata `{}`: {error}",
                metadata_path.display()
            )
        })?;
    }

    Ok(model_path.to_string_lossy().to_string())
}

/// Explicit client-driven cleanup for an in-progress upload. The broker is request-driven
/// (a Wasm guest) and cannot observe a peer disconnect mid-stream, so the orchestrator
/// (or admin tooling) signals an abort with `POST /admin/models/abort/{upload_id}` and
/// the broker drops the `.part` and the metadata file.
fn abort_upload(uri: &str) -> Result<(), String> {
    ensure_dirs()?;
    let upload_id = upload_id_from_uri(uri, ABORT_PREFIX)?;
    cleanup_staging(&upload_id);
    Ok(())
}

/// Best-effort removal of the `.part` and metadata for `upload_id`. Errors are
/// swallowed: the worst case is that `system-faas-gc` reaps the orphan via its
/// generic TTL sweep on a later tick, which is the documented fallback.
fn cleanup_staging(upload_id: &str) {
    let _ = fs::remove_file(staging_path(upload_id));
    let _ = fs::remove_file(metadata_path(upload_id));
}

fn ensure_dirs() -> Result<(), String> {
    fs::create_dir_all(models_dir())
        .map_err(|error| format!("failed to initialize models directory: {error}"))?;
    fs::create_dir_all(uploads_dir())
        .map_err(|error| format!("failed to initialize model-uploads directory: {error}"))?;
    Ok(())
}

fn models_dir() -> PathBuf {
    Path::new("models").to_path_buf()
}

fn uploads_dir() -> PathBuf {
    Path::new("model-uploads").to_path_buf()
}

fn staging_path(upload_id: &str) -> PathBuf {
    uploads_dir().join(format!("{upload_id}.part"))
}

fn metadata_path(upload_id: &str) -> PathBuf {
    uploads_dir().join(format!("{upload_id}.json"))
}

fn save_pending_upload(upload_id: &str, pending: &PendingUpload) -> Result<(), String> {
    let payload = serde_json::to_vec(pending)
        .map_err(|error| format!("failed to encode upload metadata: {error}"))?;
    fs::write(metadata_path(upload_id), payload)
        .map_err(|error| format!("failed to persist upload metadata for `{upload_id}`: {error}"))
}

fn load_pending_upload(upload_id: &str) -> Result<PendingUpload, String> {
    let payload = fs::read(metadata_path(upload_id))
        .map_err(|_| format!("unknown model upload `{upload_id}`"))?;
    serde_json::from_slice(&payload)
        .map_err(|error| format!("failed to decode upload metadata for `{upload_id}`: {error}"))
}

fn route_path(uri: &str) -> &str {
    uri.split_once('?').map(|(path, _)| path).unwrap_or(uri)
}

fn upload_id_from_uri(uri: &str, prefix: &str) -> Result<String, String> {
    let path = route_path(uri);
    let upload_id = path
        .strip_prefix(prefix)
        .ok_or_else(|| format!("model upload path must start with `{prefix}`"))?;
    if upload_id.trim().is_empty() {
        return Err("model upload identifier must not be empty".to_owned());
    }
    Ok(upload_id.to_owned())
}

fn query_u64(uri: &str, expected_key: &str) -> Result<u64, String> {
    let query = uri
        .split_once('?')
        .map(|(_, query)| query)
        .ok_or_else(|| format!("model upload requests must include `{expected_key}`"))?;

    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key == expected_key {
            return value.parse::<u64>().map_err(|error| {
                format!("model upload query parameter `{expected_key}` is invalid: {error}")
            });
        }
    }

    Err(format!(
        "model upload requests must include `{expected_key}`"
    ))
}

fn validate_hash(hash: &str) -> Result<(), String> {
    let digest = hash
        .strip_prefix("sha256:")
        .ok_or_else(|| format!("model hash `{hash}` must start with `sha256:`"))?;
    if digest.is_empty()
        || !digest
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(format!(
            "model hash `{hash}` must be a hexadecimal sha256 digest"
        ));
    }
    Ok(())
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("failed to open model file `{}`: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];

    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("failed to read model file `{}`: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
}

fn map_error_response(error: String) -> bindings::exports::tachyon::mesh::handler::Response {
    let status = if error.contains("must")
        || error.contains("expected")
        || error.contains("unknown")
        || error.contains("invalid")
        || error.contains("decode")
        || error.contains("exceeds")
    {
        400
    } else {
        500
    };
    response(status, error)
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

fn response_json<T>(
    status: u16,
    payload: &T,
) -> Result<bindings::exports::tachyon::mesh::handler::Response, String>
where
    T: Serialize,
{
    let body = serde_json::to_vec(payload)
        .map_err(|error| format!("failed to serialize response payload: {error}"))?;
    Ok(bindings::exports::tachyon::mesh::handler::Response {
        status,
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body,
        trailers: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_id_extraction_requires_identifier() {
        let error = upload_id_from_uri("/admin/models/upload/", UPLOAD_PREFIX)
            .expect_err("missing upload id should fail");

        assert_eq!(error, "model upload identifier must not be empty");
    }

    #[test]
    fn query_parser_extracts_part_number() {
        let part =
            query_u64("/admin/models/upload/abc?part=4", "part").expect("part query should parse");

        assert_eq!(part, 4);
    }

    #[test]
    fn abort_extracts_upload_id() {
        let upload_id = upload_id_from_uri("/admin/models/abort/some-id", ABORT_PREFIX)
            .expect("abort path should parse");
        assert_eq!(upload_id, "some-id");
    }

    #[test]
    fn cleanup_staging_is_idempotent_on_missing_files() {
        // Calling cleanup against a never-initialized upload must not panic, since
        // it is also invoked from the hash-mismatch error path where the staging
        // file may already be gone (e.g. concurrent gc).
        cleanup_staging("upload-that-never-existed");
    }
}
