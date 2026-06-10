use anyhow::{Context, Result};
use futures::StreamExt as _;
use object_store::{aws::AmazonS3Builder, path::Path as OsPath, ObjectStore, ObjectStoreExt};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::fs;

const S3_ENDPOINT_ENV: &str = "TACHYON_S3_ENDPOINT";
const S3_BUCKET_ENV: &str = "TACHYON_S3_BUCKET";
const S3_ACCESS_KEY_ENV: &str = "TACHYON_S3_ACCESS_KEY_ID";
const S3_SECRET_KEY_ENV: &str = "TACHYON_S3_SECRET_ACCESS_KEY";
const S3_PREFIX_ENV: &str = "TACHYON_S3_PREFIX";
const S3_REGION_ENV: &str = "TACHYON_S3_REGION";
const S3_FORCE_PATH_STYLE_ENV: &str = "TACHYON_S3_FORCE_PATH_STYLE";

const DEFAULT_REGION: &str = "us-east-1";
const BACKGROUND_FLUSH_INTERVAL: Duration = Duration::from_secs(300);

pub(crate) struct S3PersistenceConfig {
    endpoint: Option<String>,
    bucket: String,
    access_key: String,
    secret_key: String,
    prefix: String,
    region: String,
    force_path_style: bool,
}

impl S3PersistenceConfig {
    pub(crate) fn from_env() -> Option<Self> {
        let bucket = std::env::var(S3_BUCKET_ENV).ok()?;
        Some(Self {
            endpoint: std::env::var(S3_ENDPOINT_ENV).ok(),
            bucket,
            access_key: std::env::var(S3_ACCESS_KEY_ENV).unwrap_or_default(),
            secret_key: std::env::var(S3_SECRET_KEY_ENV).unwrap_or_default(),
            prefix: std::env::var(S3_PREFIX_ENV).unwrap_or_default(),
            region: std::env::var(S3_REGION_ENV).unwrap_or_else(|_| DEFAULT_REGION.to_owned()),
            force_path_style: parse_bool_env(S3_FORCE_PATH_STYLE_ENV),
        })
    }
}

fn parse_bool_env(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

pub(crate) struct S3PersistenceBackend {
    store: Arc<dyn ObjectStore>,
    prefix: String,
    local_root: PathBuf,
}

impl S3PersistenceBackend {
    pub(crate) fn new(config: S3PersistenceConfig, local_root: PathBuf) -> Result<Self> {
        let mut builder = AmazonS3Builder::new()
            .with_bucket_name(&config.bucket)
            .with_access_key_id(&config.access_key)
            .with_secret_access_key(&config.secret_key)
            .with_region(&config.region)
            .with_allow_http(true);
        if let Some(endpoint) = &config.endpoint {
            builder = builder.with_endpoint(endpoint);
        }
        if config.force_path_style {
            builder = builder.with_virtual_hosted_style_request(false);
        }
        let store = builder.build().context("failed to build S3 client")?;
        Ok(Self {
            store: Arc::new(store),
            prefix: config.prefix,
            local_root,
        })
    }

    fn s3_key(&self, local_path: &Path) -> Result<OsPath> {
        let rel = local_path
            .strip_prefix(&self.local_root)
            .context("path is outside local root")?;
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let key = if self.prefix.is_empty() {
            rel_str
        } else {
            format!("{}/{}", self.prefix.trim_end_matches('/'), rel_str)
        };
        OsPath::parse(&key).context("invalid S3 key")
    }

    /// Download all objects under the configured S3 prefix to `local_root`.
    pub(crate) async fn restore(&self) -> Result<()> {
        let prefix_path = if self.prefix.is_empty() {
            None
        } else {
            Some(OsPath::parse(&self.prefix).context("invalid S3 prefix")?)
        };

        let mut list = self.store.list(prefix_path.as_ref());
        let mut restored = 0usize;

        while let Some(entry) = list.next().await {
            let meta = match entry {
                Ok(m) => m,
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "S3 list error during restore — skipping object"
                    );
                    continue;
                }
            };

            let key_str = meta.location.to_string();
            let rel = if self.prefix.is_empty() {
                key_str.as_str()
            } else {
                key_str
                    .strip_prefix(&format!("{}/", self.prefix.trim_end_matches('/')))
                    .unwrap_or(key_str.as_str())
            };

            let local_path = self
                .local_root
                .join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));

            if let Some(parent) = local_path.parent() {
                fs::create_dir_all(parent)
                    .await
                    .with_context(|| format!("failed to create directory {}", parent.display()))?;
            }

            let bytes = self
                .store
                .get(&meta.location)
                .await
                .with_context(|| format!("failed to GET {}", meta.location))?
                .bytes()
                .await
                .with_context(|| format!("failed to read bytes for {}", meta.location))?;

            fs::write(&local_path, &bytes)
                .await
                .with_context(|| format!("failed to write {}", local_path.display()))?;
            restored += 1;
        }

        if restored > 0 {
            tracing::info!(restored, "S3 restore complete");
        }
        Ok(())
    }

    /// Upload all files under `path` recursively to S3.
    pub(crate) async fn flush_path(&self, path: &Path) -> Result<()> {
        let mut stack = vec![path.to_path_buf()];
        let mut flushed = 0usize;

        while let Some(entry_path) = stack.pop() {
            let metadata = match fs::metadata(&entry_path).await {
                Ok(m) => m,
                Err(_) => continue,
            };

            if metadata.is_dir() {
                let mut dir = fs::read_dir(&entry_path)
                    .await
                    .with_context(|| format!("failed to read dir {}", entry_path.display()))?;
                while let Some(child) = dir.next_entry().await? {
                    stack.push(child.path());
                }
            } else {
                let key = self.s3_key(&entry_path)?;
                let bytes = fs::read(&entry_path)
                    .await
                    .with_context(|| format!("failed to read {}", entry_path.display()))?;
                self.store
                    .put(&key, bytes.into())
                    .await
                    .with_context(|| format!("failed to PUT {key}"))?;
                flushed += 1;
            }
        }

        if flushed > 0 {
            tracing::debug!(
                flushed,
                path = %path.display(),
                "S3 flush complete"
            );
        }
        Ok(())
    }

    /// Spawn a background task that flushes `local_root` every `BACKGROUND_FLUSH_INTERVAL`.
    pub(crate) fn spawn_background_flush(backend: Arc<Self>) {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(BACKGROUND_FLUSH_INTERVAL).await;
                let root = backend.local_root.clone();
                if let Err(error) = backend.flush_path(&root).await {
                    tracing::warn!(error = %error, "background S3 flush failed");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_backend(prefix: &str, root: &str) -> S3PersistenceBackend {
        S3PersistenceBackend {
            store: Arc::new(object_store::memory::InMemory::new()),
            prefix: prefix.to_owned(),
            local_root: PathBuf::from(root),
        }
    }

    #[test]
    fn s3_key_strips_local_root_and_uses_prefix() {
        let b = make_backend("tachyon", "/data");
        let key = b
            .s3_key(Path::new("/data/auth-state/admin.json"))
            .expect("valid path should produce a valid S3 key");
        assert_eq!(key.to_string(), "tachyon/auth-state/admin.json");
    }

    #[test]
    fn s3_key_without_prefix() {
        let b = make_backend("", "/data");
        let key = b
            .s3_key(Path::new("/data/tachyon.db"))
            .expect("valid path should produce a valid S3 key");
        assert_eq!(key.to_string(), "tachyon.db");
    }

    #[test]
    fn from_env_returns_none_when_bucket_absent() {
        std::env::remove_var(S3_BUCKET_ENV);
        assert!(S3PersistenceConfig::from_env().is_none());
    }
}
