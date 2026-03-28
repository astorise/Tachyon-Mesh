use anyhow::{anyhow, bail, Context, Result};
use ed25519_dalek::{Signer, SigningKey};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, path::PathBuf};

const DEFAULT_HOST_ADDRESS: &str = "0.0.0.0:8080";
const DEFAULT_MAX_STDOUT_BYTES: usize = 64 * 1024;
const DEFAULT_GUEST_FUEL_BUDGET: u64 = 250_000;
const DEFAULT_RESOURCE_LIMIT_RESPONSE: &str = "Execution trapped: Resource limit exceeded";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct SealedConfig {
    pub host_address: String,
    pub max_stdout_bytes: usize,
    pub guest_fuel_budget: u64,
    pub guest_memory_limit_bytes: usize,
    pub resource_limit_response: String,
    pub routes: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct IntegrityManifest {
    config_payload: String,
    public_key: String,
    signature: String,
}

#[derive(Debug, PartialEq, Eq)]
struct GenerateRequest {
    routes: Vec<String>,
    memory_mib: u32,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if let Err(error) = run_inner() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

fn run_inner() -> Result<()> {
    if let Some(request) = parse_generate_request_from_args(std::env::args().skip(1))? {
        let manifest_path = generate_manifest(request)?;
        println!("wrote integrity manifest to {}", manifest_path.display());
        return Ok(());
    }

    #[cfg(desktop)]
    {
        tauri::Builder::default()
            .setup(
                |app| -> std::result::Result<(), Box<dyn std::error::Error>> {
                    app.handle().plugin(tauri_plugin_cli::init())?;
                    handle_cli(app)?;
                    app.handle().exit(0);
                    Ok(())
                },
            )
            .run(tauri::generate_context!())
            .map_err(|error| anyhow!("tachyon-cli runtime failed: {error}"))?;

        Ok(())
    }

    #[cfg(not(desktop))]
    {
        bail!("tachyon-cli is only supported on desktop targets");
    }
}

#[cfg(desktop)]
fn handle_cli<R: tauri::Runtime>(app: &tauri::App<R>) -> Result<()> {
    use tauri_plugin_cli::CliExt;

    let matches = app
        .cli()
        .matches()
        .context("failed to parse Tauri CLI arguments")?;
    let subcommand = matches
        .subcommand
        .context("expected `generate` subcommand, for example `tachyon-cli generate --route /api/guest-example --memory 64`")?;

    if subcommand.name != "generate" {
        bail!(
            "unsupported subcommand `{}`; expected `generate`",
            subcommand.name
        );
    }

    let request = GenerateRequest {
        routes: parse_routes_arg(&subcommand.matches.args)?,
        memory_mib: parse_memory_arg(&subcommand.matches.args)?,
    };

    let manifest_path = generate_manifest(request)?;
    println!("wrote integrity manifest to {}", manifest_path.display());
    Ok(())
}

fn parse_generate_request_from_args(
    args: impl IntoIterator<Item = String>,
) -> Result<Option<GenerateRequest>> {
    let mut args = args.into_iter();
    let Some(subcommand) = args.next() else {
        return Ok(None);
    };

    if subcommand != "generate" {
        bail!("unsupported subcommand `{subcommand}`; expected `generate`");
    }

    let mut routes = Vec::new();
    let mut memory_mib = None;

    while let Some(arg) = args.next() {
        if let Some(value) = arg.strip_prefix("--route=") {
            routes.push(value.to_owned());
            continue;
        }

        if arg == "--route" {
            let route = args
                .next()
                .context("missing value for `--route`; expected `--route /api/guest-example`")?;
            routes.push(route);
            continue;
        }

        if let Some(value) = arg.strip_prefix("--memory=") {
            memory_mib = Some(parse_memory_value(value)?);
            continue;
        }

        if arg == "--memory" {
            let value = args
                .next()
                .context("missing value for `--memory`; expected `--memory 64`")?;
            memory_mib = Some(parse_memory_value(&value)?);
            continue;
        }

        bail!("unexpected argument `{arg}`");
    }

    let memory_mib = memory_mib.context("missing required `--memory` argument")?;

    Ok(Some(GenerateRequest { routes, memory_mib }))
}

fn generate_manifest(request: GenerateRequest) -> Result<PathBuf> {
    let config = SealedConfig::from_request(request)?;
    let config_payload = config.canonical_payload()?;
    let payload_hash = Sha256::digest(config_payload.as_bytes());

    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();
    let signature = signing_key.sign(&payload_hash);

    let manifest = IntegrityManifest {
        config_payload,
        public_key: hex::encode(verifying_key.to_bytes()),
        signature: hex::encode(signature.to_bytes()),
    };

    let manifest_path = workspace_root().join("integrity.lock");
    fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?).with_context(|| {
        format!(
            "failed to write integrity manifest to {}",
            manifest_path.display()
        )
    })?;

    Ok(manifest_path)
}

impl SealedConfig {
    fn from_request(request: GenerateRequest) -> Result<Self> {
        let routes = normalize_routes(request.routes)?;
        let memory_mib = usize::try_from(request.memory_mib)
            .context("memory limit is too large for this platform")?;
        let guest_memory_limit_bytes = memory_mib
            .checked_mul(1024 * 1024)
            .context("memory limit overflowed while converting MiB to bytes")?;

        Ok(Self {
            host_address: DEFAULT_HOST_ADDRESS.to_owned(),
            max_stdout_bytes: DEFAULT_MAX_STDOUT_BYTES,
            guest_fuel_budget: DEFAULT_GUEST_FUEL_BUDGET,
            guest_memory_limit_bytes,
            resource_limit_response: DEFAULT_RESOURCE_LIMIT_RESPONSE.to_owned(),
            routes,
        })
    }

    fn canonical_payload(&self) -> Result<String> {
        serde_json::to_string(self).context("failed to serialize canonical configuration payload")
    }
}

#[cfg(desktop)]
fn parse_routes_arg(
    args: &std::collections::HashMap<String, tauri_plugin_cli::ArgData>,
) -> Result<Vec<String>> {
    let arg = args
        .get("route")
        .context("missing required `--route` argument")?;

    match &arg.value {
        Value::String(route) => Ok(vec![route.clone()]),
        Value::Array(routes) => routes
            .iter()
            .map(|route| {
                route
                    .as_str()
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| anyhow!("route values must be strings"))
            })
            .collect(),
        _ => bail!("`--route` must be provided as one or more strings"),
    }
}

#[cfg(desktop)]
fn parse_memory_arg(
    args: &std::collections::HashMap<String, tauri_plugin_cli::ArgData>,
) -> Result<u32> {
    let arg = args
        .get("memory")
        .context("missing required `--memory` argument")?;

    let value = match &arg.value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        _ => bail!("`--memory` must be provided as a number in MiB"),
    };

    parse_memory_value(&value)
}

fn parse_memory_value(value: &str) -> Result<u32> {
    let memory_mib = value
        .parse::<u32>()
        .with_context(|| format!("failed to parse `--memory {value}` as an unsigned integer"))?;

    if memory_mib == 0 {
        bail!("`--memory` must be greater than zero");
    }

    Ok(memory_mib)
}

fn normalize_routes(routes: Vec<String>) -> Result<Vec<String>> {
    if routes.is_empty() {
        bail!("at least one `--route` value must be provided");
    }

    let mut normalized = BTreeSet::new();

    for route in routes {
        normalized.insert(normalize_route(&route)?);
    }

    Ok(normalized.into_iter().collect())
}

fn normalize_route(route: &str) -> Result<String> {
    let trimmed = route.trim();
    if trimmed.is_empty() {
        bail!("route values cannot be empty");
    }

    let with_leading_slash = if trimmed.starts_with('/') {
        trimmed.to_owned()
    } else {
        format!("/{trimmed}")
    };

    let normalized = with_leading_slash.trim_end_matches('/').to_owned();
    let normalized = if normalized.is_empty() {
        "/".to_owned()
    } else {
        normalized
    };

    if normalized == "/" {
        bail!("route `/` does not resolve to a guest function");
    }

    if resolve_function_name(&normalized).is_none() {
        bail!("route `{normalized}` does not resolve to a guest function name");
    }

    Ok(normalized)
}

fn resolve_function_name(path: &str) -> Option<&str> {
    path.split('/')
        .rev()
        .find(|segment| !segment.is_empty() && *segment != "api")
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("tachyon-cli should live directly under the workspace root")
        .to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_routes_deduplicates_and_sorts() {
        let routes = normalize_routes(vec![
            "/api/guest-example/".to_owned(),
            "api/guest-example".to_owned(),
            "/api/guest-malicious".to_owned(),
        ])
        .expect("routes should normalize");

        assert_eq!(
            routes,
            vec![
                "/api/guest-example".to_owned(),
                "/api/guest-malicious".to_owned()
            ]
        );
    }

    #[test]
    fn sealed_config_payload_includes_routes_and_memory_limit() {
        let config = SealedConfig::from_request(GenerateRequest {
            routes: vec!["/api/guest-example".to_owned()],
            memory_mib: 64,
        })
        .expect("request should produce a sealed config");

        assert_eq!(config.guest_memory_limit_bytes, 64 * 1024 * 1024);
        assert_eq!(config.routes, vec!["/api/guest-example".to_owned()]);

        let payload = config
            .canonical_payload()
            .expect("payload should serialize deterministically");
        assert!(payload.contains("\"routes\":[\"/api/guest-example\"]"));
    }

    #[test]
    fn parse_generate_request_supports_headless_cli_arguments() {
        let request = parse_generate_request_from_args(
            [
                "generate",
                "--route",
                "/api/guest-example",
                "--route=/api/guest-malicious",
                "--memory",
                "64",
            ]
            .into_iter()
            .map(str::to_owned),
        )
        .expect("arguments should parse")
        .expect("subcommand should be detected");

        assert_eq!(
            request,
            GenerateRequest {
                routes: vec![
                    "/api/guest-example".to_owned(),
                    "/api/guest-malicious".to_owned()
                ],
                memory_mib: 64,
            }
        );
    }

    #[test]
    fn parse_generate_request_returns_none_without_arguments() {
        let request = parse_generate_request_from_args(std::iter::empty::<String>())
            .expect("empty arguments should be accepted");

        assert!(request.is_none());
    }
}
