use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub(crate) const CONFIG_FILE_NAME: &str = "tachyon-ui.config.json";
const CONFIG_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct AppConfig {
    pub(crate) schema_version: u32,
    pub(crate) logging: LoggingConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            logging: LoggingConfig::default(),
        }
    }
}

impl AppConfig {
    pub(crate) fn load_or_create(data_dir: &Path) -> Result<Self, String> {
        fs::create_dir_all(data_dir).map_err(|error| {
            format!(
                "failed to create application data directory '{}': {error}",
                data_dir.display()
            )
        })?;
        let config_path = data_dir.join(CONFIG_FILE_NAME);
        if !config_path.exists() {
            let config = Self::default();
            config.validate()?;
            let serialized = serde_json::to_string_pretty(&config)
                .map_err(|error| format!("failed to serialize default configuration: {error}"))?;
            fs::write(&config_path, format!("{serialized}\n")).map_err(|error| {
                format!(
                    "failed to write configuration file '{}': {error}",
                    config_path.display()
                )
            })?;
            return Ok(config);
        }

        let serialized = fs::read_to_string(&config_path).map_err(|error| {
            format!(
                "failed to read configuration file '{}': {error}",
                config_path.display()
            )
        })?;
        let config: Self = serde_json::from_str(&serialized).map_err(|error| {
            format!(
                "invalid configuration file '{}': {error}",
                config_path.display()
            )
        })?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            return Err(format!(
                "unsupported configuration schema version {}; expected {}",
                self.schema_version, CONFIG_SCHEMA_VERSION
            ));
        }
        self.logging.validate()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct LoggingConfig {
    pub(crate) level: LogLevel,
    pub(crate) file: String,
    pub(crate) max_file_bytes: u64,
    pub(crate) retained_files: usize,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            file: "logs/tachyon-ui.jsonl".to_string(),
            max_file_bytes: 5 * 1024 * 1024,
            retained_files: 5,
        }
    }
}

impl LoggingConfig {
    pub(crate) fn relative_file_path(&self) -> Result<PathBuf, String> {
        let path = Path::new(self.file.trim());
        if path.as_os_str().is_empty()
            || path.file_name().is_none()
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(
                "logging.file must be a relative file path within application data".to_string(),
            );
        }
        Ok(path.to_path_buf())
    }

    fn validate(&self) -> Result<(), String> {
        self.relative_file_path()?;
        if self.max_file_bytes < 1024 {
            return Err("logging.maxFileBytes must be at least 1024".to_string());
        }
        if !(1..=20).contains(&self.retained_files) {
            return Err("logging.retainedFiles must be between 1 and 20".to_string());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Off,
}

impl LogLevel {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
            Self::Off => "off",
        }
    }

    pub(crate) fn enables(self, event_level: Self) -> bool {
        self != Self::Off && event_level != Self::Off && event_level.rank() >= self.rank()
    }

    fn rank(self) -> u8 {
        match self {
            Self::Trace => 0,
            Self::Debug => 1,
            Self::Info => 2,
            Self::Warn => 3,
            Self::Error => 4,
            Self::Off => 5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AppConfig, LogLevel, CONFIG_FILE_NAME};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_directory(name: &str) -> std::path::PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "tachyon-ui-config-{name}-{}-{suffix}",
            std::process::id()
        ))
    }

    #[test]
    fn creates_extensible_default_configuration() {
        let directory = test_directory("default");
        let config = AppConfig::load_or_create(&directory).expect("default config should load");

        assert_eq!(config.logging.level, LogLevel::Info);
        assert!(directory.join(CONFIG_FILE_NAME).exists());

        fs::remove_dir_all(directory).expect("test directory should be removed");
    }

    #[test]
    fn rejects_log_files_outside_application_data() {
        let directory = test_directory("path");
        fs::create_dir_all(&directory).expect("test directory should be created");
        fs::write(
            directory.join(CONFIG_FILE_NAME),
            r#"{"schemaVersion":1,"logging":{"file":"../escape.jsonl"}}"#,
        )
        .expect("test config should be written");

        let error = AppConfig::load_or_create(&directory).expect_err("path should be rejected");
        assert!(error.contains("relative file path"));

        fs::remove_dir_all(directory).expect("test directory should be removed");
    }
}
