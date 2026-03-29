use anyhow::{anyhow, bail, Context, Result};
use ed25519_dalek::{Signer, SigningKey};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

const DEFAULT_HOST_ADDRESS: &str = "0.0.0.0:8080";
const DEFAULT_MAX_STDOUT_BYTES: usize = 64 * 1024;
const DEFAULT_GUEST_FUEL_BUDGET: u64 = 500_000_000;
const DEFAULT_RESOURCE_LIMIT_RESPONSE: &str = "Execution trapped: Resource limit exceeded";
const DEFAULT_ROUTE_MAX_CONCURRENCY: u32 = 100;

fn default_max_concurrency() -> u32 {
    DEFAULT_ROUTE_MAX_CONCURRENCY
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RouteRole {
    User,
    System,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct SealedRoute {
    pub path: String,
    pub role: RouteRole,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_secrets: Vec<String>,
    #[serde(default)]
    pub min_instances: u32,
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct SealedConfig {
    pub host_address: String,
    pub max_stdout_bytes: usize,
    pub guest_fuel_budget: u64,
    pub guest_memory_limit_bytes: usize,
    pub resource_limit_response: String,
    pub routes: Vec<SealedRoute>,
}

#[derive(Debug, Deserialize, Serialize)]
struct IntegrityManifest {
    config_payload: String,
    public_key: String,
    signature: String,
}

#[derive(Debug, PartialEq, Eq)]
struct GenerateRequest {
    user_routes: Vec<String>,
    system_routes: Vec<String>,
    secret_routes: Vec<String>,
    route_scales: Vec<String>,
    memory_mib: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RouteScaling {
    min_instances: u32,
    max_concurrency: u32,
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
        .context("expected `generate` subcommand, for example `tachyon-cli generate --route /api/guest-example --system-route /metrics --memory 64`")?;

    if subcommand.name != "generate" {
        bail!(
            "unsupported subcommand `{}`; expected `generate`",
            subcommand.name
        );
    }

    let request = GenerateRequest {
        user_routes: parse_required_routes_arg(&subcommand.matches.args, "route")?,
        system_routes: parse_optional_routes_arg(&subcommand.matches.args, "system-route")?,
        secret_routes: parse_optional_routes_arg(&subcommand.matches.args, "secret-route")?,
        route_scales: parse_optional_routes_arg(&subcommand.matches.args, "route-scale")?,
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

    let mut user_routes = Vec::new();
    let mut system_routes = Vec::new();
    let mut secret_routes = Vec::new();
    let mut route_scales = Vec::new();
    let mut memory_mib = None;

    while let Some(arg) = args.next() {
        if let Some(value) = arg.strip_prefix("--route=") {
            user_routes.push(value.to_owned());
            continue;
        }

        if arg == "--route" {
            let route = args
                .next()
                .context("missing value for `--route`; expected `--route /api/guest-example`")?;
            user_routes.push(route);
            continue;
        }

        if let Some(value) = arg.strip_prefix("--system-route=") {
            system_routes.push(value.to_owned());
            continue;
        }

        if arg == "--system-route" {
            let route = args.next().context(
                "missing value for `--system-route`; expected `--system-route /metrics`",
            )?;
            system_routes.push(route);
            continue;
        }

        if let Some(value) = arg.strip_prefix("--secret-route=") {
            secret_routes.push(value.to_owned());
            continue;
        }

        if arg == "--secret-route" {
            let route = args.next().context(
                "missing value for `--secret-route`; expected `--secret-route /api/guest-example=DB_PASS`",
            )?;
            secret_routes.push(route);
            continue;
        }

        if let Some(value) = arg.strip_prefix("--memory=") {
            memory_mib = Some(parse_memory_value(value)?);
            continue;
        }

        if let Some(value) = arg.strip_prefix("--route-scale=") {
            route_scales.push(value.to_owned());
            continue;
        }

        if arg == "--route-scale" {
            let route_scale = args.next().context(
                "missing value for `--route-scale`; expected `--route-scale /api/guest-example=1:8`",
            )?;
            route_scales.push(route_scale);
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
    if user_routes.is_empty() {
        bail!("missing required `--route` argument");
    }

    Ok(Some(GenerateRequest {
        user_routes,
        system_routes,
        secret_routes,
        route_scales,
        memory_mib,
    }))
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
        let routes = normalize_routes(
            request.user_routes,
            request.system_routes,
            request.secret_routes,
            request.route_scales,
        )?;
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
fn parse_required_routes_arg(
    args: &std::collections::HashMap<String, tauri_plugin_cli::ArgData>,
    name: &str,
) -> Result<Vec<String>> {
    let routes = parse_optional_routes_arg(args, name)?;
    if routes.is_empty() {
        bail!("missing required `--{name}` argument");
    }

    Ok(routes)
}

#[cfg(desktop)]
fn parse_optional_routes_arg(
    args: &std::collections::HashMap<String, tauri_plugin_cli::ArgData>,
    name: &str,
) -> Result<Vec<String>> {
    let Some(arg) = args.get(name) else {
        return Ok(Vec::new());
    };

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
        _ => bail!("`--{name}` must be provided as one or more strings"),
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

fn normalize_routes(
    user_routes: Vec<String>,
    system_routes: Vec<String>,
    secret_routes: Vec<String>,
    route_scales: Vec<String>,
) -> Result<Vec<SealedRoute>> {
    if user_routes.is_empty() {
        bail!("at least one `--route` value must be provided");
    }

    let mut normalized: BTreeMap<String, SealedRoute> = BTreeMap::new();

    for route in user_routes {
        let path = normalize_route(&route)?;
        if let Some(existing) = normalized.get(&path) {
            match existing.role {
                RouteRole::User => continue,
                RouteRole::System => {
                    bail!("route `{path}` cannot be declared as both `user` and `system`");
                }
            }
        }

        normalized.insert(
            path.clone(),
            SealedRoute {
                path,
                role: RouteRole::User,
                allowed_secrets: Vec::new(),
                min_instances: 0,
                max_concurrency: default_max_concurrency(),
            },
        );
    }

    for route in system_routes {
        let path = normalize_route(&route)?;
        let sealed_route = SealedRoute {
            path: path.clone(),
            role: RouteRole::System,
            allowed_secrets: Vec::new(),
            min_instances: 0,
            max_concurrency: default_max_concurrency(),
        };

        if let Some(existing) = normalized.get(&path) {
            bail!(
                "route `{}` cannot be declared as both `{}` and `{}`",
                sealed_route.path,
                match existing.role {
                    RouteRole::User => "user",
                    RouteRole::System => "system",
                },
                match sealed_route.role {
                    RouteRole::User => "user",
                    RouteRole::System => "system",
                }
            );
        }

        normalized.insert(path, sealed_route);
    }

    for secret_route in secret_routes {
        let (path, allowed_secrets) = parse_secret_route(&secret_route)?;
        let normalized_path = normalize_route(&path)?;
        let sealed_route = normalized.get_mut(&normalized_path).ok_or_else(|| {
            anyhow!("secret route `{normalized_path}` must also be declared with `--route`")
        })?;

        if sealed_route.role != RouteRole::User {
            bail!("secret route `{normalized_path}` must be declared as a user route");
        }

        sealed_route.allowed_secrets = merge_allowed_secrets(
            std::mem::take(&mut sealed_route.allowed_secrets),
            allowed_secrets,
        );
    }

    for route_scale in route_scales {
        let (path, scaling) = parse_route_scale(&route_scale)?;
        let normalized_path = normalize_route(&path)?;
        let sealed_route = normalized.get_mut(&normalized_path).ok_or_else(|| {
            anyhow!("route scale `{normalized_path}` must target a declared sealed route")
        })?;

        sealed_route.min_instances = scaling.min_instances;
        sealed_route.max_concurrency = scaling.max_concurrency;
    }

    Ok(normalized.into_values().collect())
}

fn parse_secret_route(value: &str) -> Result<(String, Vec<String>)> {
    let (path, secrets) = value.split_once('=').context(
        "secret routes must use the `/path=NAME[,NAME]` syntax, for example `/api/guest-example=DB_PASS`",
    )?;

    let path = path.trim();
    if path.is_empty() {
        bail!("secret routes must include a non-empty path before `=`");
    }

    let secrets = secrets
        .split(',')
        .map(str::trim)
        .filter(|secret| !secret.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    if secrets.is_empty() {
        bail!("secret routes must grant at least one named secret");
    }

    Ok((path.to_owned(), merge_allowed_secrets(Vec::new(), secrets)))
}

fn parse_route_scale(value: &str) -> Result<(String, RouteScaling)> {
    let (path, scaling) = value.split_once('=').context(
        "route scaling must use the `/path=min:max` syntax, for example `/api/guest-example=1:8`",
    )?;
    let (min_instances, max_concurrency) = scaling.split_once(':').context(
        "route scaling must include both `min_instances` and `max_concurrency`, for example `/api/guest-example=1:8`",
    )?;

    let path = path.trim();
    if path.is_empty() {
        bail!("route scaling must include a non-empty path before `=`");
    }

    let min_instances = min_instances.trim().parse::<u32>().with_context(|| {
        format!(
            "failed to parse `{}` as `min_instances` in route scaling override `{value}`",
            min_instances.trim()
        )
    })?;
    let max_concurrency = max_concurrency.trim().parse::<u32>().with_context(|| {
        format!(
            "failed to parse `{}` as `max_concurrency` in route scaling override `{value}`",
            max_concurrency.trim()
        )
    })?;

    if max_concurrency == 0 {
        bail!("route scaling override `{value}` must set `max_concurrency` above zero");
    }

    Ok((
        path.to_owned(),
        RouteScaling {
            min_instances,
            max_concurrency,
        },
    ))
}

fn merge_allowed_secrets(existing: Vec<String>, added: Vec<String>) -> Vec<String> {
    existing
        .into_iter()
        .chain(added)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
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
        let routes = normalize_routes(
            vec![
                "/api/guest-example/".to_owned(),
                "api/guest-example".to_owned(),
                "/api/guest-malicious".to_owned(),
            ],
            vec!["/metrics/".to_owned()],
            Vec::new(),
            Vec::new(),
        )
        .expect("routes should normalize");

        assert_eq!(
            routes,
            vec![
                SealedRoute {
                    path: "/api/guest-example".to_owned(),
                    role: RouteRole::User,
                    allowed_secrets: Vec::new(),
                    min_instances: 0,
                    max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
                },
                SealedRoute {
                    path: "/api/guest-malicious".to_owned(),
                    role: RouteRole::User,
                    allowed_secrets: Vec::new(),
                    min_instances: 0,
                    max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
                },
                SealedRoute {
                    path: "/metrics".to_owned(),
                    role: RouteRole::System,
                    allowed_secrets: Vec::new(),
                    min_instances: 0,
                    max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
                }
            ]
        );
    }

    #[test]
    fn sealed_config_payload_includes_routes_and_memory_limit() {
        let config = SealedConfig::from_request(GenerateRequest {
            user_routes: vec!["/api/guest-example".to_owned()],
            system_routes: vec!["/metrics".to_owned()],
            secret_routes: vec!["/api/guest-example=DB_PASS".to_owned()],
            route_scales: vec!["/api/guest-example=2:16".to_owned()],
            memory_mib: 64,
        })
        .expect("request should produce a sealed config");

        assert_eq!(config.guest_fuel_budget, DEFAULT_GUEST_FUEL_BUDGET);
        assert_eq!(config.guest_memory_limit_bytes, 64 * 1024 * 1024);
        assert_eq!(
            config.routes,
            vec![
                SealedRoute {
                    path: "/api/guest-example".to_owned(),
                    role: RouteRole::User,
                    allowed_secrets: vec!["DB_PASS".to_owned()],
                    min_instances: 2,
                    max_concurrency: 16,
                },
                SealedRoute {
                    path: "/metrics".to_owned(),
                    role: RouteRole::System,
                    allowed_secrets: Vec::new(),
                    min_instances: 0,
                    max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
                }
            ]
        );

        let payload = config
            .canonical_payload()
            .expect("payload should serialize deterministically");
        assert!(payload.contains("\"path\":\"/api/guest-example\""));
        assert!(payload.contains("\"role\":\"system\""));
        assert!(payload.contains("\"allowed_secrets\":[\"DB_PASS\"]"));
        assert!(payload.contains("\"min_instances\":2"));
        assert!(payload.contains("\"max_concurrency\":16"));
    }

    #[test]
    fn parse_generate_request_supports_headless_cli_arguments() {
        let request = parse_generate_request_from_args(
            [
                "generate",
                "--route",
                "/api/guest-example",
                "--route=/api/guest-malicious",
                "--system-route",
                "/metrics",
                "--secret-route",
                "/api/guest-example=DB_PASS,API_KEY",
                "--route-scale",
                "/api/guest-example=1:8",
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
                user_routes: vec![
                    "/api/guest-example".to_owned(),
                    "/api/guest-malicious".to_owned()
                ],
                system_routes: vec!["/metrics".to_owned()],
                secret_routes: vec!["/api/guest-example=DB_PASS,API_KEY".to_owned()],
                route_scales: vec!["/api/guest-example=1:8".to_owned()],
                memory_mib: 64,
            }
        );
    }

    #[test]
    fn normalize_routes_applies_scaling_overrides() {
        let routes = normalize_routes(
            vec!["/api/guest-example".to_owned()],
            Vec::new(),
            Vec::new(),
            vec!["/api/guest-example=3:7".to_owned()],
        )
        .expect("route scaling should normalize");

        assert_eq!(
            routes,
            vec![SealedRoute {
                path: "/api/guest-example".to_owned(),
                role: RouteRole::User,
                allowed_secrets: Vec::new(),
                min_instances: 3,
                max_concurrency: 7,
            }]
        );
    }

    #[test]
    fn normalize_routes_rejects_conflicting_roles_for_same_path() {
        let error = normalize_routes(
            vec!["/metrics".to_owned()],
            vec!["/metrics/".to_owned()],
            Vec::new(),
            Vec::new(),
        )
        .expect_err("same path with different roles should fail");

        assert!(error
            .to_string()
            .contains("cannot be declared as both `user` and `system`"));
    }

    #[test]
    fn normalize_routes_rejects_secret_grants_for_unknown_routes() {
        let error = normalize_routes(
            vec!["/api/guest-example".to_owned()],
            Vec::new(),
            vec!["/api/missing=DB_PASS".to_owned()],
            Vec::new(),
        )
        .expect_err("secret route must target a declared user route");

        assert!(error
            .to_string()
            .contains("must also be declared with `--route`"));
    }

    #[test]
    fn normalize_routes_rejects_scaling_overrides_for_unknown_routes() {
        let error = normalize_routes(
            vec!["/api/guest-example".to_owned()],
            Vec::new(),
            Vec::new(),
            vec!["/api/missing=1:8".to_owned()],
        )
        .expect_err("route scaling must target a declared route");

        assert!(error
            .to_string()
            .contains("must target a declared sealed route"));
    }

    #[test]
    fn normalize_routes_rejects_zero_max_concurrency() {
        let error = normalize_routes(
            vec!["/api/guest-example".to_owned()],
            Vec::new(),
            Vec::new(),
            vec!["/api/guest-example=1:0".to_owned()],
        )
        .expect_err("zero max_concurrency should fail");

        assert!(error
            .to_string()
            .contains("must set `max_concurrency` above zero"));
    }

    #[test]
    fn parse_generate_request_returns_none_without_arguments() {
        let request = parse_generate_request_from_args(std::iter::empty::<String>())
            .expect("empty arguments should be accepted");

        assert!(request.is_none());
    }
}
