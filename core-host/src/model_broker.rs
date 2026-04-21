use anyhow::{anyhow, Context, Result};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Path as StdPath, PathBuf},
    sync::Arc,
};
use tokio::{
    fs::{self, OpenOptions},
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};
use uuid::Uuid;

const MODEL_CHUNK_BYTES: usize = 5 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct ModelBroker {
    model_dir: PathBuf,
    upload_dir: PathBuf,
    uploads: Arc<Mutex<HashMap<String, PendingUpload>>>,
}

#[derive(Clone)]
struct PendingUpload {
    expected_hash: String,
    size_bytes: u64,
    temp_path: PathBuf,
    bytes_received: u64,
    last_part: u64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct InitUploadRequest {
    expected_hash: String,
    size_bytes: u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct InitUploadResponse {
    upload_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UploadQuery {
    part: u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct CommitUploadResponse {
    model_path: String,
}

impl ModelBroker {
    pub(crate) fn new(manifest_path: &StdPath) -> Result<Self> {
        let root = manifest_path
            .parent()
            .unwrap_or_else(|| StdPath::new("."))
            .join("tachyon_data");
        let model_dir = root.join("models");
        let upload_dir = root.join("model-uploads");
        std::fs::create_dir_all(&model_dir).with_context(|| {
            format!(
                "failed to initialize model directory `{}`",
                model_dir.display()
            )
        })?;
        std::fs::create_dir_all(&upload_dir).with_context(|| {
            format!(
                "failed to initialize model upload directory `{}`",
                upload_dir.display()
            )
        })?;

        Ok(Self {
            model_dir,
            upload_dir,
            uploads: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub(crate) async fn init_upload(&self, expected_hash: &str, size_bytes: u64) -> Result<String> {
        validate_hash(expected_hash)?;
        if size_bytes == 0 {
            anyhow::bail!("model upload size must be greater than zero");
        }

        let upload_id = Uuid::new_v4().to_string();
        let temp_path = self.upload_dir.join(format!("{upload_id}.part"));
        fs::write(&temp_path, []).await.with_context(|| {
            format!("failed to initialize upload file `{}`", temp_path.display())
        })?;

        let mut uploads = self.uploads.lock().await;
        uploads.insert(
            upload_id.clone(),
            PendingUpload {
                expected_hash: expected_hash.to_owned(),
                size_bytes,
                temp_path,
                bytes_received: 0,
                last_part: 0,
            },
        );
        Ok(upload_id)
    }

    pub(crate) async fn append_chunk(
        &self,
        upload_id: &str,
        part: u64,
        chunk: Bytes,
    ) -> Result<()> {
        if chunk.is_empty() {
            anyhow::bail!("model upload chunk must not be empty");
        }
        if chunk.len() > MODEL_CHUNK_BYTES {
            anyhow::bail!(
                "model upload chunk exceeds the 5 MiB protocol limit ({} bytes)",
                chunk.len()
            );
        }

        let mut uploads = self.uploads.lock().await;
        let pending = uploads
            .get_mut(upload_id)
            .ok_or_else(|| anyhow!("unknown model upload `{upload_id}`"))?;
        if part != pending.last_part + 1 {
            anyhow::bail!(
                "model upload `{upload_id}` expected part {}, received {part}",
                pending.last_part + 1
            );
        }

        let mut file = OpenOptions::new()
            .append(true)
            .open(&pending.temp_path)
            .await
            .with_context(|| {
                format!(
                    "failed to open staging file `{}` for upload `{upload_id}`",
                    pending.temp_path.display()
                )
            })?;
        file.write_all(&chunk).await.with_context(|| {
            format!(
                "failed to append bytes to staging file `{}`",
                pending.temp_path.display()
            )
        })?;
        file.flush().await.with_context(|| {
            format!(
                "failed to flush staging file `{}`",
                pending.temp_path.display()
            )
        })?;

        pending.bytes_received = pending.bytes_received.saturating_add(chunk.len() as u64);
        pending.last_part = part;
        Ok(())
    }

    pub(crate) async fn commit_upload(&self, upload_id: &str) -> Result<String> {
        let pending = {
            let mut uploads = self.uploads.lock().await;
            uploads
                .remove(upload_id)
                .ok_or_else(|| anyhow!("unknown model upload `{upload_id}`"))?
        };

        if pending.bytes_received != pending.size_bytes {
            let _ = fs::remove_file(&pending.temp_path).await;
            anyhow::bail!(
                "model upload `{upload_id}` expected {} bytes but received {}",
                pending.size_bytes,
                pending.bytes_received
            );
        }

        let computed_hash = hash_file(&pending.temp_path).await?;
        if computed_hash != pending.expected_hash {
            let _ = fs::remove_file(&pending.temp_path).await;
            anyhow::bail!(
                "model upload `{upload_id}` hash mismatch: expected `{}`, computed `{computed_hash}`",
                pending.expected_hash
            );
        }

        let final_path = self.model_dir.join(format!(
            "{}.gguf",
            pending.expected_hash.trim_start_matches("sha256:")
        ));
        if fs::try_exists(&final_path).await.unwrap_or(false) {
            fs::remove_file(&final_path).await.with_context(|| {
                format!(
                    "failed to replace existing model `{}`",
                    final_path.display()
                )
            })?;
        }
        fs::rename(&pending.temp_path, &final_path)
            .await
            .with_context(|| {
                format!(
                    "failed to finalize model upload from `{}` to `{}`",
                    pending.temp_path.display(),
                    final_path.display()
                )
            })?;

        Ok(final_path.to_string_lossy().to_string())
    }
}

pub(crate) async fn init_upload_handler(
    State(state): State<crate::AppState>,
    Json(payload): Json<InitUploadRequest>,
) -> Result<Json<InitUploadResponse>, Response> {
    let upload_id = state
        .model_broker
        .init_upload(&payload.expected_hash, payload.size_bytes)
        .await
        .map_err(model_error_to_response)?;
    Ok(Json(InitUploadResponse { upload_id }))
}

pub(crate) async fn upload_chunk_handler(
    State(state): State<crate::AppState>,
    Path(upload_id): Path<String>,
    Query(query): Query<UploadQuery>,
    body: Bytes,
) -> Result<StatusCode, Response> {
    state
        .model_broker
        .append_chunk(&upload_id, query.part, body)
        .await
        .map_err(model_error_to_response)?;
    Ok(StatusCode::ACCEPTED)
}

pub(crate) async fn commit_upload_handler(
    State(state): State<crate::AppState>,
    Path(upload_id): Path<String>,
) -> Result<Json<CommitUploadResponse>, Response> {
    let model_path = state
        .model_broker
        .commit_upload(&upload_id)
        .await
        .map_err(model_error_to_response)?;
    Ok(Json(CommitUploadResponse { model_path }))
}

fn validate_hash(hash: &str) -> Result<()> {
    let digest = hash
        .strip_prefix("sha256:")
        .ok_or_else(|| anyhow!("model hash `{hash}` must start with `sha256:`"))?;
    if digest.is_empty()
        || !digest
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        anyhow::bail!("model hash `{hash}` must be a hexadecimal sha256 digest");
    }
    Ok(())
}

async fn hash_file(path: &StdPath) -> Result<String> {
    let mut file = fs::File::open(path)
        .await
        .with_context(|| format!("failed to open model file `{}` for hashing", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; MODEL_CHUNK_BYTES];
    loop {
        let read = file.read(&mut buffer).await.with_context(|| {
            format!("failed to read model file `{}` for hashing", path.display())
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
}

fn model_error_to_response(error: anyhow::Error) -> Response {
    let message = error.to_string();
    let status = if message.contains("must")
        || message.contains("expected")
        || message.contains("unknown")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };

    (status, message).into_response()
}
