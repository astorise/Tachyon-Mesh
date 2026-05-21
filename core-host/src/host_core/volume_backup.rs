use super::*;

#[cfg(feature = "s3-persistence")]
const BACKUP_PREFIX_ENV: &str = "TACHYON_S3_BACKUP_PREFIX";
#[cfg(feature = "s3-persistence")]
const DEFAULT_BACKUP_PREFIX: &str = "backups";

#[cfg(feature = "s3-persistence")]
fn backup_prefix() -> String {
    std::env::var(BACKUP_PREFIX_ENV).unwrap_or_else(|_| DEFAULT_BACKUP_PREFIX.to_owned())
}

#[cfg(feature = "s3-persistence")]
fn s3_bucket() -> Option<String> {
    std::env::var("TACHYON_S3_BUCKET").ok()
}

/// Metadata returned for a single volume snapshot.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct BackupSnapshot {
    /// Unique identifier: `<route_norm>/<guest_norm>/<timestamp_ms>`.
    pub(crate) snapshot_id: String,
    pub(crate) route_path: String,
    pub(crate) guest_path: String,
    pub(crate) timestamp_ms: u64,
    pub(crate) object_count: usize,
}

#[cfg(feature = "s3-persistence")]
fn normalize_path_segment(segment: &str) -> String {
    segment.trim_matches('/').replace(['/', '\\'], "_")
}

#[cfg(feature = "s3-persistence")]
fn snapshot_s3_prefix(route_path: &str, guest_path: &str, timestamp_ms: u64) -> String {
    let prefix = backup_prefix();
    let route = normalize_path_segment(route_path);
    let guest = normalize_path_segment(guest_path);
    format!("{prefix}/{route}/{guest}/{timestamp_ms}")
}

#[cfg(feature = "s3-persistence")]
fn now_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Resolve the host-side directory for a given sealed volume.
#[cfg(feature = "s3-persistence")]
fn resolve_host_dir(
    config: &IntegrityConfig,
    route_path: &str,
    guest_path: &str,
) -> Result<PathBuf> {
    let route = config
        .sealed_route(route_path)
        .ok_or_else(|| anyhow!("route `{route_path}` not found in sealed manifest"))?;
    let volume = route
        .volumes
        .iter()
        .find(|v| v.guest_path == guest_path)
        .ok_or_else(|| anyhow!("volume `{guest_path}` not found on route `{route_path}`"))?;

    if volume.volume_type.is_s3() {
        anyhow::bail!("S3 FaaS volumes cannot be backed up — they already live in S3");
    }

    Ok(PathBuf::from(&volume.host_path))
}

/// Upload all files from `host_dir` to `s3://bucket/<s3_prefix>/`.
#[cfg(feature = "s3-persistence")]
async fn upload_dir(host_dir: &std::path::Path, bucket: &str, s3_prefix: &str) -> Result<usize> {
    use object_store::{path::Path as OsPath, ObjectStore};

    let store = build_s3_store(bucket)?;
    let mut stack = vec![host_dir.to_path_buf()];
    let mut count = 0usize;

    while let Some(entry_path) = stack.pop() {
        let meta = match tokio::fs::metadata(&entry_path).await {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_dir() {
            let mut read = tokio::fs::read_dir(&entry_path).await?;
            while let Some(child) = read.next_entry().await? {
                stack.push(child.path());
            }
        } else {
            let rel = entry_path
                .strip_prefix(host_dir)
                .unwrap_or(&entry_path)
                .to_string_lossy()
                .replace('\\', "/");
            let key = OsPath::parse(format!("{s3_prefix}/{rel}")).map_err(|e| anyhow!("{e}"))?;
            let bytes = tokio::fs::read(&entry_path).await?;
            store.put(&key, bytes.into()).await?;
            count += 1;
        }
    }
    Ok(count)
}

/// Download all objects under `s3://bucket/<s3_prefix>/` to `host_dir`.
#[cfg(feature = "s3-persistence")]
async fn download_snapshot(
    s3_prefix: &str,
    bucket: &str,
    host_dir: &std::path::Path,
) -> Result<()> {
    use futures::StreamExt as _;
    use object_store::{path::Path as OsPath, ObjectStore};

    let store = build_s3_store(bucket)?;
    let prefix_path = OsPath::parse(s3_prefix).map_err(|e| anyhow!("{e}"))?;
    let mut list = store.list(Some(&prefix_path));

    while let Some(entry) = list.next().await {
        let meta = entry?;
        let key_str = meta.location.to_string();
        let rel = key_str
            .strip_prefix(&format!("{s3_prefix}/"))
            .unwrap_or(key_str.as_str());

        let local = host_dir.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(parent) = local.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let bytes = store.get(&meta.location).await?.bytes().await?;
        tokio::fs::write(&local, &bytes).await?;
    }
    Ok(())
}

/// Create a snapshot of the volume and upload it to S3.
#[cfg_attr(not(feature = "s3-persistence"), allow(unused_variables))]
pub(crate) async fn backup_volume(
    config: &IntegrityConfig,
    route_path: &str,
    guest_path: &str,
) -> Result<BackupSnapshot> {
    #[cfg(not(feature = "s3-persistence"))]
    anyhow::bail!("s3-persistence feature is required for volume backups");

    #[cfg(feature = "s3-persistence")]
    {
        let bucket = s3_bucket().ok_or_else(|| anyhow!("TACHYON_S3_BUCKET is not configured"))?;
        let host_dir = resolve_host_dir(config, route_path, guest_path)?;
        let timestamp_ms = now_timestamp_ms();
        let route = normalize_path_segment(route_path);
        let guest = normalize_path_segment(guest_path);
        let snapshot_id = format!("{route}/{guest}/{timestamp_ms}");
        let s3_prefix = snapshot_s3_prefix(route_path, guest_path, timestamp_ms);
        let object_count = upload_dir(&host_dir, &bucket, &s3_prefix).await?;

        tracing::info!(
            route = route_path,
            guest_path,
            snapshot_id = %snapshot_id,
            object_count,
            "volume backup completed"
        );

        Ok(BackupSnapshot {
            snapshot_id,
            route_path: route_path.to_owned(),
            guest_path: guest_path.to_owned(),
            timestamp_ms,
            object_count,
        })
    }
}

/// Restore a snapshot from S3 into the volume's local directory.
#[cfg_attr(not(feature = "s3-persistence"), allow(unused_variables))]
pub(crate) async fn restore_volume(
    config: &IntegrityConfig,
    route_path: &str,
    guest_path: &str,
    snapshot_id: &str,
) -> Result<()> {
    #[cfg(not(feature = "s3-persistence"))]
    anyhow::bail!("s3-persistence feature is required for volume backups");

    #[cfg(feature = "s3-persistence")]
    {
        let bucket = s3_bucket().ok_or_else(|| anyhow!("TACHYON_S3_BUCKET is not configured"))?;
        let host_dir = resolve_host_dir(config, route_path, guest_path)?;
        let s3_prefix = format!("{}/{snapshot_id}", backup_prefix());

        tokio::fs::create_dir_all(&host_dir)
            .await
            .with_context(|| format!("failed to create volume directory {}", host_dir.display()))?;

        download_snapshot(&s3_prefix, &bucket, &host_dir).await?;

        tracing::info!(
            route = route_path,
            guest_path,
            snapshot_id,
            "volume restore completed"
        );
        Ok(())
    }
}

/// List available snapshots for a volume, newest first.
#[cfg_attr(not(feature = "s3-persistence"), allow(unused_variables))]
pub(crate) async fn list_volume_backups(
    config: &IntegrityConfig,
    route_path: &str,
    guest_path: &str,
) -> Result<Vec<BackupSnapshot>> {
    #[cfg(not(feature = "s3-persistence"))]
    anyhow::bail!("s3-persistence feature is required for volume backups");

    #[cfg(feature = "s3-persistence")]
    {
        use futures::StreamExt as _;
        use object_store::{path::Path as OsPath, ObjectStore};

        // Validate the volume exists before hitting S3.
        let _ = resolve_host_dir(config, route_path, guest_path)?;
        let bucket = s3_bucket().ok_or_else(|| anyhow!("TACHYON_S3_BUCKET is not configured"))?;
        let store = build_s3_store(&bucket)?;
        let route = normalize_path_segment(route_path);
        let guest = normalize_path_segment(guest_path);
        let volume_prefix_str = format!("{}/{route}/{guest}", backup_prefix());
        let volume_prefix = OsPath::parse(&volume_prefix_str).map_err(|e| anyhow!("{e}"))?;

        let mut timestamps = std::collections::BTreeSet::new();
        let mut list = store.list(Some(&volume_prefix));
        while let Some(entry) = list.next().await {
            let meta = match entry {
                Ok(m) => m,
                Err(_) => continue,
            };
            let key = meta.location.to_string();
            let after_volume = key
                .strip_prefix(&format!("{volume_prefix_str}/"))
                .unwrap_or("");
            if let Some(ts_str) = after_volume.split('/').next() {
                if let Ok(ts) = ts_str.parse::<u64>() {
                    timestamps.insert(ts);
                }
            }
        }

        let mut snapshots: Vec<BackupSnapshot> = Vec::new();
        for ts in &timestamps {
            let snap_prefix_str = format!("{volume_prefix_str}/{ts}");
            let snap_prefix = OsPath::parse(&snap_prefix_str).map_err(|e| anyhow!("{e}"))?;
            let mut count = 0usize;
            let mut obj_list = store.list(Some(&snap_prefix));
            while let Some(entry) = obj_list.next().await {
                if entry.is_ok() {
                    count += 1;
                }
            }
            snapshots.push(BackupSnapshot {
                snapshot_id: format!("{route}/{guest}/{ts}"),
                route_path: route_path.to_owned(),
                guest_path: guest_path.to_owned(),
                timestamp_ms: *ts,
                object_count: count,
            });
        }

        snapshots.sort_by_key(|s| std::cmp::Reverse(s.timestamp_ms));
        Ok(snapshots)
    }
}
