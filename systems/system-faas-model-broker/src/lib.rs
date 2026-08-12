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
    sync::{Arc, Mutex, OnceLock},
};
use uuid::Uuid;

const MODEL_CHUNK_BYTES: usize = 16 * 1024 * 1024;
/// Host dispatch sidecar written into each unpacked model directory. Mirrors
/// `core-host`'s `candle_llm_runtime::MODEL_META_JSON` — the broker performs the
/// format *detection* and records the result; the host honours the declared
/// value (and still validates the bytes through the matching loader).
const MODEL_META_JSON: &str = ".tachyon-model.json";
/// GGUF files begin with this ASCII magic.
const GGUF_MAGIC: &[u8; 4] = b"GGUF";
const FORMAT_GGUF: &str = "gguf";
const FORMAT_SAFETENSORS: &str = "safetensors";
const INIT_PATH: &str = "/admin/models/init";
const UPLOAD_PREFIX: &str = "/admin/models/upload/";
const COMMIT_PREFIX: &str = "/admin/models/commit/";
const ABORT_PREFIX: &str = "/admin/models/abort/";
const AUTH_SESSION_CDC_PATH: &str = "/internal/model-broker/cdc/auth-session";
const PROMPT_FINISHED_PATH: &str = "/internal/model-broker/prompt-finished";
const STANDARD_VRAM_TTL_SECONDS: u64 = 300;
const EXTENDED_VRAM_TTL_SECONDS: u64 = 1_800;
const HIGH_FOLLOWUP_PROBABILITY: f32 = 0.8;

/// Serializes the live-directory swap, publication and rollback for one alias.
/// Different aliases remain independent.
static MODEL_COMMIT_LOCKS: OnceLock<Mutex<std::collections::BTreeMap<String, Arc<Mutex<()>>>>> =
    OnceLock::new();

fn commit_lock(alias: &str) -> Result<Arc<Mutex<()>>, String> {
    let locks = MODEL_COMMIT_LOCKS.get_or_init(|| Mutex::new(std::collections::BTreeMap::new()));
    let mut locks = locks
        .lock()
        .map_err(|_| "model commit lock registry was poisoned".to_owned())?;
    Ok(locks
        .entry(alias.to_owned())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone())
}

struct Component;

#[derive(Debug, Deserialize)]
struct InitUploadRequest {
    size_bytes: u64,
    #[serde(default)]
    alias: Option<String>,
    #[serde(default)]
    files: Vec<ModelUploadFileManifest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelUploadFileManifest {
    path: String,
    size_bytes: u64,
    sha256: String,
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
    size_bytes: u64,
    bytes_received: u64,
    last_part: u64,
    #[serde(default)]
    alias: Option<String>,
    #[serde(default)]
    files: Vec<ModelUploadFileManifest>,
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
    validate_file_manifest(&payload.files)?;
    if payload.files.is_empty() {
        return Err("model upload init must include a files manifest".to_owned());
    }
    if payload.size_bytes == 0 {
        return Err("model upload size must be greater than zero".to_owned());
    }
    if let Some(alias) = &payload.alias {
        validate_alias(alias)?;
    }

    let upload_id = Uuid::new_v4().to_string();
    fs::write(staging_path(&upload_id), [])
        .map_err(|error| format!("failed to initialize model staging file: {error}"))?;
    save_pending_upload(
        &upload_id,
        &PendingUpload {
            size_bytes: payload.size_bytes,
            bytes_received: 0,
            last_part: 0,
            alias: payload.alias,
            files: payload.files,
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
            "model upload chunk exceeds the 16 MiB protocol limit ({} bytes)",
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
    if pending.bytes_received == 0 || pending.bytes_received > pending.size_bytes {
        return Err(format!(
            "model upload `{upload_id}` received {} bytes, declared limit {}",
            pending.bytes_received, pending.size_bytes
        ));
    }

    let staging_path = staging_path(&upload_id);
    if !staging_path.exists() {
        return Err(format!(
            "model upload `{upload_id}` staging archive is missing"
        ));
    }

    // The uploaded blob is a gzip+tar archive (single `.gguf` or a safetensors
    // directory). Unpack it into a per-alias model directory that core-host can
    // mmap, detect the on-disk format, and drop the host dispatch sidecar.
    let alias = model_alias(&pending);
    let alias_lock = commit_lock(&alias)?;
    let _alias_guard = alias_lock
        .lock()
        .map_err(|_| format!("model commit lock for `{alias}` was poisoned"))?;
    let model_dir = models_dir().join(&alias);

    // Unpack beside the live directory, never into it. An upload can still be
    // refused after its bytes are on disk — publication rejects an alias a
    // configured binding owns — and when that binding points at this same
    // `models/<alias>` path, unpacking in place has already destroyed the
    // operator's checkpoint by the time the refusal arrives. The registry row
    // and the runtime alias survive that; the files do not, so the next load
    // finds nothing. Staging keeps the live directory intact until the upload
    // is accepted.
    //
    // Keyed by upload id so two uploads in flight cannot share a staging
    // directory, and so a leftover from a crashed attempt belongs to a
    // finished upload rather than blocking this one.
    let incoming_dir = models_dir().join(format!(".incoming-{upload_id}"));
    let backup_dir = models_dir().join(format!(".replaced-{upload_id}"));
    let _ = fs::remove_dir_all(&incoming_dir);
    let _ = fs::remove_dir_all(&backup_dir);
    fs::create_dir_all(&incoming_dir)
        .map_err(|error| format!("failed to create the staging model directory: {error}"))?;

    let format = unpack_targz(&staging_path, &incoming_dir)
        .and_then(|()| validate_extracted_file_manifest(&incoming_dir, &pending.files))
        .and_then(|()| detect_format(&incoming_dir))
        .and_then(|format| write_meta_sidecar(&incoming_dir, format, &alias).map(|()| format))
        .inspect_err(|_error| {
            // Nothing outside the staging directory has been touched yet, so a
            // failure here costs the live checkpoint nothing.
            let _ = fs::remove_dir_all(&incoming_dir);
            cleanup_staging(&upload_id);
        })?;

    // Move the previous checkpoint aside rather than deleting it, so a refused
    // publication can put it back exactly as it was.
    let replaced = model_dir.exists();
    if replaced {
        fs::rename(&model_dir, &backup_dir).map_err(|error| {
            let _ = fs::remove_dir_all(&incoming_dir);
            cleanup_staging(&upload_id);
            format!(
                "failed to set aside the existing model `{}`: {error}",
                model_dir.display()
            )
        })?;
    }
    if let Err(error) = fs::rename(&incoming_dir, &model_dir) {
        let restore = replaced.then(|| fs::rename(&backup_dir, &model_dir));
        let _ = fs::remove_dir_all(&incoming_dir);
        cleanup_staging(&upload_id);
        if let Some(Err(restore_error)) = restore {
            return Err(format!(
                "failed to install the uploaded model at `{}`: {error}; restoring the previous checkpoint failed: {restore_error}",
                model_dir.display()
            ));
        }
        return Err(format!(
            "failed to install the uploaded model at `{}`: {error}",
            model_dir.display()
        ));
    }

    // Published last, because this is the step that can still refuse the
    // upload. On refusal the new files go and the previous checkpoint comes
    // back, leaving the alias exactly as the manifest left it.
    if let Err(error) = publish_model_uploaded(&alias, format, &model_dir, &pending.files) {
        let rollback = fs::remove_dir_all(&model_dir).and_then(|()| {
            if replaced {
                fs::rename(&backup_dir, &model_dir)
            } else {
                Ok(())
            }
        });
        cleanup_staging(&upload_id);
        if let Err(rollback_error) = rollback {
            return Err(format!("{error}; rollback failed: {rollback_error}"));
        }
        return Err(error);
    }

    if replaced {
        if let Err(error) = fs::remove_dir_all(&backup_dir) {
            // Publication is already durable. The upload must no longer stay
            // retryable merely because GC of its obsolete backup failed.
            cleanup_staging(&upload_id);
            return Err(format!(
                "model was published but the replaced checkpoint at `{}` could not be removed: {error}",
                backup_dir.display()
            ));
        }
    }
    cleanup_staging(&upload_id);

    Ok(model_dir.to_string_lossy().to_string())
}

/// The model directory / registry name: the caller-supplied alias, or the bare
/// hash digest when none was given. Aliases are validated at `init` time, so
/// this is always a safe single path component.
fn model_alias(pending: &PendingUpload) -> String {
    pending.alias.clone().unwrap_or_else(|| {
        pending
            .files
            .first()
            .map(|file| file.sha256.trim_start_matches("sha256:").to_owned())
            .unwrap_or_else(|| "model".to_owned())
    })
}

fn validate_file_manifest(files: &[ModelUploadFileManifest]) -> Result<(), String> {
    let mut seen = std::collections::BTreeSet::new();
    for file in files {
        validate_hash(&file.sha256)?;
        let clean = sanitize_relative(Path::new(&file.path))?;
        let clean_path = path_to_manifest_key(&clean);
        if clean_path != file.path {
            return Err(format!(
                "model file manifest path `{}` must be normalized as `{clean_path}`",
                file.path
            ));
        }
        if !seen.insert(file.path.clone()) {
            return Err(format!(
                "model file manifest contains duplicate path `{}`",
                file.path
            ));
        }
    }
    Ok(())
}

fn validate_extracted_file_manifest(
    model_dir: &Path,
    manifest: &[ModelUploadFileManifest],
) -> Result<(), String> {
    if manifest.is_empty() {
        return Ok(());
    }

    let expected: std::collections::BTreeMap<String, &ModelUploadFileManifest> = manifest
        .iter()
        .map(|file| (file.path.clone(), file))
        .collect();
    let actual = collect_regular_files(model_dir, model_dir)?;
    for path in actual.keys() {
        if !expected.contains_key(path) {
            return Err(format!(
                "model archive contains unexpected file `{path}` not declared in manifest"
            ));
        }
    }
    for (path, file) in expected {
        let Some(actual_path) = actual.get(&path) else {
            return Err(format!("model archive is missing declared file `{path}`"));
        };
        let metadata = fs::metadata(actual_path)
            .map_err(|error| format!("failed to inspect extracted file `{path}`: {error}"))?;
        if metadata.len() != file.size_bytes {
            return Err(format!(
                "model file `{path}` size mismatch: expected {}, found {}",
                file.size_bytes,
                metadata.len()
            ));
        }
        let computed_hash = hash_file(actual_path)?;
        if computed_hash != file.sha256 {
            return Err(format!(
                "model file `{path}` hash mismatch: expected `{}`, computed `{computed_hash}`",
                file.sha256
            ));
        }
    }
    Ok(())
}

fn collect_regular_files(
    root: &Path,
    dir: &Path,
) -> Result<std::collections::BTreeMap<String, PathBuf>, String> {
    let mut files = std::collections::BTreeMap::new();
    for entry in
        fs::read_dir(dir).map_err(|error| format!("failed to scan `{}`: {error}", dir.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        let path = entry.path();
        let metadata = entry
            .metadata()
            .map_err(|error| format!("failed to inspect `{}`: {error}", path.display()))?;
        if metadata.is_dir() {
            files.extend(collect_regular_files(root, &path)?);
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|error| format!("failed to relativize `{}`: {error}", path.display()))?;
            files.insert(path_to_manifest_key(relative), path);
        }
    }
    Ok(files)
}

fn path_to_manifest_key(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(part) => Some(part.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn publish_model_uploaded(
    alias: &str,
    engine: &str,
    model_dir: &Path,
    files: &[ModelUploadFileManifest],
) -> Result<(), String> {
    let event = bindings::tachyon::mesh::model_events::ModelUploaded {
        alias: alias.to_owned(),
        engine: engine.to_owned(),
        model_path: model_dir.to_string_lossy().into_owned(),
        files: files
            .iter()
            .map(|file| bindings::tachyon::mesh::model_events::ModelFile {
                path: file.path.clone(),
                size_bytes: file.size_bytes,
                sha256: file.sha256.clone(),
            })
            .collect(),
    };
    bindings::tachyon::mesh::model_events::publish_model_uploaded(&event)
        .map_err(|error| format!("failed to publish model upload event: {error}"))
}

/// Stream a gzip+tar archive into `dest`, writing only regular files and
/// directories. Extraction is manual (no permission/mtime/xattr restoration) so
/// it stays within the WASI filesystem surface, and every entry path is
/// sanitised to keep writes inside `dest` (defence against `../` tar slips).
fn unpack_targz(archive_path: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(archive_path)
        .map_err(|error| format!("failed to open uploaded archive: {error}"))?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|error| format!("uploaded archive is not a valid tar stream: {error}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|error| format!("failed to read archive entry: {error}"))?;
        let raw_path = entry
            .path()
            .map_err(|error| format!("archive entry has an invalid path: {error}"))?
            .into_owned();
        let relative = sanitize_relative(&raw_path)?;
        let out_path = dest.join(&relative);
        if entry.header().entry_type().is_dir() {
            fs::create_dir_all(&out_path)
                .map_err(|error| format!("failed to create archive directory: {error}"))?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create archive parent: {error}"))?;
        }
        let mut out = fs::File::create(&out_path)
            .map_err(|error| format!("failed to create extracted file: {error}"))?;
        std::io::copy(&mut entry, &mut out)
            .map_err(|error| format!("failed to write extracted file: {error}"))?;
    }
    Ok(())
}

/// Reduce an archive entry path to a safe relative path, rejecting absolute
/// paths and any `..` component so extraction can never escape the target dir.
fn sanitize_relative(path: &Path) -> Result<PathBuf, String> {
    use std::path::Component;
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            _ => {
                return Err(format!(
                    "model archive contains an unsafe path `{}`",
                    path.display()
                ))
            }
        }
    }
    if clean.as_os_str().is_empty() {
        return Err("model archive contains an empty path entry".to_owned());
    }
    Ok(clean)
}

/// Detect the on-disk format of an unpacked model directory by content: a file
/// starting with the GGUF magic wins; otherwise a `config.json` next to a
/// `.safetensors` file marks a Hugging Face safetensors checkpoint.
fn detect_format(dir: &Path) -> Result<&'static str, String> {
    let mut has_config = false;
    let mut has_safetensors = false;
    let read = fs::read_dir(dir)
        .map_err(|error| format!("failed to scan unpacked model directory: {error}"))?;
    for entry in read.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if file_starts_with_gguf_magic(&path) {
            return Ok(FORMAT_GGUF);
        }
        match path.file_name().and_then(|name| name.to_str()) {
            Some("config.json") => has_config = true,
            Some(name) if name.ends_with(".safetensors") => has_safetensors = true,
            _ => {}
        }
    }
    if has_config && has_safetensors {
        Ok(FORMAT_SAFETENSORS)
    } else {
        Err(
            "uploaded model archive contains neither a GGUF file nor a safetensors checkpoint \
             (config.json + .safetensors)"
                .to_owned(),
        )
    }
}

/// Cheap content probe: does the file begin with the 4-byte GGUF magic?
fn file_starts_with_gguf_magic(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut magic = [0_u8; 4];
    file.read_exact(&mut magic).is_ok() && &magic == GGUF_MAGIC
}

/// Write the host dispatch sidecar declaring the detected format.
fn write_meta_sidecar(dir: &Path, format: &str, alias: &str) -> Result<(), String> {
    let path = dir.join(MODEL_META_JSON);
    let tool_call_parser = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| {
            value
                .get("tool_call_parser")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        });
    let mut metadata = serde_json::json!({ "format": format, "alias": alias });
    if let Some(parser) = tool_call_parser {
        metadata["tool_call_parser"] = serde_json::Value::String(parser);
    }
    let body = serde_json::to_vec(&metadata)
        .map_err(|error| format!("failed to encode model metadata sidecar: {error}"))?;
    fs::write(path, body)
        .map_err(|error| format!("failed to write model metadata sidecar: {error}"))
}

/// Reject aliases that are not a single safe path component.
fn validate_alias(alias: &str) -> Result<(), String> {
    if alias.is_empty()
        || alias.contains('/')
        || alias.contains('\\')
        || alias.contains("..")
        || alias.starts_with('.')
    {
        return Err(format!(
            "model alias `{alias}` must be a non-empty name without path separators, `..`, or a leading dot"
        ));
    }
    Ok(())
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
    fn model_upload_accepts_large_local_chunks() {
        assert_eq!(MODEL_CHUNK_BYTES, 16 * 1024 * 1024);
    }

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

    fn unique_tmp(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("tachyon-broker-{tag}-{}", Uuid::new_v4()))
    }

    fn build_targz(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        {
            let mut builder = tar::Builder::new(&mut encoder);
            for (name, data) in files {
                let mut header = tar::Header::new_gnu();
                header.set_size(data.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                builder
                    .append_data(&mut header, name, &data[..])
                    .expect("append archive entry");
            }
            builder.finish().expect("finish tar");
        }
        encoder.finish().expect("finish gzip")
    }

    #[test]
    fn validate_alias_rejects_unsafe_names() {
        assert!(validate_alias("tinyllama-1.1b").is_ok());
        assert!(validate_alias("").is_err());
        assert!(validate_alias("../evil").is_err());
        assert!(validate_alias("a/b").is_err());
        assert!(validate_alias(".hidden").is_err());
    }

    #[test]
    fn sanitize_relative_blocks_traversal_and_absolute_paths() {
        assert!(sanitize_relative(Path::new("../escape")).is_err());
        assert!(sanitize_relative(Path::new("/abs/path")).is_err());
        assert_eq!(
            sanitize_relative(Path::new("./nested/model.gguf")).expect("safe path"),
            PathBuf::from("nested/model.gguf")
        );
    }

    #[test]
    fn unpack_then_detect_gguf_archive() {
        let tmp = unique_tmp("gguf");
        fs::create_dir_all(&tmp).expect("tmp dir");
        let archive = build_targz(&[
            ("model.gguf", b"GGUF\x00\x00\x00\x00body".to_vec()),
            ("tokenizer.json", b"{}".to_vec()),
        ]);
        let staging = tmp.join("upload.part");
        fs::write(&staging, &archive).expect("write archive");
        let dest = tmp.join("model");
        fs::create_dir_all(&dest).expect("dest dir");

        unpack_targz(&staging, &dest).expect("archive should unpack");
        assert!(dest.join("model.gguf").exists());
        assert!(dest.join("tokenizer.json").exists());
        assert_eq!(detect_format(&dest).expect("format"), FORMAT_GGUF);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn extracted_file_manifest_accepts_matching_files() {
        let tmp = unique_tmp("manifest-ok");
        fs::create_dir_all(&tmp).expect("tmp dir");
        let model = tmp.join("model.gguf");
        fs::write(&model, b"GGUFmodel").expect("model");
        let tokenizer = tmp.join("tokenizer.json");
        fs::write(&tokenizer, b"{}").expect("tokenizer");
        let manifest = vec![
            ModelUploadFileManifest {
                path: "model.gguf".to_owned(),
                size_bytes: 9,
                sha256: hash_file(&model).expect("hash model"),
            },
            ModelUploadFileManifest {
                path: "tokenizer.json".to_owned(),
                size_bytes: 2,
                sha256: hash_file(&tokenizer).expect("hash tokenizer"),
            },
        ];

        validate_extracted_file_manifest(&tmp, &manifest).expect("manifest should match");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn extracted_file_manifest_rejects_hash_mismatch() {
        let tmp = unique_tmp("manifest-bad");
        fs::create_dir_all(&tmp).expect("tmp dir");
        fs::write(tmp.join("model.gguf"), b"GGUFmodel").expect("model");
        let manifest = vec![ModelUploadFileManifest {
            path: "model.gguf".to_owned(),
            size_bytes: 9,
            sha256: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .to_owned(),
        }];

        let error = validate_extracted_file_manifest(&tmp, &manifest)
            .expect_err("hash mismatch should fail");
        assert!(error.contains("hash mismatch"));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn detect_format_identifies_safetensors_directory() {
        let tmp = unique_tmp("safetensors");
        fs::create_dir_all(&tmp).expect("tmp dir");
        fs::write(tmp.join("config.json"), br#"{"model_type":"llama"}"#).expect("config");
        fs::write(tmp.join("model.safetensors"), b"\x00\x00").expect("weights");
        fs::write(tmp.join("tokenizer.json"), b"{}").expect("tokenizer");

        assert_eq!(detect_format(&tmp).expect("format"), FORMAT_SAFETENSORS);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn detect_format_rejects_an_unrecognized_archive() {
        let tmp = unique_tmp("junk");
        fs::create_dir_all(&tmp).expect("tmp dir");
        fs::write(tmp.join("README.txt"), b"not a model").expect("file");
        assert!(detect_format(&tmp).is_err());
        let _ = fs::remove_dir_all(&tmp);
    }
}
