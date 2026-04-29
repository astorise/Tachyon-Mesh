use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "control-plane-faas",
    });

    export!(Component);
}

use serde::{Deserialize, Serialize};

const BUFFER_DIR_ENV: &str = "BUFFER_DIR";
const RAM_QUEUE_CAPACITY_ENV: &str = "RAM_QUEUE_CAPACITY";
const REPLAY_CPU_LIMIT_ENV: &str = "REPLAY_CPU_LIMIT";
const REPLAY_RAM_LIMIT_ENV: &str = "REPLAY_RAM_LIMIT";
const REPLAY_BATCH_SIZE_ENV: &str = "REPLAY_BATCH_SIZE";
const ORIGINAL_ROUTE_HEADER: &str = "x-tachyon-original-route";
const REPLAY_HEADER: &str = "x-tachyon-buffer-replay";
const DEFAULT_BUFFER_DIR: &str = "/buffer";
const DEFAULT_RAM_QUEUE_CAPACITY: usize = 32;
const DEFAULT_REPLAY_CPU_LIMIT: u8 = 65;
const DEFAULT_REPLAY_RAM_LIMIT: u8 = 65;
const DEFAULT_REPLAY_BATCH_SIZE: usize = 8;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct Component;

impl bindings::Guest for Component {
    fn on_tick() {
        if let Err(error) = replay_queued_requests() {
            eprintln!("system-faas-buffer tick failed: {error}");
        }
    }
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        let route = normalize_route(&req.uri).unwrap_or_else(|_| "/".to_owned());
        if req.method.eq_ignore_ascii_case("GET") && route == "/queue_depth" {
            return response(
                200,
                queue_depth().to_string().into_bytes(),
                &[("content-type", "text/plain; charset=utf-8")],
            );
        }
        if req.method.eq_ignore_ascii_case("POST") && route == "/push" {
            return match enqueue_request(req) {
                Ok(queue_file) => response(
                    202,
                    format!(r#"{{"job_id":"{queue_file}","status":"queued"}}"#).into_bytes(),
                    &[("content-type", "application/json")],
                ),
                Err(error) => response(500, error.into_bytes(), &[]),
            };
        }
        if req.method.eq_ignore_ascii_case("POST") && route == "/pop" {
            return match pop_request() {
                Ok(Some((id, body))) => response(
                    200,
                    body,
                    &[
                        ("content-type", "application/json"),
                        ("x-tachyon-job-id", &id),
                    ],
                ),
                Ok(None) => response(204, Vec::new(), &[]),
                Err(error) => response(500, error.into_bytes(), &[]),
            };
        }
        if req.method.eq_ignore_ascii_case("POST") && route.starts_with("/ack/") {
            let id = route.trim_start_matches("/ack/");
            return match ack_request(id) {
                Ok(true) => response(204, Vec::new(), &[]),
                Ok(false) => response(404, b"unknown job id".to_vec(), &[]),
                Err(error) => response(500, error.into_bytes(), &[]),
            };
        }
        match enqueue_request(req) {
            Ok(queue_file) => response(
                202,
                format!("buffered:{queue_file}").into_bytes(),
                &[("content-type", "text/plain; charset=utf-8")],
            ),
            Err(error) => response(500, error.into_bytes(), &[]),
        }
    }
}

fn queue_depth() -> usize {
    queued_files(&buffer_root())
        .map(|files| files.len())
        .unwrap_or(0)
}

fn pop_request() -> Result<Option<(String, Vec<u8>)>, String> {
    let Some(path) = queued_files(&buffer_root())?.into_iter().next() else {
        return Ok(None);
    };
    let id = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "queued file name is invalid".to_owned())?
        .to_owned();
    let body = fs::read(&path).map_err(|error| format!("failed to read queued job: {error}"))?;
    Ok(Some((id, body)))
}

fn ack_request(id: &str) -> Result<bool, String> {
    for queue_name in ["ram", "disk"] {
        let path = buffer_root().join(queue_name).join(id);
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|error| format!("failed to ack queued job `{id}`: {error}"))?;
            return Ok(true);
        }
    }
    Ok(false)
}

fn enqueue_request(
    req: bindings::exports::tachyon::mesh::handler::Request,
) -> Result<String, String> {
    let root = buffer_root();
    let ram_dir = root.join("ram");
    let disk_dir = root.join("disk");
    fs::create_dir_all(&ram_dir).map_err(|error| format!("failed to create RAM queue: {error}"))?;
    fs::create_dir_all(&disk_dir)
        .map_err(|error| format!("failed to create disk queue: {error}"))?;

    let original_route = original_route(&req)?;
    let priority = request_priority(&req.headers);
    let ram_capacity = parse_usize_env(RAM_QUEUE_CAPACITY_ENV, DEFAULT_RAM_QUEUE_CAPACITY);
    let target_dir = if queue_len(&ram_dir) < ram_capacity {
        ram_dir
    } else {
        disk_dir
    };

    let envelope = BufferedRequestEnvelope {
        method: req.method,
        original_route,
        headers: req.headers,
        body: req.body,
        priority: priority.as_str().to_owned(),
    };
    let body = serde_json::to_vec(&envelope)
        .map_err(|error| format!("failed to encode buffered request: {error}"))?;
    let file_name = queue_file_name(priority);
    let file_path = target_dir.join(&file_name);
    fs::write(&file_path, body).map_err(|error| {
        format!(
            "failed to persist buffered request `{}`: {error}",
            file_path.display()
        )
    })?;
    Ok(file_name)
}

fn replay_queued_requests() -> Result<(), String> {
    let snapshot = bindings::tachyon::mesh::telemetry_reader::get_metrics();
    if snapshot.cpu_pressure > parse_u8_env(REPLAY_CPU_LIMIT_ENV, DEFAULT_REPLAY_CPU_LIMIT)
        || snapshot.ram_pressure > parse_u8_env(REPLAY_RAM_LIMIT_ENV, DEFAULT_REPLAY_RAM_LIMIT)
    {
        return Ok(());
    }

    let batch_size = parse_usize_env(REPLAY_BATCH_SIZE_ENV, DEFAULT_REPLAY_BATCH_SIZE);
    let mut replayed = 0_usize;
    for path in queued_files(&buffer_root())? {
        if replayed >= batch_size {
            break;
        }

        let payload =
            fs::read(&path).map_err(|error| format!("failed to read buffered request: {error}"))?;
        let envelope = serde_json::from_slice::<BufferedRequestEnvelope>(&payload)
            .map_err(|error| format!("failed to decode buffered request: {error}"))?;
        let mut headers = envelope.headers.clone();
        headers.push((REPLAY_HEADER.to_owned(), "1".to_owned()));
        let response = bindings::tachyon::mesh::outbound_http::send_request(
            &envelope.method,
            &format!("http://mesh{}", envelope.original_route),
            &headers,
            &envelope.body,
        )?;
        if response.status < 500 && response.status != 429 {
            fs::remove_file(&path).map_err(|error| {
                format!(
                    "failed to remove replayed buffered request `{}`: {error}",
                    path.display()
                )
            })?;
            replayed = replayed.saturating_add(1);
        }
    }

    Ok(())
}

fn buffer_root() -> PathBuf {
    std::env::var(BUFFER_DIR_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_BUFFER_DIR))
}

fn original_route(
    req: &bindings::exports::tachyon::mesh::handler::Request,
) -> Result<String, String> {
    header_value(&req.headers, ORIGINAL_ROUTE_HEADER)
        .map(normalize_route)
        .transpose()?
        .or_else(|| normalize_route(&req.uri).ok())
        .ok_or_else(|| "buffered requests must include an original route".to_owned())
}

fn request_priority(headers: &[(String, String)]) -> RequestPriority {
    match header_value(headers, "x-priority")
        .or_else(|| header_value(headers, "priority"))
        .unwrap_or("normal")
        .to_ascii_lowercase()
        .as_str()
    {
        "high" => RequestPriority::High,
        "low" => RequestPriority::Low,
        _ => RequestPriority::Normal,
    }
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn normalize_route(route: &str) -> Result<String, String> {
    let trimmed = route.trim();
    if trimmed.is_empty() {
        return Err("route must not be empty".to_owned());
    }
    let path = if let Some((_, suffix)) = trimmed.split_once("://") {
        suffix
            .find('/')
            .map(|index| &suffix[index..])
            .unwrap_or("/")
            .to_owned()
    } else if trimmed.starts_with('/') {
        trimmed.to_owned()
    } else {
        format!("/{trimmed}")
    };
    Ok(path.trim_end_matches('/').to_owned().pipe(|normalized| {
        if normalized.is_empty() {
            "/".to_owned()
        } else {
            normalized
        }
    }))
}

fn queue_len(path: &Path) -> usize {
    fs::read_dir(path)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .count()
}

fn queued_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for queue_name in ["ram", "disk"] {
        let queue_dir = root.join(queue_name);
        let entries = match fs::read_dir(&queue_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "failed to enumerate buffered queue `{}`: {error}",
                    queue_dir.display()
                ))
            }
        };
        for entry in entries {
            let entry = entry.map_err(|error| format!("failed to read buffered entry: {error}"))?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn queue_file_name(priority: RequestPriority) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("{}-{timestamp:020}-{sequence:020}.json", priority.as_str())
}

fn parse_usize_env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(default)
}

fn parse_u8_env(name: &str, default: u8) -> u8 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u8>().ok())
        .unwrap_or(default)
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RequestPriority {
    High,
    Normal,
    Low,
}

impl RequestPriority {
    fn as_str(self) -> &'static str {
        match self {
            Self::High => "0-high",
            Self::Normal => "1-normal",
            Self::Low => "2-low",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct BufferedRequestEnvelope {
    method: String,
    original_route: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    priority: String,
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}

impl<T> Pipe for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_route_accepts_absolute_urls() {
        assert_eq!(
            normalize_route("http://127.0.0.1:8080/api/example").expect("url should normalize"),
            "/api/example"
        );
    }

    #[test]
    fn request_priority_defaults_to_normal() {
        assert_eq!(request_priority(&[]), RequestPriority::Normal);
        assert_eq!(
            request_priority(&[("x-priority".to_owned(), "HIGH".to_owned())]),
            RequestPriority::High
        );
    }

    #[test]
    fn queued_files_are_sorted_by_priority_prefix() {
        let root = std::env::temp_dir().join(format!(
            "tachyon-buffer-{}",
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let ram_dir = root.join("ram");
        let disk_dir = root.join("disk");
        fs::create_dir_all(&ram_dir).expect("ram queue should create");
        fs::create_dir_all(&disk_dir).expect("disk queue should create");
        fs::write(ram_dir.join("1-normal-002.json"), b"{}").expect("normal file should write");
        fs::write(disk_dir.join("0-high-001.json"), b"{}").expect("high file should write");

        let files = queued_files(&root).expect("queued files should enumerate");
        assert_eq!(
            files
                .iter()
                .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
                .collect::<Vec<_>>(),
            vec!["0-high-001.json".to_owned(), "1-normal-002.json".to_owned()]
        );

        let _ = fs::remove_dir_all(root);
    }
}
