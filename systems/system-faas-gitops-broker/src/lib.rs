mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

use serde::Serialize;
use std::path::{Path, PathBuf};

const CONFIG_STORE_PATH_ENV: &str = "CONFIG_STORE_PATH";
const DEFAULT_CONFIG_STORE_PATH: &str = "/var/lib/tachyon/config-store";
const CONFIG_STORE_PATH_HEADER: &str = "x-tachyon-config-store-path";

struct Component;

#[derive(Debug, Serialize)]
pub struct RepositoryInfo {
    workdir: String,
    git_dir: String,
    initialized: bool,
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        if !req.method.eq_ignore_ascii_case("POST") {
            return response(405, "Method Not Allowed", &[]);
        }

        let path = config_store_path(&req.headers);
        match initialize_git_repo(&path) {
            Ok(info) => json_response(202, &info),
            Err(error) => response(500, error, &[]),
        }
    }
}

pub fn initialize_git_repo(path: impl AsRef<Path>) -> Result<RepositoryInfo, String> {
    let path = path.as_ref();
    std::fs::create_dir_all(path).map_err(|error| {
        format!(
            "failed to create config store directory `{}`: {error}",
            path.display()
        )
    })?;

    let already_initialized = path.join(".git").is_dir();
    let repo = if already_initialized {
        gix::open(path).map_err(|error| {
            format!(
                "failed to open git config store `{}`: {error}",
                path.display()
            )
        })?
    } else {
        gix::init(path).map_err(|error| {
            format!(
                "failed to initialize git config store `{}`: {error}",
                path.display()
            )
        })?
    };

    Ok(RepositoryInfo {
        workdir: repo.workdir().unwrap_or(path).display().to_string(),
        git_dir: repo.git_dir().display().to_string(),
        initialized: !already_initialized,
    })
}

fn config_store_path(headers: &[(String, String)]) -> PathBuf {
    header_value(headers, CONFIG_STORE_PATH_HEADER)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var(CONFIG_STORE_PATH_ENV)
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_STORE_PATH))
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn response(
    status: u16,
    body: impl Into<Vec<u8>>,
    headers: &[(&str, &str)],
) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status,
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        body: body.into(),
        trailers: vec![],
    }
}

fn json_response<T: Serialize>(
    status: u16,
    body: &T,
) -> bindings::exports::tachyon::mesh::handler::Response {
    match serde_json::to_vec(body) {
        Ok(body) => response(status, body, &[("content-type", "application/json")]),
        Err(error) => response(500, format!("failed to encode response: {error}"), &[]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn initialize_git_repo_creates_repository_once() {
        let path = unique_test_dir();
        let first = initialize_git_repo(&path).expect("repo should initialize");
        let second = initialize_git_repo(&path).expect("repo should reopen");

        assert!(first.initialized);
        assert!(!second.initialized);
        assert!(path.join(".git").is_dir());
        std::fs::remove_dir_all(path).ok();
    }

    #[test]
    fn config_store_path_prefers_header() {
        let path = config_store_path(&[(
            CONFIG_STORE_PATH_HEADER.to_owned(),
            "/tmp/tachyon-config-store".to_owned(),
        )]);

        assert_eq!(path, PathBuf::from("/tmp/tachyon-config-store"));
    }

    fn unique_test_dir() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be valid")
            .as_nanos();
        std::env::temp_dir().join(format!("tachyon-gitops-broker-{nonce}"))
    }
}
