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
    reclaim_discarded_backups();
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
    // Held from before the first rename until after publication or rollback,
    // so the ownership check below and the deletion it guards cannot be split
    // by a second commit for this alias. Released on drop, including on every
    // early return between here and the end of the function.
    let _commit_lock = AliasCommitLock::acquire(&alias)?;
    reconcile_interrupted_commit(&model_dir, &backup_dir, &upload_id)?;
    fs::create_dir_all(&incoming_dir)
        .map_err(|error| format!("failed to create the staging model directory: {error}"))?;

    let format = unpack_targz(&staging_path, &incoming_dir)
        .and_then(|()| validate_extracted_file_manifest(&incoming_dir, &pending.files))
        .and_then(|()| detect_format(&incoming_dir))
        .and_then(|format| {
            write_meta_sidecar(&incoming_dir, format, &alias, &upload_id).map(|()| format)
        })
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
        if replaced {
            let _ = fs::rename(&backup_dir, &model_dir);
        }
        let _ = fs::remove_dir_all(&incoming_dir);
        cleanup_staging(&upload_id);
        return Err(format!(
            "failed to install the uploaded model at `{}`: {error}",
            model_dir.display()
        ));
    }

    // Published last, because this is the step that can still refuse the
    // upload. On refusal the new files go and the previous checkpoint comes
    // back, leaving the alias exactly as the manifest left it.
    if let Err(error) = publish_model_uploaded(&alias, format, &model_dir, &pending.files) {
        // Unwind only what is still ours. Two commits for the same alias can
        // interleave: by the time this refusal arrives, a second upload may
        // have moved our directory aside and installed its own. Removing the
        // live directory blindly would delete a checkpoint its caller was told
        // had installed, and dropping our backup on top would lose that one
        // too — the sequence ends with both callers holding a failure and
        // neither model live.
        if installed_upload_id(&model_dir).as_deref() == Some(upload_id.as_str()) {
            let _ = fs::remove_dir_all(&model_dir);
            if replaced {
                let _ = fs::rename(&backup_dir, &model_dir);
            }
            cleanup_staging(&upload_id);
            return Err(error);
        }
        // Someone else owns the live path. Deleting is the irreversible half
        // of a rollback and restoring is the destructive one, so do neither:
        // leave the backup on disk and say where it is. An operator can put it
        // back; nothing here can put it back correctly.
        cleanup_staging(&upload_id);
        if replaced {
            return Err(format!(
                "{error} (a concurrent upload now owns `{}`; this upload's previous checkpoint \
                 was left at `{}` rather than restored over it)",
                model_dir.display(),
                backup_dir.display()
            ));
        }
        return Err(error);
    }

    // Publication succeeded, so the backup is dead weight — and *which name it
    // carries* is the durable record of that fact.
    //
    // Renamed before it is deleted, rather than deleted with a rename as the
    // fallback. A crash in this window used to leave a `.replaced-` directory
    // beside an already-published upload, and the retry could not tell that
    // from a crash *before* publication: it rewound the live path to the
    // predecessor, so a request arriving during the re-unpack could load the
    // old files under the newly published alias, and a retry that then failed
    // left the registry describing a checkpoint that was no longer there. The
    // two states now have two different names.
    //
    // It also has to be a separate name from `.replaced-` for the older
    // reason: the concurrent-upload path above deliberately leaves one behind
    // for an operator to restore by hand, and a sweep that could not tell the
    // two apart would delete exactly the one kept on purpose.
    if replaced {
        let discarded = models_dir().join(format!("{DISCARDED_BACKUP_PREFIX}{upload_id}"));
        match fs::rename(&backup_dir, &discarded) {
            Ok(()) => {
                let _ = fs::remove_dir_all(&discarded);
            }
            // The rename is a single directory entry within one directory, so
            // a failure here is unusual. Fall back to removing in place; the
            // window it reopens is the one this rename exists to close, and
            // leaving the directory would strand a whole checkpoint.
            Err(_) => {
                let _ = fs::remove_dir_all(&backup_dir);
            }
        }
    }
    cleanup_staging(&upload_id);

    Ok(model_dir.to_string_lossy().to_string())
}

/// Directory prefix for a replaced checkpoint whose removal failed after its
/// replacement went live. Leading dot, like the other internal names, so
/// `validate_alias` can never let an upload claim one.
const DISCARDED_BACKUP_PREFIX: &str = ".discarded-";

/// Delete backups an earlier commit finished with but could not remove.
///
/// Nothing is renamed into this namespace until its upload has fully succeeded
/// and something else owns the live path, so a sweep here cannot take a
/// checkpoint anyone is still counting on.
fn reclaim_discarded_backups() {
    let Ok(entries) = fs::read_dir(models_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(DISCARDED_BACKUP_PREFIX))
        {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// How long a commit lock may sit untouched before a later commit takes it.
///
/// Generous on purpose: a commit unpacks and hashes a multi-gigabyte archive
/// while holding this, so a limit tuned for a small upload would let a slow but
/// healthy one be stolen from underneath itself. The only thing this bounds is
/// how long an alias stays wedged after a crash.
const ALIAS_LOCK_STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// Serializes the install / publish / rollback sequence for one alias.
///
/// Every step of that sequence is atomic on its own, and the sequence is not:
/// the rollback checks that the live directory still carries this upload's id
/// and *then* deletes it. Between those two operations a second commit for the
/// same alias can rename ours aside and install its own — so the rollback
/// deleted a checkpoint whose caller had been told it installed, restored a
/// predecessor over it, and the second commit went on to publish successfully
/// for files that no longer existed. No arrangement of individually-atomic
/// filesystem calls closes that; only holding the alias does.
///
/// The lock is a file created with `create_new`, which is atomic, and released
/// on drop — including on every `?` and every early return in the commit path.
#[derive(Debug)]
struct AliasCommitLock {
    path: PathBuf,
}

impl AliasCommitLock {
    fn acquire(alias: &str) -> Result<Self, String> {
        let path = models_dir().join(format!(".commit-{alias}.lock"));
        for _ in 0..2 {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    // Held. Either by a commit still running — in which case
                    // refusing is the whole point — or by one that died, in
                    // which case the alias must not stay wedged forever.
                    let held_for = fs::metadata(&path)
                        .and_then(|meta| meta.modified())
                        .ok()
                        .and_then(|at| std::time::SystemTime::now().duration_since(at).ok());
                    if !held_for.is_some_and(|age| age >= ALIAS_LOCK_STALE_AFTER) {
                        return Err(format!(
                            "another commit for model `{alias}` is in progress; retry once it \
                             finishes"
                        ));
                    }
                    // Taken over rather than removed and assumed: the retry
                    // below goes through `create_new` again, so two processes
                    // reclaiming the same stale lock cannot both win.
                    let _ = fs::remove_file(&path);
                }
                Err(error) => {
                    return Err(format!(
                        "failed to take the commit lock for model `{alias}`: {error}"
                    ))
                }
            }
        }
        Err(format!(
            "could not take the commit lock for model `{alias}`: it was reclaimed by another \
             commit"
        ))
    }
}

impl Drop for AliasCommitLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Put the live path back the way a crashed attempt found it.
///
/// A commit renames twice — live checkpoint to `.replaced-{upload_id}`, then
/// the staged upload onto the live path — and a crash between or just after
/// them leaves a backup behind. Three states are distinguishable when a retry
/// arrives, and each needs a different answer:
///
/// - **Nothing on the live path.** The crash landed between the renames. Put
///   the backup back. This one was already handled: opening by *deleting* the
///   backup meant a retry that was then refused had nothing to restore, and
///   the operator's checkpoint was gone for good.
/// - **Our own files on the live path.** The crash landed after the second
///   rename, before publication accepted or refused them. Both directories
///   exist, so the case above does not fire — and deleting the backup here
///   discarded the operator's checkpoint while leaving an *unpublished* upload
///   live. A refusal on the retry could then only roll back to the rejected
///   upload. Our half-installed files are the disposable half: the staging
///   archive still holds them and the retry re-unpacks them anyway, so rewind
///   to the state the first attempt started from and let it redo the sequence.
/// - **Somebody else's files on the live path.** What the concurrent-upload
///   path deliberately parks for an operator, and what a crash another upload
///   then committed over also leaves. Neither directory is safe to touch:
///   deleting the backup loses the checkpoint it was kept for, restoring it
///   destroys a model somebody was told had installed. Refusing names the two
///   paths that need a decision.
///
/// Past this point the live directory is either whole or absent, which is what
/// the rest of the commit assumes.
fn reconcile_interrupted_commit(
    model_dir: &Path,
    backup_dir: &Path,
    upload_id: &str,
) -> Result<(), String> {
    if !backup_dir.exists() {
        return Ok(());
    }
    if model_dir.exists() {
        if installed_upload_id(model_dir).as_deref() != Some(upload_id) {
            return Err(format!(
                "model upload `{upload_id}` cannot commit: an earlier attempt's checkpoint is \
                 parked at `{}` while `{}` is owned by another upload; restore or remove the \
                 parked directory before retrying",
                backup_dir.display(),
                model_dir.display()
            ));
        }
        let _ = fs::remove_dir_all(model_dir);
    }
    fs::rename(backup_dir, model_dir).map_err(|error| {
        format!(
            "failed to restore the checkpoint an interrupted upload left in `{}`: {error}",
            backup_dir.display()
        )
    })?;
    let _ = fs::remove_dir_all(backup_dir);
    Ok(())
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
fn write_meta_sidecar(
    dir: &Path,
    format: &str,
    alias: &str,
    upload_id: &str,
) -> Result<(), String> {
    // Carried over from the archive's own sidecar, if it brought one. This is
    // the documented escape hatch for a checkpoint whose chat template does not
    // match how it was actually fine-tuned to emit calls — a Qwen Coder build
    // whose template says `<tool_call>` while the model speaks a different
    // dialect, say. Rewriting the sidecar from scratch threw the uploader's
    // declaration away, and the loss is silent: tool calls simply come back as
    // prose, which reads like the model choosing not to call anything.
    //
    // The *value* is relayed unvalidated — the host checks it against the
    // dialects the guest implements and warns on anything else, and a second
    // allowlist here would be one more place to forget a new one. Its *type* is
    // not: the host reads this file into `ModelMeta { tool_call_parser:
    // Option<String> }`, so a non-string fails that deserialization and takes
    // the whole sidecar with it — `format` and `alias` included. The upload
    // would commit, publish, and then be unloadable, which is a worse outcome
    // than losing one optional hint.
    //
    // Dropped rather than refused: the sidecar is an escape hatch, and failing
    // an otherwise valid checkpoint over a malformed optional field would trade
    // a silent loss for a loud one nobody asked for. The host's own warning
    // still fires for a string naming an unknown dialect.
    let declared_parser = fs::read(dir.join(MODEL_META_JSON))
        .ok()
        .and_then(|raw| serde_json::from_slice::<serde_json::Value>(&raw).ok())
        .and_then(|meta| meta.get("tool_call_parser").cloned())
        .filter(|parser| parser.is_string());
    let mut body = serde_json::json!({
        "format": format,
        "alias": alias,
        // Who installed this directory. The host ignores unknown sidecar keys,
        // so this costs it nothing and buys the rollback below the one fact it
        // cannot otherwise have: whether the checkpoint sitting at the live
        // path is still the one this upload put there.
        "upload_id": upload_id,
    });
    if let (Some(parser), Some(object)) = (declared_parser, body.as_object_mut()) {
        object.insert("tool_call_parser".to_owned(), parser);
    }
    let body = serde_json::to_vec(&body)
        .map_err(|error| format!("failed to encode model metadata sidecar: {error}"))?;
    fs::write(dir.join(MODEL_META_JSON), body)
        .map_err(|error| format!("failed to write model metadata sidecar: {error}"))
}

/// The upload that installed the checkpoint at `dir`, when it says.
///
/// `None` covers a directory with no sidecar, an unreadable one, and one
/// written before this key existed — all of which mean the same thing to the
/// only caller: do not assume this directory is yours.
fn installed_upload_id(dir: &Path) -> Option<String> {
    let raw = fs::read(dir.join(MODEL_META_JSON)).ok()?;
    let meta: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    meta.get("upload_id")?.as_str().map(str::to_owned)
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

    /// An uploader's tool-call dialect survives the sidecar rewrite.
    ///
    /// It is the documented escape hatch for a checkpoint whose chat template
    /// does not match how it was fine-tuned to emit calls, and rewriting the
    /// sidecar from scratch threw it away. The loss is silent: calls come back
    /// as prose, which reads exactly like a model choosing not to call
    /// anything.
    #[test]
    fn an_uploaded_tool_call_parser_survives_the_sidecar_rewrite() {
        let dir = std::env::temp_dir().join(format!(
            "tachyon-broker-sidecar-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after the epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("fixture dir");

        // The archive brought its own declaration.
        fs::write(
            dir.join(MODEL_META_JSON),
            br#"{"format":"gguf","tool_call_parser":"qwen_coder"}"#,
        )
        .expect("uploaded sidecar");

        write_meta_sidecar(&dir, "gguf", "coder", "up-1").expect("sidecar rewrite");
        let raw = fs::read(dir.join(MODEL_META_JSON)).expect("read back");
        let meta: serde_json::Value = serde_json::from_slice(&raw).expect("valid JSON");
        assert_eq!(meta["tool_call_parser"], "qwen_coder");
        // The broker's own fields still win: they describe where the files
        // actually landed.
        assert_eq!(meta["alias"], "coder");
        assert_eq!(meta["upload_id"], "up-1");

        // A non-string declaration is dropped rather than relayed. The host
        // reads this file into `ModelMeta { tool_call_parser: Option<String> }`,
        // so relaying an object failed that deserialization and took the whole
        // sidecar with it — `format` and `alias` included — leaving an upload
        // that committed, published, and could not load.
        fs::write(
            dir.join(MODEL_META_JSON),
            br#"{"format":"gguf","tool_call_parser":{"dialect":"qwen"}}"#,
        )
        .expect("uploaded sidecar");
        write_meta_sidecar(&dir, "gguf", "coder", "up-3").expect("sidecar rewrite");
        let raw = fs::read(dir.join(MODEL_META_JSON)).expect("read back");
        let meta: serde_json::Value = serde_json::from_slice(&raw).expect("valid JSON");
        assert!(
            meta.get("tool_call_parser").is_none(),
            "an unusable hint is worth less than a loadable checkpoint: {meta}"
        );
        assert_eq!(meta["format"], "gguf");
        assert_eq!(meta["alias"], "coder");

        // An archive with no declaration gets no key invented for it, so the
        // host falls back to reading the chat template as it always did.
        fs::remove_file(dir.join(MODEL_META_JSON)).expect("clear");
        write_meta_sidecar(&dir, "gguf", "coder", "up-2").expect("sidecar rewrite");
        let raw = fs::read(dir.join(MODEL_META_JSON)).expect("read back");
        let meta: serde_json::Value = serde_json::from_slice(&raw).expect("valid JSON");
        assert!(meta.get("tool_call_parser").is_none());

        let _ = fs::remove_dir_all(dir);
    }

    /// The two crash windows a retry has to tell apart.
    #[test]
    fn an_interrupted_commit_restores_the_checkpoint_it_displaced() {
        let root = std::env::temp_dir().join(format!(
            "tachyon-broker-reconcile-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after the epoch")
                .as_nanos()
        ));
        let model_dir = root.join("coder");
        let backup_dir = root.join(".replaced-up-1");
        let plant = |dir: &Path, marker: &str, upload_id: Option<&str>| {
            fs::create_dir_all(dir).expect("fixture dir");
            fs::write(dir.join("weights.bin"), marker).expect("fixture file");
            if let Some(upload_id) = upload_id {
                fs::write(
                    dir.join(MODEL_META_JSON),
                    format!(r#"{{"format":"gguf","alias":"coder","upload_id":"{upload_id}"}}"#),
                )
                .expect("fixture sidecar");
            }
        };
        let live = || fs::read_to_string(model_dir.join("weights.bin")).expect("live checkpoint");

        // Crash between the renames: the checkpoint is parked and nothing is
        // live. Deleting the backup here — which is what this used to do —
        // left a refused retry with nothing to put back.
        plant(&backup_dir, "operator", None);
        reconcile_interrupted_commit(&model_dir, &backup_dir, "up-1").expect("recovers");
        assert_eq!(live(), "operator");
        assert!(!backup_dir.exists());

        // Crash after the second rename: our own unpublished files are live
        // *and* the checkpoint is parked. Both exist, so the case above does
        // not fire, and deleting the backup discarded the operator's
        // checkpoint while leaving an upload nothing had accepted.
        plant(&backup_dir, "operator", None);
        plant(&model_dir, "upload", Some("up-1"));
        reconcile_interrupted_commit(&model_dir, &backup_dir, "up-1").expect("rewinds");
        assert_eq!(
            live(),
            "operator",
            "the retry must start from the state the first attempt found"
        );
        assert!(!backup_dir.exists());

        // Someone else owns the live path. Neither directory is safe to touch,
        // and the refusal names both so an operator can decide.
        plant(&backup_dir, "operator", None);
        plant(&model_dir, "other-upload", Some("up-2"));
        let error = reconcile_interrupted_commit(&model_dir, &backup_dir, "up-1")
            .expect_err("a live directory owned by another upload is not ours to move");
        assert!(error.contains(".replaced-up-1"), "{error}");
        assert_eq!(live(), "other-upload");
        assert!(
            backup_dir.exists(),
            "the parked checkpoint is left for an operator"
        );

        // Nothing parked is the ordinary case and must stay a no-op.
        fs::remove_dir_all(&backup_dir).expect("clear");
        reconcile_interrupted_commit(&model_dir, &backup_dir, "up-1").expect("no-op");
        assert_eq!(live(), "other-upload");

        // A crash *after* publication is a different state, and it has a
        // different name: the commit renames the backup into the discarded
        // namespace before deleting it, so a retry that finds no `.replaced-`
        // directory leaves the published checkpoint alone. Rewinding it would
        // let a request load the predecessor under the newly published alias.
        let discarded = root.join(format!("{DISCARDED_BACKUP_PREFIX}up-1"));
        plant(&discarded, "operator", None);
        plant(&model_dir, "published", Some("up-1"));
        reconcile_interrupted_commit(&model_dir, &backup_dir, "up-1")
            .expect("a discarded backup is not a pending rollback");
        assert_eq!(
            live(),
            "published",
            "a published upload must survive a retry"
        );

        let _ = fs::remove_dir_all(&root);
    }

    /// Two commits for one alias must not interleave.
    ///
    /// Every filesystem step in the install sequence is atomic on its own, and
    /// the sequence is not: the rollback checks the live directory still
    /// carries this upload's id and *then* deletes it. A second commit landing
    /// between those two operations had its published checkpoint deleted by
    /// the first one's rollback.
    #[test]
    fn a_commit_lock_excludes_a_second_commit_for_the_same_alias() {
        let root = std::env::temp_dir().join(format!(
            "tachyon-broker-lock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after the epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("fixture dir");
        let previous = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(&root).expect("enter fixture");
        fs::create_dir_all(models_dir()).expect("models dir");

        let held = AliasCommitLock::acquire("coder").expect("a free alias locks");
        let denied = AliasCommitLock::acquire("coder")
            .expect_err("a second commit for the same alias must wait, not interleave");
        assert!(denied.contains("in progress"), "{denied}");

        // A different alias is unaffected: the lock is per alias, not global.
        let other = AliasCommitLock::acquire("embedder").expect("an unrelated alias is free");
        drop(other);

        // Released on drop, including on the early returns the commit path is
        // full of.
        drop(held);
        AliasCommitLock::acquire("coder").expect("the alias frees when the commit ends");

        std::env::set_current_dir(previous).expect("restore cwd");
        let _ = fs::remove_dir_all(&root);
    }

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
