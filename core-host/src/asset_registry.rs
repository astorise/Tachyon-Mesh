use anyhow::{anyhow, Context, Result};
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock, RwLock},
};

const ASSET_URI_PREFIX: &str = "tachyon://sha256:";
const EXPECTED_HASH_HEADER: &str = "x-tachyon-expected-sha256";

#[derive(Clone)]
struct AssetRegistryRuntime {
    core_store: Arc<crate::store::CoreStore>,
    asset_dir: PathBuf,
}

#[derive(Debug, Serialize)]
pub(crate) struct AssetUploadResponse {
    pub(crate) asset_uri: String,
}

static ASSET_REGISTRY_RUNTIME: OnceLock<RwLock<Option<AssetRegistryRuntime>>> = OnceLock::new();

fn asset_registry_runtime() -> &'static RwLock<Option<AssetRegistryRuntime>> {
    ASSET_REGISTRY_RUNTIME.get_or_init(|| RwLock::new(None))
}

pub(crate) fn asset_registry_dir(manifest_path: &Path) -> PathBuf {
    manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("asset-registry")
}

pub(crate) fn configure_asset_registry(
    core_store: Arc<crate::store::CoreStore>,
    manifest_path: &Path,
) -> Result<()> {
    let asset_dir = asset_registry_dir(manifest_path);
    fs::create_dir_all(&asset_dir).with_context(|| {
        format!(
            "failed to initialize asset registry directory `{}`",
            asset_dir.display()
        )
    })?;

    let mut runtime = asset_registry_runtime()
        .write()
        .expect("asset registry runtime should not be poisoned");
    *runtime = Some(AssetRegistryRuntime {
        core_store,
        asset_dir,
    });
    Ok(())
}

pub(crate) fn is_asset_uri(value: &str) -> bool {
    value.starts_with(ASSET_URI_PREFIX)
}

pub(crate) fn materialize_asset_uri(uri: &str) -> Result<PathBuf> {
    let hash = hash_from_asset_uri(uri)?;
    let runtime = asset_registry_runtime()
        .read()
        .expect("asset registry runtime should not be poisoned")
        .clone()
        .ok_or_else(|| anyhow!("asset registry runtime is not configured"))?;
    let bytes = load_asset(&runtime.core_store, &hash)?;
    let path = materialized_asset_path(&runtime.asset_dir, &hash);
    write_materialized_asset(&path, &bytes)?;
    Ok(path)
}

pub(crate) fn save_asset(
    core_store: &crate::store::CoreStore,
    hash: &str,
    data: &[u8],
) -> Result<()> {
    validate_hash(hash)?;
    core_store.put(crate::store::CoreStoreBucket::AssetRegistry, hash, data)
}

pub(crate) fn load_asset(core_store: &crate::store::CoreStore, hash: &str) -> Result<Vec<u8>> {
    validate_hash(hash)?;
    core_store
        .get(crate::store::CoreStoreBucket::AssetRegistry, hash)?
        .ok_or_else(|| anyhow!("asset `{hash}` was not found in the embedded registry"))
}

pub(crate) async fn upload_asset_handler(
    State(state): State<crate::AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<AssetUploadResponse>, Response> {
    if body.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "asset body must not be empty").into_response());
    }

    let hash = sha256_hash(&body);
    if let Some(expected_hash) = headers
        .get(EXPECTED_HASH_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if expected_hash != hash {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("asset checksum mismatch: expected `{expected_hash}`, computed `{hash}`"),
            )
                .into_response());
        }
    }

    save_asset(&state.core_store, &hash, &body).map_err(asset_error_to_response)?;
    let materialized_path =
        materialized_asset_path(&asset_registry_dir(&state.manifest_path), &hash);
    write_materialized_asset(&materialized_path, &body).map_err(asset_error_to_response)?;

    Ok(Json(AssetUploadResponse {
        asset_uri: asset_uri(&hash),
    }))
}

pub(crate) fn asset_uri(hash: &str) -> String {
    format!("tachyon://{hash}")
}

fn sha256_hash(data: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(data)))
}

fn hash_from_asset_uri(uri: &str) -> Result<String> {
    let hash = uri
        .strip_prefix("tachyon://")
        .ok_or_else(|| anyhow!("asset URI `{uri}` must start with `tachyon://`"))?;
    validate_hash(hash)?;
    Ok(hash.to_owned())
}

fn validate_hash(hash: &str) -> Result<()> {
    let digest = hash
        .strip_prefix("sha256:")
        .ok_or_else(|| anyhow!("asset hash `{hash}` must start with `sha256:`"))?;
    if digest.is_empty()
        || !digest
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        anyhow::bail!("asset hash `{hash}` must be a hexadecimal sha256 digest");
    }
    Ok(())
}

fn materialized_asset_path(asset_dir: &Path, hash: &str) -> PathBuf {
    asset_dir.join(format!("{}.wasm", hash.trim_start_matches("sha256:")))
}

fn write_materialized_asset(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create asset registry parent directory `{}`",
                parent.display()
            )
        })?;
    }
    fs::write(path, bytes)
        .with_context(|| format!("failed to write materialized asset `{}`", path.display()))
}

fn asset_error_to_response(error: anyhow::Error) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_uri_round_trips_to_hash() {
        let uri = asset_uri("sha256:abc123");

        assert_eq!(uri, "tachyon://sha256:abc123");
        assert_eq!(
            hash_from_asset_uri(&uri).expect("asset URI should parse"),
            "sha256:abc123"
        );
    }
}
