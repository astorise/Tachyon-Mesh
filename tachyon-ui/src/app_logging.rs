use crate::app_config::{LogLevel, LoggingConfig};
use serde::Deserialize;
use serde_json::json;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_MESSAGE_CHARS: usize = 8192;
const MAX_SOURCE_CHARS: usize = 160;
static GLOBAL_LOGGER: OnceLock<AppLogger> = OnceLock::new();

#[derive(Clone)]
pub(crate) struct AppLogger {
    inner: Arc<LoggerInner>,
}

struct LoggerInner {
    minimum_level: LogLevel,
    file_path: PathBuf,
    max_file_bytes: u64,
    retained_files: usize,
    write_lock: Mutex<()>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrontendLogPayload {
    level: LogLevel,
    message: String,
    #[serde(default)]
    source: Option<String>,
}

impl AppLogger {
    pub(crate) fn new(data_dir: &Path, config: &LoggingConfig) -> Result<Self, String> {
        let file_path = data_dir.join(config.relative_file_path()?);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create log directory '{}': {error}",
                    parent.display()
                )
            })?;
        }
        prune_rotations(&file_path, config.retained_files)?;
        Ok(Self {
            inner: Arc::new(LoggerInner {
                minimum_level: config.level,
                file_path,
                max_file_bytes: config.max_file_bytes,
                retained_files: config.retained_files,
                write_lock: Mutex::new(()),
            }),
        })
    }

    pub(crate) fn register_global(&self) {
        let _ = GLOBAL_LOGGER.set(self.clone());
    }

    pub(crate) fn write_frontend(&self, payload: FrontendLogPayload) -> Result<(), String> {
        self.write(
            payload.level,
            payload.source.as_deref().unwrap_or("frontend"),
            &payload.message,
        )
    }

    pub(crate) fn write(&self, level: LogLevel, source: &str, message: &str) -> Result<(), String> {
        if level == LogLevel::Off {
            return Err("off is a configuration level and cannot be emitted".to_string());
        }
        if !self.inner.minimum_level.enables(level) {
            return Ok(());
        }

        let timestamp_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("failed to obtain log timestamp: {error}"))?
            .as_millis();
        let entry = json!({
            "timestampUnixMs": timestamp_unix_ms,
            "level": level.as_str(),
            "source": truncate(source, MAX_SOURCE_CHARS),
            "message": truncate(message, MAX_MESSAGE_CHARS),
        });
        let mut encoded = serde_json::to_vec(&entry)
            .map_err(|error| format!("failed to encode log entry: {error}"))?;
        encoded.push(b'\n');

        let _guard = self
            .inner
            .write_lock
            .lock()
            .map_err(|_| "failed to lock application log writer".to_string())?;
        self.rotate_if_required(encoded.len() as u64)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.inner.file_path)
            .map_err(|error| {
                format!(
                    "failed to open log file '{}': {error}",
                    self.inner.file_path.display()
                )
            })?;
        file.write_all(&encoded).map_err(|error| {
            format!(
                "failed to write log file '{}': {error}",
                self.inner.file_path.display()
            )
        })
    }

    fn rotate_if_required(&self, incoming_bytes: u64) -> Result<(), String> {
        let existing_bytes = match fs::metadata(&self.inner.file_path) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(format!(
                    "failed to inspect log file '{}': {error}",
                    self.inner.file_path.display()
                ));
            }
        };
        if existing_bytes == 0
            || existing_bytes.saturating_add(incoming_bytes) <= self.inner.max_file_bytes
        {
            return Ok(());
        }

        let oldest = rotated_path(&self.inner.file_path, self.inner.retained_files);
        if oldest.exists() {
            fs::remove_file(&oldest).map_err(|error| {
                format!(
                    "failed to remove rotated log '{}': {error}",
                    oldest.display()
                )
            })?;
        }
        for index in (1..self.inner.retained_files).rev() {
            let source = rotated_path(&self.inner.file_path, index);
            if source.exists() {
                let target = rotated_path(&self.inner.file_path, index + 1);
                fs::rename(&source, &target).map_err(|error| {
                    format!(
                        "failed to rotate log '{}' to '{}': {error}",
                        source.display(),
                        target.display()
                    )
                })?;
            }
        }
        let first_rotation = rotated_path(&self.inner.file_path, 1);
        fs::rename(&self.inner.file_path, &first_rotation).map_err(|error| {
            format!(
                "failed to rotate log '{}' to '{}': {error}",
                self.inner.file_path.display(),
                first_rotation.display()
            )
        })
    }
}

pub(crate) fn write_global(level: LogLevel, source: &str, message: &str) {
    if let Some(logger) = GLOBAL_LOGGER.get() {
        let _ = logger.write(level, source, message);
    }
}

fn truncate(value: &str, maximum_chars: usize) -> String {
    value.chars().take(maximum_chars).collect()
}

fn rotated_path(file_path: &Path, index: usize) -> PathBuf {
    let mut suffixed: OsString = file_path.as_os_str().to_os_string();
    suffixed.push(format!(".{index}"));
    PathBuf::from(suffixed)
}

fn prune_rotations(file_path: &Path, retained_files: usize) -> Result<(), String> {
    let Some(parent) = file_path.parent() else {
        return Ok(());
    };
    let Some(file_name) = file_path.file_name().and_then(|name| name.to_str()) else {
        return Ok(());
    };
    let rotation_prefix = format!("{file_name}.");
    let entries = fs::read_dir(parent).map_err(|error| {
        format!(
            "failed to inspect log directory '{}': {error}",
            parent.display()
        )
    })?;

    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "failed to inspect an entry in log directory '{}': {error}",
                parent.display()
            )
        })?;
        let entry_name = entry.file_name();
        let Some(suffix) = entry_name
            .to_str()
            .and_then(|name| name.strip_prefix(&rotation_prefix))
        else {
            continue;
        };
        let Ok(index) = suffix.parse::<usize>() else {
            continue;
        };
        if index == 0 || index <= retained_files {
            continue;
        }
        let entry_path = entry.path();
        if !entry
            .file_type()
            .map_err(|error| {
                format!(
                    "failed to inspect rotated log '{}': {error}",
                    entry_path.display()
                )
            })?
            .is_file()
        {
            continue;
        }
        fs::remove_file(&entry_path).map_err(|error| {
            format!(
                "failed to remove rotated log '{}': {error}",
                entry_path.display()
            )
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{rotated_path, AppLogger};
    use crate::app_config::{LogLevel, LoggingConfig};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_directory(name: &str) -> std::path::PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "tachyon-ui-log-{name}-{}-{suffix}",
            std::process::id()
        ))
    }

    #[test]
    fn filters_entries_below_the_configured_level() {
        let directory = test_directory("level");
        let config = LoggingConfig {
            level: LogLevel::Warn,
            ..LoggingConfig::default()
        };
        let logger = AppLogger::new(&directory, &config).expect("logger should initialize");

        logger
            .write(LogLevel::Info, "test", "not retained")
            .expect("filtered write should succeed");
        logger
            .write(LogLevel::Error, "test", "retained")
            .expect("error should be written");
        let contents =
            fs::read_to_string(directory.join("logs/tachyon-ui.jsonl")).expect("log should exist");

        assert!(!contents.contains("not retained"));
        assert!(contents.contains("retained"));
        fs::remove_dir_all(directory).expect("test directory should be removed");
    }

    #[test]
    fn rotates_log_files_when_the_limit_is_reached() {
        let directory = test_directory("rotate");
        let config = LoggingConfig {
            level: LogLevel::Trace,
            max_file_bytes: 1024,
            retained_files: 2,
            ..LoggingConfig::default()
        };
        let logger = AppLogger::new(&directory, &config).expect("logger should initialize");
        let message = "x".repeat(500);

        for _ in 0..4 {
            logger
                .write(LogLevel::Error, "test", &message)
                .expect("log should be written");
        }

        assert!(directory.join("logs/tachyon-ui.jsonl").exists());
        assert!(directory.join("logs/tachyon-ui.jsonl.1").exists());
        fs::remove_dir_all(directory).expect("test directory should be removed");
    }

    #[test]
    fn prunes_rotations_above_configured_retention_on_startup() {
        let directory = test_directory("reduced-retention");
        let log_path = directory.join("logs/tachyon-ui.jsonl");
        fs::create_dir_all(log_path.parent().expect("log path should have a parent"))
            .expect("log directory should be created");
        for index in 1..=20 {
            fs::write(rotated_path(&log_path, index), format!("rotation {index}"))
                .expect("rotation should be created");
        }
        let unrelated_path = directory.join("logs/tachyon-ui.jsonl.backup");
        fs::write(&unrelated_path, "unrelated").expect("unrelated file should be created");

        let config = LoggingConfig {
            retained_files: 5,
            ..LoggingConfig::default()
        };
        let _logger = AppLogger::new(&directory, &config).expect("logger should initialize");

        for index in 1..=5 {
            assert!(rotated_path(&log_path, index).exists());
        }
        for index in 6..=20 {
            assert!(!rotated_path(&log_path, index).exists());
        }
        assert!(unrelated_path.exists());
        fs::remove_dir_all(directory).expect("test directory should be removed");
    }
}
