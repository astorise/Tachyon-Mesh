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
const AUTH_SESSION_CDC_PATH: &str = "/internal/model-broker/cdc/auth-session";
const PROMPT_FINISHED_PATH: &str = "/internal/model-broker/prompt-finished";
const MODEL_REGISTRY_REGISTER_URL: &str = "http://mesh/internal/guest-openai/register";
const STANDARD_VRAM_TTL_SECONDS: u64 = 300;
const EXTENDED_VRAM_TTL_SECONDS: u64 = 1_800;
const HIGH_FOLLOWUP_PROBABILITY: f32 = 0.8;

struct Component;

#[derive(Debug, Deserialize)]
struct InitUploadRequest {
    expected_hash: String,
    size_bytes: u64,
    #[serde(default)]
    alias: Option<String>,
}

#[derive(Debug, Serialize)]
struct InitUploadResponse {
    upload_id: String,
}

#[derive(Debug, Serialize)]
struct CommitUploadResponse {
    model_path: String,
}

#[derive(Debug, Deserialize)]
struct CdcMutationEvent {
    namespace: String,
    #[allow(dead_code)]
    key: String,
    op: String,
    #[serde(default, alias = "new-value", alias = "newValue")]
    new_value: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, PartialEq)]
struct PrewarmInstruction {
    model: String,
    layer_index: u32,
    priority: &'static str,
}

#[derive(Debug, Serialize)]
struct PrewarmResponse {
    prewarm: Option<PrewarmInstruction>,
}

#[derive(Debug, Deserialize)]
struct PromptFinishedRequest {
    tenant_id: String,
    #[serde(default)]
    historical_prompts_at_hour: u32,
    #[serde(default)]
    observed_days: u32,
    #[serde(default)]
    probability: Option<f32>,
}

#[derive(Debug, Serialize)]
struct PromptTtlResponse {
    tenant_id: String,
    probability: f32,
    ttl_seconds: u64,
    priority: &'static str,
}

#[derive(Debug, Serialize, Deserialize)]
struct PendingUpload {
    expected_hash: String,
    size_bytes: u64,
    bytes_received: u64,
    last_part: u64,
    #[serde(default)]
    alias: Option<String>,
}

#[derive(Debug, Serialize)]
struct ModelRegistryEntry {
    alias: String,
    engine: String,
    #[serde(rename = "vramRequiredMb")]
    vram_required_mb: u64,
    status: String,
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        let result = if req.method.eq_ignore_ascii_case("POST")
            && route_path(&req.uri) == AUTH_SESSION_CDC_PATH
        {
            handle_auth_session_cdc(&req.body)
        } else if req.method.eq_ignore_ascii_case("POST")
            && route_path(&req.uri) == PROMPT_FINISHED_PATH
        {
            handle_prompt_finished(&req.body)
        } else if req.method.eq_ignore_ascii_case("POST") && route_path(&req.uri) == INIT_PATH {
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

fn handle_auth_session_cdc(
    body: &[u8],
) -> Result<bindings::exports::tachyon::mesh::handler::Response, String> {
    let event: CdcMutationEvent = serde_json::from_slice(body)
        .map_err(|error| format!("failed to decode auth session CDC event: {error}"))?;
    let prewarm = jit_prewarm_from_event(&event);
    let status = if prewarm.is_some() { 202 } else { 204 };
    response_json(status, &PrewarmResponse { prewarm })
}

fn handle_prompt_finished(
    body: &[u8],
) -> Result<bindings::exports::tachyon::mesh::handler::Response, String> {
    let request: PromptFinishedRequest = serde_json::from_slice(body)
        .map_err(|error| format!("failed to decode prompt-finished payload: {error}"))?;
    if request.tenant_id.trim().is_empty() {
        return Err("prompt-finished payload must include a tenant_id".to_owned());
    }

    let probability = followup_probability(&request);
    response_json(
        200,
        &PromptTtlResponse {
            tenant_id: request.tenant_id,
            probability,
            ttl_seconds: calculate_dynamic_ttl_seconds(probability),
            priority: "volatile",
        },
    )
}

fn jit_prewarm_from_event(event: &CdcMutationEvent) -> Option<PrewarmInstruction> {
    if !event.namespace.contains("auth") {
        return None;
    }
    if !event.op.eq_ignore_ascii_case("insert")
        && !event.op.eq_ignore_ascii_case("session_started")
        && !event.op.eq_ignore_ascii_case("session-issued")
    {
        return None;
    }

    let tenant_id = tenant_id_from_event(event)?;
    Some(PrewarmInstruction {
        model: resolve_tenant_adapter(&tenant_id),
        layer_index: 0,
        priority: "volatile",
    })
}

fn tenant_id_from_event(event: &CdcMutationEvent) -> Option<String> {
    let value = event.new_value.as_ref()?;
    tenant_id_from_value(value).or_else(|| {
        value
            .as_str()
            .and_then(|encoded| serde_json::from_str::<serde_json::Value>(encoded).ok())
            .and_then(|decoded| tenant_id_from_value(&decoded))
    })
}

fn tenant_id_from_value(value: &serde_json::Value) -> Option<String> {
    ["tenant_id", "tenantId", "x-tenant-id"]
        .iter()
        .find_map(|key| value.get(key)?.as_str())
        .map(str::trim)
        .filter(|tenant| !tenant.is_empty())
        .map(str::to_owned)
}

fn resolve_tenant_adapter(tenant_id: &str) -> String {
    format!("lora:{tenant_id}:default")
}

fn followup_probability(request: &PromptFinishedRequest) -> f32 {
    if let Some(probability) = request.probability {
        return probability.clamp(0.0, 1.0);
    }

    let observed_days = request.observed_days.max(1) as f32;
    (request.historical_prompts_at_hour as f32 / observed_days).clamp(0.0, 1.0)
}

fn calculate_dynamic_ttl_seconds(probability: f32) -> u64 {
    if probability > HIGH_FOLLOWUP_PROBABILITY {
        EXTENDED_VRAM_TTL_SECONDS
    } else {
        STANDARD_VRAM_TTL_SECONDS
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
            alias: payload.alias,
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

    let alias = pending.alias.unwrap_or_else(|| {
        pending
            .expected_hash
            .trim_start_matches("sha256:")
            .to_owned()
    });
    notify_model_registry(&alias);

    Ok(model_path.to_string_lossy().to_string())
}

/// Best-effort notification to the `guest-openai` model registry that a model is now
/// available. Failures are swallowed — the commit has already succeeded and the registry
/// can be refreshed on the next model upload or operator intervention.
fn notify_model_registry(alias: &str) {
    let entry = ModelRegistryEntry {
        alias: alias.to_owned(),
        engine: "gguf".to_owned(),
        vram_required_mb: 0,
        status: "available".to_owned(),
    };
    let Ok(body) = serde_json::to_vec(&entry) else {
        return;
    };
    let _ = bindings::tachyon::mesh::outbound_http::send_request(
        "POST",
        MODEL_REGISTRY_REGISTER_URL,
        &[("content-type".to_owned(), "application/json".to_owned())],
        &body,
    );
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

    #[test]
    fn auth_session_event_generates_volatile_prewarm_instruction() {
        let event = CdcMutationEvent {
            namespace: "auth:sessions".to_owned(),
            key: "session-1".to_owned(),
            op: "insert".to_owned(),
            new_value: Some(serde_json::json!({
                "tenant_id": "tenant-a",
                "subject": "user-1"
            })),
        };

        let instruction = jit_prewarm_from_event(&event).expect("session should prewarm LoRA");

        assert_eq!(
            instruction,
            PrewarmInstruction {
                model: "lora:tenant-a:default".to_owned(),
                layer_index: 0,
                priority: "volatile",
            }
        );
    }

    #[test]
    fn non_auth_cdc_events_do_not_prewarm() {
        let event = CdcMutationEvent {
            namespace: "billing:invoices".to_owned(),
            key: "invoice-1".to_owned(),
            op: "insert".to_owned(),
            new_value: Some(serde_json::json!({ "tenant_id": "tenant-a" })),
        };

        assert!(jit_prewarm_from_event(&event).is_none());
    }

    #[test]
    fn dynamic_ttl_extends_when_followup_probability_is_high() {
        assert_eq!(
            calculate_dynamic_ttl_seconds(0.81),
            EXTENDED_VRAM_TTL_SECONDS
        );
        assert_eq!(
            calculate_dynamic_ttl_seconds(0.8),
            STANDARD_VRAM_TTL_SECONDS
        );
    }

    #[test]
    fn followup_probability_uses_timeseries_hour_density() {
        let request = PromptFinishedRequest {
            tenant_id: "tenant-a".to_owned(),
            historical_prompts_at_hour: 24,
            observed_days: 30,
            probability: None,
        };

        assert_eq!(followup_probability(&request), 0.8);
    }
}
