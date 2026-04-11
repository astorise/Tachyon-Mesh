use anyhow::{anyhow, bail, Context, Result};
use ed25519_dalek::{Signer, SigningKey};
use rand_core::OsRng;
use semver::{Version, VersionReq};
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
const DEFAULT_ROUTE_VERSION: &str = "0.0.0";
const DEFAULT_TELEMETRY_SAMPLE_RATE: f64 = 0.0;

fn default_max_concurrency() -> u32 {
    DEFAULT_ROUTE_MAX_CONCURRENCY
}

fn default_route_version() -> String {
    DEFAULT_ROUTE_VERSION.to_owned()
}

fn is_default_route_version(version: &String) -> bool {
    version == DEFAULT_ROUTE_VERSION
}

fn default_telemetry_sample_rate() -> f64 {
    DEFAULT_TELEMETRY_SAMPLE_RATE
}

fn is_default_telemetry_sample_rate(sample_rate: &f64) -> bool {
    (*sample_rate - DEFAULT_TELEMETRY_SAMPLE_RATE).abs() < f64::EPSILON
}

fn default_volume_type() -> VolumeType {
    VolumeType::Host
}

fn is_default_volume_type(volume_type: &VolumeType) -> bool {
    *volume_type == VolumeType::Host
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RouteRole {
    User,
    System,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct HeaderMatch {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct RetryPolicy {
    #[serde(default)]
    pub max_retries: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retry_on: Vec<u16>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ResiliencyConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_policy: Option<RetryPolicy>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct RouteTarget {
    pub module: String,
    #[serde(default)]
    pub weight: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub websocket: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_header: Option<HeaderMatch>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VolumeType {
    Host,
    Ram,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VolumeEvictionPolicy {
    Hibernate,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct SealedRoute {
    pub path: String,
    pub role: RouteRole,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(
        default = "default_route_version",
        skip_serializing_if = "is_default_route_version"
    )]
    pub version: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires_credentials: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub middleware: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_secrets: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<RouteTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resiliency: Option<ResiliencyConfig>,
    #[serde(default)]
    pub min_instances: u32,
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub volumes: Vec<SealedVolume>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct SealedLayer4Config {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tcp: Vec<SealedTcpBinding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub udp: Vec<SealedUdpBinding>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct SealedTcpBinding {
    pub port: u16,
    pub target: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct SealedUdpBinding {
    pub port: u16,
    pub target: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct SealedVolume {
    #[serde(
        rename = "type",
        default = "default_volume_type",
        skip_serializing_if = "is_default_volume_type"
    )]
    pub volume_type: VolumeType,
    pub host_path: String,
    pub guest_path: String,
    #[serde(default)]
    pub readonly: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_timeout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eviction_policy: Option<VolumeEvictionPolicy>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct SealedBatchTarget {
    pub name: String,
    pub module: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub volumes: Vec<SealedVolume>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SealedConfig {
    pub host_address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advertise_ip: Option<String>,
    pub max_stdout_bytes: usize,
    pub guest_fuel_budget: u64,
    pub guest_memory_limit_bytes: usize,
    pub resource_limit_response: String,
    #[serde(default, skip_serializing_if = "SealedLayer4Config::is_empty")]
    pub layer4: SealedLayer4Config,
    #[serde(
        default = "default_telemetry_sample_rate",
        skip_serializing_if = "is_default_telemetry_sample_rate"
    )]
    pub telemetry_sample_rate: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub batch_targets: Vec<SealedBatchTarget>,
    pub routes: Vec<SealedRoute>,
}

#[derive(Debug, Deserialize, Serialize)]
struct IntegrityManifest {
    config_payload: String,
    public_key: String,
    signature: String,
}

#[derive(Debug, PartialEq)]
struct GenerateRequest {
    user_routes: Vec<String>,
    system_routes: Vec<String>,
    secret_routes: Vec<String>,
    batch_targets: Vec<String>,
    batch_target_envs: Vec<String>,
    batch_target_volumes: Vec<String>,
    route_targets: Vec<String>,
    route_names: Vec<String>,
    route_versions: Vec<String>,
    route_dependencies: Vec<String>,
    route_credentials: Vec<String>,
    route_middlewares: Vec<String>,
    route_envs: Vec<String>,
    route_scales: Vec<String>,
    tcp_ports: Vec<String>,
    udp_ports: Vec<String>,
    volumes: Vec<String>,
    advertise_ip: Option<String>,
    telemetry_sample_rate: f64,
    memory_mib: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RouteScaling {
    min_instances: u32,
    max_concurrency: u32,
}

impl SealedLayer4Config {
    fn is_empty(&self) -> bool {
        self.tcp.is_empty() && self.udp.is_empty()
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if let Err(error) = run_inner() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

#[tauri::command]
async fn get_engine_status() -> std::result::Result<String, String> {
    tachyon_client::get_engine_status()
        .await
        .map_err(|error| error.to_string())
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
            .plugin(tauri_plugin_cli::init())
            .invoke_handler(tauri::generate_handler![get_engine_status])
            .run(tauri::generate_context!())
            .map_err(|error| anyhow!("tachyon-ui runtime failed: {error}"))?;

        Ok(())
    }

    #[cfg(not(desktop))]
    {
        bail!("tachyon-ui is only supported on desktop targets");
    }
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
    let mut batch_targets = Vec::new();
    let mut batch_target_envs = Vec::new();
    let mut batch_target_volumes = Vec::new();
    let mut route_targets = Vec::new();
    let mut route_names = Vec::new();
    let mut route_versions = Vec::new();
    let mut route_dependencies = Vec::new();
    let mut route_credentials = Vec::new();
    let mut route_middlewares = Vec::new();
    let mut route_envs = Vec::new();
    let mut route_scales = Vec::new();
    let mut tcp_ports = Vec::new();
    let mut udp_ports = Vec::new();
    let mut volumes = Vec::new();
    let mut advertise_ip = None;
    let mut telemetry_sample_rate = DEFAULT_TELEMETRY_SAMPLE_RATE;
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

        if let Some(value) = arg.strip_prefix("--batch-target=") {
            batch_targets.push(value.to_owned());
            continue;
        }

        if arg == "--batch-target" {
            let batch_target = args.next().context(
                "missing value for `--batch-target`; expected `--batch-target gc-job=system-faas-gc`",
            )?;
            batch_targets.push(batch_target);
            continue;
        }

        if let Some(value) = arg.strip_prefix("--batch-target-env=") {
            batch_target_envs.push(value.to_owned());
            continue;
        }

        if arg == "--batch-target-env" {
            let batch_target_env = args.next().context(
                "missing value for `--batch-target-env`; expected `--batch-target-env gc-job=TTL_SECONDS=300`",
            )?;
            batch_target_envs.push(batch_target_env);
            continue;
        }

        if let Some(value) = arg.strip_prefix("--batch-target-volume=") {
            batch_target_volumes.push(value.to_owned());
            continue;
        }

        if arg == "--batch-target-volume" {
            let batch_target_volume = args.next().context(
                "missing value for `--batch-target-volume`; expected `--batch-target-volume gc-job=/tmp/test-cache:/cache:rw`",
            )?;
            batch_target_volumes.push(batch_target_volume);
            continue;
        }

        if let Some(value) = arg.strip_prefix("--route-target=") {
            route_targets.push(value.to_owned());
            continue;
        }

        if arg == "--route-target" {
            let route_target = args.next().context(
                "missing value for `--route-target`; expected `--route-target /api/checkout=checkout-v2,weight=10,websocket=true`",
            )?;
            route_targets.push(route_target);
            continue;
        }

        if let Some(value) = arg.strip_prefix("--route-name=") {
            route_names.push(value.to_owned());
            continue;
        }

        if arg == "--route-name" {
            let route_name = args.next().context(
                "missing value for `--route-name`; expected `--route-name /api/guest-example=guest-example`",
            )?;
            route_names.push(route_name);
            continue;
        }

        if let Some(value) = arg.strip_prefix("--route-version=") {
            route_versions.push(value.to_owned());
            continue;
        }

        if arg == "--route-version" {
            let route_version = args.next().context(
                "missing value for `--route-version`; expected `--route-version /api/guest-example=1.2.3`",
            )?;
            route_versions.push(route_version);
            continue;
        }

        if let Some(value) = arg.strip_prefix("--route-dependency=") {
            route_dependencies.push(value.to_owned());
            continue;
        }

        if arg == "--route-dependency" {
            let route_dependency = args.next().context(
                "missing value for `--route-dependency`; expected `--route-dependency /api/faas-a=faas-b@^3.1.0`",
            )?;
            route_dependencies.push(route_dependency);
            continue;
        }

        if let Some(value) = arg.strip_prefix("--route-credential=") {
            route_credentials.push(value.to_owned());
            continue;
        }

        if arg == "--route-credential" {
            let route_credential = args.next().context(
                "missing value for `--route-credential`; expected `--route-credential /api/faas-a=cred-a,cred-b`",
            )?;
            route_credentials.push(route_credential);
            continue;
        }

        if let Some(value) = arg.strip_prefix("--route-middleware=") {
            route_middlewares.push(value.to_owned());
            continue;
        }

        if arg == "--route-middleware" {
            let route_middleware = args.next().context(
                "missing value for `--route-middleware`; expected `--route-middleware /api/faas-a=system-faas-auth`",
            )?;
            route_middlewares.push(route_middleware);
            continue;
        }

        if let Some(value) = arg.strip_prefix("--route-env=") {
            route_envs.push(value.to_owned());
            continue;
        }

        if arg == "--route-env" {
            let route_env = args.next().context(
                "missing value for `--route-env`; expected `--route-env /system/sqs=QUEUE_URL=https://queue.example`",
            )?;
            route_envs.push(route_env);
            continue;
        }

        if let Some(value) = arg.strip_prefix("--memory=") {
            memory_mib = Some(parse_memory_value(value)?);
            continue;
        }

        if let Some(value) = arg.strip_prefix("--advertise-ip=") {
            advertise_ip = Some(parse_non_empty_arg_value("--advertise-ip", value)?);
            continue;
        }

        if let Some(value) = arg.strip_prefix("--telemetry-sample-rate=") {
            telemetry_sample_rate = parse_telemetry_sample_rate_value(value)?;
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

        if arg == "--advertise-ip" {
            let value = args.next().context(
                "missing value for `--advertise-ip`; expected `--advertise-ip 203.0.113.50`",
            )?;
            advertise_ip = Some(parse_non_empty_arg_value("--advertise-ip", &value)?);
            continue;
        }

        if let Some(value) = arg.strip_prefix("--tcp-port=") {
            tcp_ports.push(value.to_owned());
            continue;
        }

        if arg == "--tcp-port" {
            let binding = args.next().context(
                "missing value for `--tcp-port`; expected `--tcp-port 2222=guest-tcp-echo`",
            )?;
            tcp_ports.push(binding);
            continue;
        }

        if let Some(value) = arg.strip_prefix("--udp-port=") {
            udp_ports.push(value.to_owned());
            continue;
        }

        if arg == "--udp-port" {
            let binding = args.next().context(
                "missing value for `--udp-port`; expected `--udp-port 5353=guest-udp-echo`",
            )?;
            udp_ports.push(binding);
            continue;
        }

        if let Some(value) = arg.strip_prefix("--volume=") {
            volumes.push(value.to_owned());
            continue;
        }

        if arg == "--volume" {
            let volume = args.next().context(
                "missing value for `--volume`; expected `--volume /api/guest-volume=/tmp/tachyon_data:/app/data:rw`",
            )?;
            volumes.push(volume);
            continue;
        }

        if arg == "--telemetry-sample-rate" {
            let value = args.next().context(
                "missing value for `--telemetry-sample-rate`; expected `--telemetry-sample-rate 0.001`",
            )?;
            telemetry_sample_rate = parse_telemetry_sample_rate_value(&value)?;
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
    if user_routes.is_empty() && batch_targets.is_empty() {
        bail!("missing required `--route` or `--batch-target` argument");
    }

    Ok(Some(GenerateRequest {
        user_routes,
        system_routes,
        secret_routes,
        batch_targets,
        batch_target_envs,
        batch_target_volumes,
        route_targets,
        route_names,
        route_versions,
        route_dependencies,
        route_credentials,
        route_middlewares,
        route_envs,
        route_scales,
        tcp_ports,
        udp_ports,
        volumes,
        advertise_ip,
        telemetry_sample_rate,
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
        if !request.telemetry_sample_rate.is_finite()
            || !(0.0..=1.0).contains(&request.telemetry_sample_rate)
        {
            bail!("`telemetry_sample_rate` must be between 0.0 and 1.0");
        }

        let routes = normalize_routes_with_env(
            request.user_routes,
            request.system_routes,
            request.secret_routes,
            !request.batch_targets.is_empty(),
            request.route_scales,
            request.volumes,
            request.route_targets,
            request.route_names,
            request.route_versions,
            request.route_dependencies,
            request.route_credentials,
            request.route_middlewares,
            request.route_envs,
        )?;
        let batch_targets = normalize_batch_targets(
            request.batch_targets,
            request.batch_target_envs,
            request.batch_target_volumes,
        )?;
        let layer4 = normalize_layer4_bindings(request.tcp_ports, request.udp_ports, &routes)?;
        let memory_mib = usize::try_from(request.memory_mib)
            .context("memory limit is too large for this platform")?;
        let guest_memory_limit_bytes = memory_mib
            .checked_mul(1024 * 1024)
            .context("memory limit overflowed while converting MiB to bytes")?;

        Ok(Self {
            host_address: DEFAULT_HOST_ADDRESS.to_owned(),
            advertise_ip: request.advertise_ip,
            max_stdout_bytes: DEFAULT_MAX_STDOUT_BYTES,
            guest_fuel_budget: DEFAULT_GUEST_FUEL_BUDGET,
            guest_memory_limit_bytes,
            resource_limit_response: DEFAULT_RESOURCE_LIMIT_RESPONSE.to_owned(),
            layer4,
            telemetry_sample_rate: request.telemetry_sample_rate,
            batch_targets,
            routes,
        })
    }

    fn canonical_payload(&self) -> Result<String> {
        serde_json::to_string(self).context("failed to serialize canonical configuration payload")
    }
}

#[cfg(desktop)]
#[allow(dead_code)]
fn parse_required_routes_arg(
    args: &std::collections::HashMap<String, tauri_plugin_cli::ArgData>,
    name: &str,
) -> Result<Vec<String>> {
    let routes = parse_optional_string_args(args, name, "route")?;
    if routes.is_empty() {
        bail!("missing required `--{name}` argument");
    }

    Ok(routes)
}

#[cfg(desktop)]
fn parse_optional_string_args(
    args: &std::collections::HashMap<String, tauri_plugin_cli::ArgData>,
    name: &str,
    label: &str,
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
                    .ok_or_else(|| anyhow!("{label} values must be strings"))
            })
            .collect(),
        _ => bail!("`--{name}` must be provided as one or more strings"),
    }
}

fn parse_non_empty_arg_value(flag: &str, value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("`{flag}` must not be empty");
    }
    Ok(trimmed.to_owned())
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

fn parse_telemetry_sample_rate_value(value: &str) -> Result<f64> {
    let sample_rate = value.parse::<f64>().with_context(|| {
        format!("failed to parse `--telemetry-sample-rate {value}` as a floating-point number")
    })?;

    if !sample_rate.is_finite() || !(0.0..=1.0).contains(&sample_rate) {
        bail!("`--telemetry-sample-rate` must be between 0.0 and 1.0");
    }

    Ok(sample_rate)
}

#[cfg_attr(not(test), allow(dead_code))]
#[allow(clippy::too_many_arguments)]
fn normalize_routes(
    user_routes: Vec<String>,
    system_routes: Vec<String>,
    secret_routes: Vec<String>,
    allow_empty: bool,
    route_scales: Vec<String>,
    route_volumes: Vec<String>,
    route_targets: Vec<String>,
    route_names: Vec<String>,
    route_versions: Vec<String>,
    route_dependencies: Vec<String>,
    route_credentials: Vec<String>,
    route_middlewares: Vec<String>,
) -> Result<Vec<SealedRoute>> {
    normalize_routes_with_env(
        user_routes,
        system_routes,
        secret_routes,
        allow_empty,
        route_scales,
        route_volumes,
        route_targets,
        route_names,
        route_versions,
        route_dependencies,
        route_credentials,
        route_middlewares,
        Vec::new(),
    )
}

#[allow(clippy::too_many_arguments)]
fn normalize_routes_with_env(
    user_routes: Vec<String>,
    system_routes: Vec<String>,
    secret_routes: Vec<String>,
    allow_empty: bool,
    route_scales: Vec<String>,
    route_volumes: Vec<String>,
    route_targets: Vec<String>,
    route_names: Vec<String>,
    route_versions: Vec<String>,
    route_dependencies: Vec<String>,
    route_credentials: Vec<String>,
    route_middlewares: Vec<String>,
    route_envs: Vec<String>,
) -> Result<Vec<SealedRoute>> {
    if user_routes.is_empty() {
        if allow_empty {
            return Ok(Vec::new());
        }
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
                path: path.clone(),
                role: RouteRole::User,
                name: default_route_name(&path),
                version: default_route_version(),
                dependencies: BTreeMap::new(),
                requires_credentials: Vec::new(),
                middleware: None,
                env: BTreeMap::new(),
                allowed_secrets: Vec::new(),
                targets: Vec::new(),
                resiliency: None,
                min_instances: 0,
                max_concurrency: default_max_concurrency(),
                volumes: Vec::new(),
            },
        );
    }

    for route in system_routes {
        let path = normalize_route(&route)?;
        let sealed_route = SealedRoute {
            path: path.clone(),
            role: RouteRole::System,
            name: default_route_name(&path),
            version: default_route_version(),
            dependencies: BTreeMap::new(),
            requires_credentials: Vec::new(),
            middleware: None,
            env: BTreeMap::new(),
            allowed_secrets: Vec::new(),
            targets: Vec::new(),
            resiliency: None,
            min_instances: 0,
            max_concurrency: default_max_concurrency(),
            volumes: Vec::new(),
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

    for route_name in route_names {
        let (path, name) = parse_route_name(&route_name)?;
        let normalized_path = normalize_route(&path)?;
        let sealed_route = normalized.get_mut(&normalized_path).ok_or_else(|| {
            anyhow!("route name `{normalized_path}` must target a declared sealed route")
        })?;
        sealed_route.name = normalize_route_name(&name)?;
    }

    for route_version in route_versions {
        let (path, version) = parse_route_version(&route_version)?;
        let normalized_path = normalize_route(&path)?;
        let sealed_route = normalized.get_mut(&normalized_path).ok_or_else(|| {
            anyhow!("route version `{normalized_path}` must target a declared sealed route")
        })?;
        sealed_route.version = version;
    }

    for route_dependency in route_dependencies {
        let (path, dependency, requirement) = parse_route_dependency(&route_dependency)?;
        let normalized_path = normalize_route(&path)?;
        let sealed_route = normalized.get_mut(&normalized_path).ok_or_else(|| {
            anyhow!("route dependency `{normalized_path}` must target a declared sealed route")
        })?;
        insert_route_dependency(sealed_route, dependency, requirement)?;
    }

    for route_credential in route_credentials {
        let (path, credentials) = parse_route_credentials(&route_credential)?;
        let normalized_path = normalize_route(&path)?;
        let sealed_route = normalized.get_mut(&normalized_path).ok_or_else(|| {
            anyhow!("route credential `{normalized_path}` must target a declared sealed route")
        })?;
        sealed_route.requires_credentials = merge_route_credentials(
            std::mem::take(&mut sealed_route.requires_credentials),
            credentials,
        );
    }

    for route_middleware in route_middlewares {
        let (path, middleware) = parse_route_middleware(&route_middleware)?;
        let normalized_path = normalize_route(&path)?;
        let sealed_route = normalized.get_mut(&normalized_path).ok_or_else(|| {
            anyhow!("route middleware `{normalized_path}` must target a declared sealed route")
        })?;
        sealed_route.middleware = Some(middleware);
    }

    for route_env in route_envs {
        let (path, key, value) = parse_route_env(&route_env)?;
        let normalized_path = normalize_route(&path)?;
        let sealed_route = normalized.get_mut(&normalized_path).ok_or_else(|| {
            anyhow!("route env `{normalized_path}` must target a declared sealed route")
        })?;
        sealed_route.env.insert(key, value);
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

    for route_target in route_targets {
        let (path, target) = parse_route_target(&route_target)?;
        let normalized_path = normalize_route(&path)?;
        let sealed_route = normalized.get_mut(&normalized_path).ok_or_else(|| {
            anyhow!("route target `{normalized_path}` must target a declared sealed route")
        })?;
        insert_route_target(sealed_route, target);
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

    let sealed_route_count = normalized.len();
    for route_volume in route_volumes {
        let (path, volume) = parse_route_volume(&route_volume, sealed_route_count)?;
        let normalized_path = match path {
            Some(path) => normalize_route(&path)?,
            None => normalized
                .keys()
                .next()
                .cloned()
                .context("volume mounts require at least one sealed route")?,
        };
        let sealed_route = normalized.get_mut(&normalized_path).ok_or_else(|| {
            anyhow!("volume route `{normalized_path}` must target a declared sealed route")
        })?;
        insert_route_volume(sealed_route, volume)?;
    }

    normalized
        .into_values()
        .map(finalize_route)
        .collect::<Result<Vec<_>>>()
}

fn normalize_batch_targets(
    batch_targets: Vec<String>,
    batch_target_envs: Vec<String>,
    batch_target_volumes: Vec<String>,
) -> Result<Vec<SealedBatchTarget>> {
    let mut normalized = BTreeMap::<String, SealedBatchTarget>::new();

    for batch_target in batch_targets {
        let (name, module) = parse_batch_target(&batch_target)?;
        normalized.insert(
            name.clone(),
            SealedBatchTarget {
                name,
                module,
                env: BTreeMap::new(),
                volumes: Vec::new(),
            },
        );
    }

    for env in batch_target_envs {
        let (name, key, value) = parse_batch_target_env(&env)?;
        let target = normalized.get_mut(&name).ok_or_else(|| {
            anyhow!("batch target env `{name}` must target a declared batch target")
        })?;
        target.env.insert(key, value);
    }

    for volume in batch_target_volumes {
        let (name, parsed_volume) = parse_batch_target_volume(&volume)?;
        let target = normalized.get_mut(&name).ok_or_else(|| {
            anyhow!("batch target volume `{name}` must target a declared batch target")
        })?;
        insert_batch_target_volume(target, parsed_volume)?;
    }

    Ok(normalized.into_values().collect())
}

fn normalize_layer4_bindings(
    tcp_bindings: Vec<String>,
    udp_bindings: Vec<String>,
    routes: &[SealedRoute],
) -> Result<SealedLayer4Config> {
    let route_names = routes
        .iter()
        .map(|route| route.name.clone())
        .collect::<BTreeSet<_>>();
    let mut normalized_tcp = tcp_bindings
        .into_iter()
        .map(|binding| {
            let (port, target) = parse_tcp_binding(&binding)?;
            if !route_names.contains(&target) {
                bail!("TCP Layer 4 target `{target}` must reference a declared sealed route name");
            }
            Ok(SealedTcpBinding { port, target })
        })
        .collect::<Result<Vec<_>>>()?;

    normalized_tcp.sort_by_key(|binding| binding.port);
    for pair in normalized_tcp.windows(2) {
        if pair[0].port == pair[1].port {
            bail!(
                "TCP Layer 4 port `{}` is defined more than once",
                pair[0].port
            );
        }
    }

    let mut normalized_udp = udp_bindings
        .into_iter()
        .map(|binding| {
            let (port, target) = parse_udp_binding(&binding)?;
            if !route_names.contains(&target) {
                bail!("UDP Layer 4 target `{target}` must reference a declared sealed route name");
            }
            Ok(SealedUdpBinding { port, target })
        })
        .collect::<Result<Vec<_>>>()?;

    normalized_udp.sort_by_key(|binding| binding.port);
    for pair in normalized_udp.windows(2) {
        if pair[0].port == pair[1].port {
            bail!(
                "UDP Layer 4 port `{}` is defined more than once",
                pair[0].port
            );
        }
    }

    Ok(SealedLayer4Config {
        tcp: normalized_tcp,
        udp: normalized_udp,
    })
}

fn parse_tcp_binding(value: &str) -> Result<(u16, String)> {
    let trimmed = value.trim();
    let (port, target) = trimmed
        .split_once('=')
        .context("TCP Layer 4 bindings must use the `PORT=TARGET` syntax")?;
    let port = port
        .trim()
        .parse::<u16>()
        .with_context(|| format!("failed to parse TCP Layer 4 port `{port}`"))?;
    if port == 0 {
        bail!("TCP Layer 4 bindings must use a port above zero");
    }

    let target = normalize_route_name(target.trim())
        .context("TCP Layer 4 targets must use a non-empty sealed route name")?;
    Ok((port, target))
}

fn parse_udp_binding(value: &str) -> Result<(u16, String)> {
    let trimmed = value.trim();
    let (port, target) = trimmed
        .split_once('=')
        .context("UDP Layer 4 bindings must use the `PORT=TARGET` syntax")?;
    let port = port
        .trim()
        .parse::<u16>()
        .with_context(|| format!("failed to parse UDP Layer 4 port `{port}`"))?;
    if port == 0 {
        bail!("UDP Layer 4 bindings must use a port above zero");
    }

    let target = normalize_route_name(target.trim())
        .context("UDP Layer 4 targets must use a non-empty sealed route name")?;
    Ok((port, target))
}

fn parse_batch_target(value: &str) -> Result<(String, String)> {
    let trimmed = value.trim();
    let (name, module) = trimmed
        .split_once('=')
        .context("batch targets must use the `NAME=MODULE` syntax")?;
    let name = normalize_route_name(name.trim())
        .context("batch targets must use a non-empty target name")?;
    let module = normalize_route_name(module.trim())
        .context("batch targets must use a non-empty module name")?;
    Ok((name, module))
}

fn parse_batch_target_env(value: &str) -> Result<(String, String, String)> {
    let trimmed = value.trim();
    let (name, remainder) = trimmed
        .split_once('=')
        .context("batch target env values must use the `NAME=KEY=VALUE` syntax")?;
    let (key, env_value) = remainder
        .split_once('=')
        .context("batch target env values must use the `NAME=KEY=VALUE` syntax")?;
    let name = normalize_route_name(name.trim())
        .context("batch target env values must use a non-empty target name")?;
    let key = key.trim();
    if key.is_empty() {
        bail!("batch target env values must include a non-empty key");
    }
    Ok((name, key.to_owned(), env_value.trim().to_owned()))
}

fn parse_batch_target_volume(value: &str) -> Result<(String, SealedVolume)> {
    let trimmed = value.trim();
    let (name, volume) = trimmed
        .split_once('=')
        .context("batch target volume values must use the `NAME=HOST:GUEST[:ro|rw]` syntax")?;
    let name = normalize_route_name(name.trim())
        .context("batch target volumes must use a non-empty target name")?;
    Ok((name, parse_volume_spec(volume)?))
}

fn parse_route_volume(
    value: &str,
    sealed_route_count: usize,
) -> Result<(Option<String>, SealedVolume)> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("volume values cannot be empty");
    }

    let route_separator = trimmed.find('=');
    let mapping_separator = trimmed.find(":/");
    let (route_path, volume_spec) = match (route_separator, mapping_separator) {
        (Some(route_separator), Some(mapping_separator)) if route_separator < mapping_separator => {
            let (path, volume_spec) = trimmed.split_at(route_separator);
            let volume_spec = &volume_spec[1..];
            let path = path.trim();
            if path.is_empty() {
                bail!("volume values must include a non-empty route before `=`");
            }
            (Some(path.to_owned()), volume_spec.trim())
        }
        _ => {
            if sealed_route_count != 1 {
                bail!(
                    "volume `{trimmed}` must target a declared sealed route using `/path=HOST:GUEST[:ro|rw]` when more than one route is configured"
                );
            }
            (None, trimmed)
        }
    };

    Ok((route_path, parse_volume_spec(volume_spec)?))
}

fn parse_volume_spec(value: &str) -> Result<SealedVolume> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("volume definitions cannot be empty");
    }

    let mut segments = trimmed.split(',');
    let mapping_segment = segments
        .next()
        .context("volume definitions cannot be empty")?
        .trim();
    let (mapping, readonly) = match mapping_segment.rsplit_once(':') {
        Some((mapping, mode))
            if matches!(mode.trim().to_ascii_lowercase().as_str(), "ro" | "rw") =>
        {
            (mapping.trim(), mode.trim().eq_ignore_ascii_case("ro"))
        }
        _ => (mapping_segment, false),
    };

    let separator = mapping.rfind(":/").context(
        "volumes must use the `HOST:GUEST[:ro|rw]` syntax, for example `/tmp/tachyon_data:/app/data:rw`",
    )?;
    let host_path = mapping[..separator].trim();
    if host_path.is_empty() {
        bail!("volume definitions must include a non-empty host path");
    }

    let mut volume_type = VolumeType::Host;
    let mut ttl_seconds = None;
    let mut idle_timeout = None;
    let mut eviction_policy = None;
    for option in segments {
        let (key, option_value) = option
            .split_once('=')
            .context("volume options must use the `key=value` syntax")?;
        let key = key.trim();
        let option_value = option_value.trim();
        if option_value.is_empty() {
            bail!("volume option `{key}` must include a non-empty value");
        }

        match key {
            "type" => {
                volume_type = match option_value {
                    "host" => VolumeType::Host,
                    "ram" => VolumeType::Ram,
                    _ => bail!("volume `type` must be `host` or `ram`"),
                };
            }
            "ttl_seconds" => {
                ttl_seconds = Some(parse_ttl_seconds(option_value)?);
            }
            "idle_timeout" => {
                validate_idle_timeout(option_value)?;
                idle_timeout = Some(option_value.to_owned());
            }
            "eviction_policy" => {
                eviction_policy = Some(match option_value {
                    "hibernate" => VolumeEvictionPolicy::Hibernate,
                    _ => bail!("volume `eviction_policy` must be `hibernate`"),
                });
            }
            _ => bail!("unsupported volume option `{key}`"),
        }
    }

    if idle_timeout.is_some() && volume_type != VolumeType::Ram {
        bail!("volume `idle_timeout` is only valid for `type=ram`");
    }
    if eviction_policy.is_some() && volume_type != VolumeType::Ram {
        bail!("volume `eviction_policy` is only valid for `type=ram`");
    }
    if eviction_policy == Some(VolumeEvictionPolicy::Hibernate) && idle_timeout.is_none() {
        bail!("volume `eviction_policy=hibernate` requires `idle_timeout`");
    }

    Ok(SealedVolume {
        volume_type,
        host_path: host_path.to_owned(),
        guest_path: normalize_guest_volume_path(&mapping[separator + 1..])?,
        readonly,
        ttl_seconds,
        idle_timeout,
        eviction_policy,
    })
}

fn normalize_guest_volume_path(path: &str) -> Result<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        bail!("volume definitions must include a non-empty guest path");
    }
    if !trimmed.starts_with('/') {
        bail!("guest volume paths must be absolute, for example `/app/data`");
    }
    if trimmed.contains('\\') {
        bail!("guest volume paths must use `/` separators");
    }

    let normalized = trimmed.trim_end_matches('/');
    let normalized = if normalized.is_empty() {
        "/".to_owned()
    } else {
        normalized.to_owned()
    };

    if normalized == "/" {
        bail!("guest volume path `/` is not allowed");
    }
    if normalized
        .split('/')
        .skip(1)
        .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
    {
        bail!("guest volume paths cannot contain empty, `.` or `..` segments");
    }

    Ok(normalized)
}

fn insert_route_volume(route: &mut SealedRoute, volume: SealedVolume) -> Result<()> {
    if route.role == RouteRole::User && volume.volume_type == VolumeType::Host && !volume.readonly {
        bail!(
            "route `{}` is a user route and cannot request writable host mounts; use `:ro` and delegate writes through a system storage broker",
            route.path
        );
    }

    if let Some(existing) = route
        .volumes
        .iter()
        .find(|existing| existing.guest_path == volume.guest_path)
    {
        if existing == &volume {
            return Ok(());
        }

        bail!(
            "route `{}` defines guest volume path `{}` more than once",
            route.path,
            volume.guest_path
        );
    }

    route.volumes.push(volume);
    route
        .volumes
        .sort_by(|left, right| left.guest_path.cmp(&right.guest_path));
    Ok(())
}

fn insert_batch_target_volume(target: &mut SealedBatchTarget, volume: SealedVolume) -> Result<()> {
    if let Some(existing) = target
        .volumes
        .iter()
        .find(|existing| existing.guest_path == volume.guest_path)
    {
        if existing == &volume {
            return Ok(());
        }

        bail!(
            "batch target `{}` defines guest volume path `{}` more than once",
            target.name,
            volume.guest_path
        );
    }

    target.volumes.push(volume);
    target
        .volumes
        .sort_by(|left, right| left.guest_path.cmp(&right.guest_path));
    Ok(())
}

fn validate_idle_timeout(value: &str) -> Result<()> {
    let trimmed = value.trim();
    let (digits, suffix) = if let Some(value) = trimmed.strip_suffix("ms") {
        (value.trim(), "ms")
    } else if let Some(value) = trimmed.strip_suffix('s') {
        (value.trim(), "s")
    } else if let Some(value) = trimmed.strip_suffix('m') {
        (value.trim(), "m")
    } else {
        bail!("volume `idle_timeout` must use one of the `ms`, `s`, or `m` suffixes");
    };

    let amount = digits
        .parse::<u64>()
        .with_context(|| format!("failed to parse volume `idle_timeout {trimmed}`"))?;
    if amount == 0 {
        bail!("volume `idle_timeout` must be greater than zero");
    }

    let _ = suffix;
    Ok(())
}

fn parse_ttl_seconds(value: &str) -> Result<u64> {
    let ttl_seconds = value
        .trim()
        .parse::<u64>()
        .with_context(|| format!("failed to parse volume `ttl_seconds {value}`"))?;
    if ttl_seconds == 0 {
        bail!("volume `ttl_seconds` must be greater than zero");
    }

    Ok(ttl_seconds)
}

fn insert_route_target(route: &mut SealedRoute, target: RouteTarget) {
    route.targets.push(target);
}

fn normalize_route_capabilities(capabilities: Vec<String>, context: &str) -> Result<Vec<String>> {
    let mut normalized = BTreeSet::new();
    let source = if capabilities.is_empty() {
        vec!["core:wasi".to_owned()]
    } else {
        capabilities
    };
    for capability in source {
        let trimmed = capability.trim();
        if trimmed.is_empty() {
            bail!("{context} must not contain empty capabilities");
        }
        let canonical = trimmed.to_ascii_lowercase();
        if !matches!(
            canonical.as_str(),
            "core:wasi"
                | "legacy:oci"
                | "accel:cuda"
                | "accel:openvino"
                | "accel:tpu"
                | "net:layer4"
                | "feature:websockets"
                | "feature:http3"
                | "feature:ai-inference"
                | "os:linux"
                | "os:windows"
        ) {
            bail!("{context} declares unsupported capability `{canonical}`");
        }
        normalized.insert(canonical);
    }
    Ok(normalized.into_iter().collect())
}

fn normalize_route_target(target: RouteTarget) -> Result<RouteTarget> {
    Ok(RouteTarget {
        module: normalize_target_module(&target.module)?,
        weight: target.weight,
        websocket: target.websocket,
        match_header: target.match_header,
        requires: normalize_route_capabilities(
            target.requires,
            &format!("route target `{}` capabilities", target.module),
        )?,
    })
}

fn normalize_route_targets(targets: Vec<RouteTarget>) -> Result<Vec<RouteTarget>> {
    targets
        .into_iter()
        .map(normalize_route_target)
        .collect::<Result<Vec<_>>>()
}

fn insert_route_dependency(
    route: &mut SealedRoute,
    dependency: String,
    requirement: String,
) -> Result<()> {
    if let Some(existing) = route.dependencies.get(&dependency) {
        if existing == &requirement {
            return Ok(());
        }

        bail!(
            "route `{}` defines dependency `{dependency}` more than once",
            route.path
        );
    }

    route.dependencies.insert(dependency, requirement);
    Ok(())
}

fn finalize_route(route: SealedRoute) -> Result<SealedRoute> {
    let mut route = route;
    route.targets = normalize_route_targets(route.targets)?;

    if route.targets.is_empty() && resolve_function_name(&route.path).is_none() {
        bail!(
            "route `{}` does not resolve to a guest function name and must define at least one `--route-target`",
            route.path
        );
    }

    if route
        .middleware
        .as_ref()
        .is_some_and(|middleware| middleware == &route.name)
    {
        bail!(
            "route `{}` cannot use itself (`{}`) as middleware",
            route.path,
            route.name
        );
    }

    Ok(route)
}

fn default_route_name(path: &str) -> String {
    resolve_function_name(path)
        .unwrap_or_else(|| path.trim_matches('/'))
        .to_owned()
}

fn normalize_route_name(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("route names must include a non-empty service name");
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        bail!("route names must not contain path separators");
    }

    Ok(trimmed.to_owned())
}

fn parse_route_name(value: &str) -> Result<(String, String)> {
    let (path, name) = value.split_once('=').context(
        "route names must use the `/path=NAME` syntax, for example `/api/faas-b-v2=faas-b`",
    )?;
    let path = path.trim();
    if path.is_empty() {
        bail!("route names must include a non-empty path before `=`");
    }

    Ok((path.to_owned(), name.trim().to_owned()))
}

fn parse_route_version(value: &str) -> Result<(String, String)> {
    let (path, version) = value.split_once('=').context(
        "route versions must use the `/path=VERSION` syntax, for example `/api/faas-b-v2=2.1.0`",
    )?;
    let path = path.trim();
    if path.is_empty() {
        bail!("route versions must include a non-empty path before `=`");
    }
    let version = Version::parse(version.trim()).with_context(|| {
        format!(
            "failed to parse route version `{}` in `{value}` as semantic version",
            version.trim()
        )
    })?;

    Ok((path.to_owned(), version.to_string()))
}

fn parse_route_dependency(value: &str) -> Result<(String, String, String)> {
    let (path, dependency_spec) = value.split_once('=').context(
        "route dependencies must use the `/path=NAME@REQ` syntax, for example `/api/faas-a=faas-b@^3.1.0`",
    )?;
    let path = path.trim();
    if path.is_empty() {
        bail!("route dependencies must include a non-empty path before `=`");
    }

    let (dependency, requirement) = dependency_spec.trim().split_once('@').context(
        "route dependencies must use the `/path=NAME@REQ` syntax, for example `/api/faas-a=faas-b@^3.1.0`",
    )?;
    let dependency = normalize_route_name(dependency)?;
    let requirement = VersionReq::parse(requirement.trim()).with_context(|| {
        format!(
            "failed to parse route dependency requirement `{}` in `{value}`",
            requirement.trim()
        )
    })?;

    Ok((path.to_owned(), dependency, requirement.to_string()))
}

fn parse_route_credentials(value: &str) -> Result<(String, Vec<String>)> {
    let (path, raw_credentials) = value.split_once('=').context(
        "route credentials must use the `/path=CRED[,CRED]` syntax, for example `/api/faas-a=cred-a,cred-b`",
    )?;
    let path = path.trim();
    if path.is_empty() {
        bail!("route credentials must include a non-empty path before `=`");
    }

    let credentials = raw_credentials
        .split(',')
        .map(normalize_route_credential)
        .collect::<Result<Vec<_>>>()?;
    if credentials.is_empty() {
        bail!("route credentials must include at least one credential name");
    }

    Ok((path.to_owned(), credentials))
}

fn parse_route_middleware(value: &str) -> Result<(String, String)> {
    let (path, middleware) = value.split_once('=').context(
        "route middleware must use the `/path=NAME` syntax, for example `/api/faas-a=system-faas-auth`",
    )?;
    let path = path.trim();
    if path.is_empty() {
        bail!("route middleware must include a non-empty path before `=`");
    }

    Ok((path.to_owned(), normalize_route_name(middleware)?))
}

fn parse_route_env(value: &str) -> Result<(String, String, String)> {
    let trimmed = value.trim();
    let (path, remainder) = trimmed.split_once('=').context(
        "route env values must use the `/path=KEY=VALUE` syntax, for example `/system/sqs=QUEUE_URL=https://queue.example`",
    )?;
    let (key, env_value) = remainder.split_once('=').context(
        "route env values must use the `/path=KEY=VALUE` syntax, for example `/system/sqs=QUEUE_URL=https://queue.example`",
    )?;
    let path = normalize_route(path)?;
    let key = key.trim();
    if key.is_empty() {
        bail!("route env values must include a non-empty key");
    }
    Ok((path, key.to_owned(), env_value.trim().to_owned()))
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

fn parse_route_target(value: &str) -> Result<(String, RouteTarget)> {
    let (path, target) = value.split_once('=').context(
        "route targets must use the `/path=MODULE[,weight=80][,header=X-Cohort=beta][,websocket=true][,requires=core:wasi+net:layer4]` syntax",
    )?;

    let path = path.trim();
    if path.is_empty() {
        bail!("route targets must include a non-empty path before `=`");
    }

    let mut segments = target.split(',').map(str::trim);
    let module = normalize_target_module(
        segments
            .next()
            .context("route targets must include a module name after `=`")?,
    )?;
    let mut weight = None;
    let mut websocket = None;
    let mut match_header = None;
    let mut requires = None;

    for segment in segments {
        if segment.is_empty() {
            continue;
        }

        if let Some(raw_weight) = segment.strip_prefix("weight=") {
            if weight.is_some() {
                bail!("route target `{value}` defines `weight` more than once");
            }

            let parsed_weight = raw_weight.trim().parse::<u32>().with_context(|| {
                format!(
                    "failed to parse route target weight `{}` in `{value}`",
                    raw_weight.trim()
                )
            })?;
            if parsed_weight > 100 {
                bail!("route target `{value}` must keep `weight` between 0 and 100");
            }
            weight = Some(parsed_weight);
            continue;
        }

        if let Some(raw_header) = segment.strip_prefix("header=") {
            if match_header.is_some() {
                bail!("route target `{value}` defines `header` more than once");
            }
            match_header = Some(parse_header_match(raw_header)?);
            continue;
        }

        if let Some(raw_websocket) = segment.strip_prefix("websocket=") {
            if websocket.is_some() {
                bail!("route target `{value}` defines `websocket` more than once");
            }
            websocket = Some(parse_bool_option(raw_websocket, value, "websocket")?);
            continue;
        }

        if let Some(raw_requires) = segment.strip_prefix("requires=") {
            if requires.is_some() {
                bail!("route target `{value}` defines `requires` more than once");
            }
            let parsed = raw_requires
                .split('+')
                .map(str::trim)
                .filter(|capability| !capability.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            requires = Some(normalize_route_capabilities(
                parsed,
                &format!("route target `{value}` capabilities"),
            )?);
            continue;
        }

        bail!(
            "unsupported route target option `{segment}`; expected `weight=`, `header=`, `websocket=` or `requires=`"
        );
    }

    Ok((
        path.to_owned(),
        RouteTarget {
            module,
            weight: weight.unwrap_or_else(|| u32::from(match_header.is_none()) * 100),
            websocket: websocket.unwrap_or(false),
            match_header,
            requires: requires.unwrap_or_else(|| vec!["core:wasi".to_owned()]),
        },
    ))
}

fn parse_bool_option(raw: &str, original: &str, option: &str) -> Result<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => bail!(
            "route target `{original}` has invalid `{option}` value `{}`",
            raw.trim()
        ),
    }
}

fn normalize_target_module(module: &str) -> Result<String> {
    let trimmed = module.trim();
    if trimmed.is_empty() {
        bail!("route targets must include a non-empty module name");
    }

    let normalized = trimmed.strip_suffix(".wasm").unwrap_or(trimmed).trim();
    if normalized.is_empty() {
        bail!("route targets must include a non-empty module name");
    }
    if normalized.contains('/') || normalized.contains('\\') {
        bail!("route targets must use module names, not filesystem paths");
    }

    Ok(normalized.to_owned())
}

fn parse_header_match(value: &str) -> Result<HeaderMatch> {
    let (name, header_value) = value.split_once('=').context(
        "route target headers must use the `header=NAME=VALUE` syntax, for example `header=X-Cohort=beta`",
    )?;
    let name = name.trim();
    let header_value = header_value.trim();

    if name.is_empty() {
        bail!("route target headers must include a non-empty header name");
    }
    if header_value.is_empty() {
        bail!("route target headers must include a non-empty header value");
    }

    Ok(HeaderMatch {
        name: name.to_ascii_lowercase(),
        value: header_value.to_owned(),
    })
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

fn merge_route_credentials(existing: Vec<String>, added: Vec<String>) -> Vec<String> {
    existing
        .into_iter()
        .chain(added)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn normalize_route_credential(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("credential names must be non-empty");
    }
    if trimmed.contains(',') {
        bail!("credential names must not contain commas");
    }

    Ok(trimmed.to_owned())
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
        bail!("route `/` is not allowed");
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
        .expect("tachyon-ui should live directly under the workspace root")
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
            false,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
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
                    name: "guest-example".to_owned(),
                    version: default_route_version(),
                    dependencies: BTreeMap::new(),
                    requires_credentials: Vec::new(),
                    middleware: None,
                    env: BTreeMap::new(),
                    allowed_secrets: Vec::new(),
                    targets: Vec::new(),
                    resiliency: None,
                    min_instances: 0,
                    max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
                    volumes: Vec::new(),
                },
                SealedRoute {
                    path: "/api/guest-malicious".to_owned(),
                    role: RouteRole::User,
                    name: "guest-malicious".to_owned(),
                    version: default_route_version(),
                    dependencies: BTreeMap::new(),
                    requires_credentials: Vec::new(),
                    middleware: None,
                    env: BTreeMap::new(),
                    allowed_secrets: Vec::new(),
                    targets: Vec::new(),
                    resiliency: None,
                    min_instances: 0,
                    max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
                    volumes: Vec::new(),
                },
                SealedRoute {
                    path: "/metrics".to_owned(),
                    role: RouteRole::System,
                    name: "metrics".to_owned(),
                    version: default_route_version(),
                    dependencies: BTreeMap::new(),
                    requires_credentials: Vec::new(),
                    middleware: None,
                    env: BTreeMap::new(),
                    allowed_secrets: Vec::new(),
                    targets: Vec::new(),
                    resiliency: None,
                    min_instances: 0,
                    max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
                    volumes: Vec::new(),
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
            batch_targets: Vec::new(),
            batch_target_envs: Vec::new(),
            batch_target_volumes: Vec::new(),
            route_targets: Vec::new(),
            route_names: vec!["/api/guest-example=faas-a".to_owned()],
            route_versions: vec!["/api/guest-example=2.0.0".to_owned()],
            route_dependencies: vec!["/api/guest-example=faas-b@^3.1.0".to_owned()],
            route_credentials: Vec::new(),
            route_middlewares: Vec::new(),
            route_envs: vec!["/metrics=QUEUE_URL=https://queue.example/mock".to_owned()],
            route_scales: vec!["/api/guest-example=2:16".to_owned()],
            tcp_ports: vec!["2222=faas-a".to_owned()],
            udp_ports: vec!["5353=faas-a".to_owned()],
            volumes: vec!["/api/guest-example=/tmp/tachyon_data:/app/data:ro".to_owned()],
            advertise_ip: Some("203.0.113.50".to_owned()),
            telemetry_sample_rate: 0.25,
            memory_mib: 64,
        })
        .expect("request should produce a sealed config");

        assert_eq!(config.guest_fuel_budget, DEFAULT_GUEST_FUEL_BUDGET);
        assert_eq!(config.guest_memory_limit_bytes, 64 * 1024 * 1024);
        assert_eq!(config.advertise_ip.as_deref(), Some("203.0.113.50"));
        assert_eq!(
            config.layer4,
            SealedLayer4Config {
                tcp: vec![SealedTcpBinding {
                    port: 2222,
                    target: "faas-a".to_owned(),
                }],
                udp: vec![SealedUdpBinding {
                    port: 5353,
                    target: "faas-a".to_owned(),
                }],
            }
        );
        assert_eq!(config.telemetry_sample_rate, 0.25);
        assert_eq!(
            config.routes,
            vec![
                SealedRoute {
                    path: "/api/guest-example".to_owned(),
                    role: RouteRole::User,
                    name: "faas-a".to_owned(),
                    version: "2.0.0".to_owned(),
                    dependencies: BTreeMap::from([("faas-b".to_owned(), "^3.1.0".to_owned(),)]),
                    requires_credentials: Vec::new(),
                    middleware: None,
                    env: BTreeMap::new(),
                    allowed_secrets: vec!["DB_PASS".to_owned()],
                    targets: Vec::new(),
                    resiliency: None,
                    min_instances: 2,
                    max_concurrency: 16,
                    volumes: vec![SealedVolume {
                        volume_type: VolumeType::Host,
                        host_path: "/tmp/tachyon_data".to_owned(),
                        guest_path: "/app/data".to_owned(),
                        readonly: true,
                        ttl_seconds: None,
                        idle_timeout: None,
                        eviction_policy: None,
                    }],
                },
                SealedRoute {
                    path: "/metrics".to_owned(),
                    role: RouteRole::System,
                    name: "metrics".to_owned(),
                    version: default_route_version(),
                    dependencies: BTreeMap::new(),
                    requires_credentials: Vec::new(),
                    middleware: None,
                    env: BTreeMap::from([(
                        "QUEUE_URL".to_owned(),
                        "https://queue.example/mock".to_owned(),
                    )]),
                    allowed_secrets: Vec::new(),
                    targets: Vec::new(),
                    resiliency: None,
                    min_instances: 0,
                    max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
                    volumes: Vec::new(),
                }
            ]
        );

        let payload = config
            .canonical_payload()
            .expect("payload should serialize deterministically");
        assert!(payload.contains("\"path\":\"/api/guest-example\""));
        assert!(payload.contains("\"name\":\"faas-a\""));
        assert!(payload.contains("\"version\":\"2.0.0\""));
        assert!(payload.contains("\"dependencies\":{\"faas-b\":\"^3.1.0\"}"));
        assert!(payload.contains("\"role\":\"system\""));
        assert!(payload.contains("\"allowed_secrets\":[\"DB_PASS\"]"));
        assert!(payload.contains("\"min_instances\":2"));
        assert!(payload.contains("\"max_concurrency\":16"));
        assert!(payload.contains("\"layer4\":{\"tcp\":[{\"port\":2222,\"target\":\"faas-a\"}],\"udp\":[{\"port\":5353,\"target\":\"faas-a\"}]}"));
        assert!(payload.contains("\"env\":{\"QUEUE_URL\":\"https://queue.example/mock\"}"));
        assert!(payload.contains("\"guest_path\":\"/app/data\""));
        assert!(payload.contains("\"readonly\":true"));
        assert!(payload.contains("\"telemetry_sample_rate\":0.25"));
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
                "--route-target",
                "/api/guest-example=guest-example,weight=70",
                "--route-name",
                "/api/guest-example=faas-a",
                "--route-version",
                "/api/guest-example=2.0.0",
                "--route-dependency",
                "/api/guest-example=faas-b@^3.1.0",
                "--route-env",
                "/metrics=QUEUE_URL=https://queue.example/mock",
                "--route-scale",
                "/api/guest-example=1:8",
                "--tcp-port",
                "2222=faas-a",
                "--udp-port",
                "5353=faas-a",
                "--advertise-ip",
                "203.0.113.50",
                "--telemetry-sample-rate",
                "0.5",
                "--volume",
                "/api/guest-example=C:\\\\tachyon_data:/app/data:rw",
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
                batch_targets: Vec::new(),
                batch_target_envs: Vec::new(),
                batch_target_volumes: Vec::new(),
                route_targets: vec!["/api/guest-example=guest-example,weight=70".to_owned()],
                route_names: vec!["/api/guest-example=faas-a".to_owned()],
                route_versions: vec!["/api/guest-example=2.0.0".to_owned()],
                route_dependencies: vec!["/api/guest-example=faas-b@^3.1.0".to_owned()],
                route_credentials: Vec::new(),
                route_middlewares: Vec::new(),
                route_envs: vec!["/metrics=QUEUE_URL=https://queue.example/mock".to_owned()],
                route_scales: vec!["/api/guest-example=1:8".to_owned()],
                tcp_ports: vec!["2222=faas-a".to_owned()],
                udp_ports: vec!["5353=faas-a".to_owned()],
                volumes: vec!["/api/guest-example=C:\\\\tachyon_data:/app/data:rw".to_owned()],
                advertise_ip: Some("203.0.113.50".to_owned()),
                telemetry_sample_rate: 0.5,
                memory_mib: 64,
            }
        );
    }

    #[test]
    fn parse_generate_request_supports_route_credentials_and_middleware() {
        let request = parse_generate_request_from_args(
            [
                "generate",
                "--route",
                "/api/faas-a",
                "--system-route",
                "/system/auth",
                "--route-name",
                "/system/auth=system-faas-auth",
                "--route-credential",
                "/api/faas-a=cred-b,cred-a",
                "--route-middleware",
                "/api/faas-a=system-faas-auth",
                "--memory",
                "64",
            ]
            .into_iter()
            .map(str::to_owned),
        )
        .expect("arguments should parse")
        .expect("subcommand should be detected");

        assert_eq!(
            request.route_credentials,
            vec!["/api/faas-a=cred-b,cred-a".to_owned()]
        );
        assert_eq!(
            request.route_middlewares,
            vec!["/api/faas-a=system-faas-auth".to_owned()]
        );
    }

    #[test]
    fn normalize_routes_applies_scaling_overrides() {
        let routes = normalize_routes(
            vec!["/api/guest-example".to_owned()],
            Vec::new(),
            Vec::new(),
            false,
            vec!["/api/guest-example=3:7".to_owned()],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("route scaling should normalize");

        assert_eq!(
            routes,
            vec![SealedRoute {
                path: "/api/guest-example".to_owned(),
                role: RouteRole::User,
                name: "guest-example".to_owned(),
                version: default_route_version(),
                dependencies: BTreeMap::new(),
                requires_credentials: Vec::new(),
                middleware: None,
                env: BTreeMap::new(),
                allowed_secrets: Vec::new(),
                targets: Vec::new(),
                resiliency: None,
                min_instances: 3,
                max_concurrency: 7,
                volumes: Vec::new(),
            }]
        );
    }

    #[test]
    fn normalize_routes_applies_route_env_overrides() {
        let routes = normalize_routes_with_env(
            vec!["/api/guest-example".to_owned()],
            vec!["/system/sqs".to_owned()],
            Vec::new(),
            false,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![
                "/system/sqs=QUEUE_URL=https://queue.example/mock".to_owned(),
                "/system/sqs=TARGET_ROUTE=/api/guest-example".to_owned(),
            ],
        )
        .expect("route envs should normalize");

        let connector = routes
            .iter()
            .find(|route| route.path == "/system/sqs")
            .expect("system connector route should exist");

        assert_eq!(
            connector.env,
            BTreeMap::from([
                (
                    "QUEUE_URL".to_owned(),
                    "https://queue.example/mock".to_owned(),
                ),
                ("TARGET_ROUTE".to_owned(), "/api/guest-example".to_owned()),
            ])
        );
    }

    #[test]
    fn normalize_routes_applies_credentials_and_middleware() {
        let routes = normalize_routes(
            vec!["/api/faas-a".to_owned()],
            vec!["/system/auth".to_owned()],
            Vec::new(),
            false,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec!["/system/auth=system-faas-auth".to_owned()],
            Vec::new(),
            Vec::new(),
            vec!["/api/faas-a=cred-b,cred-a".to_owned()],
            vec!["/api/faas-a=system-faas-auth".to_owned()],
        )
        .expect("security overrides should normalize");

        assert_eq!(
            routes[0].requires_credentials,
            vec!["cred-a".to_owned(), "cred-b".to_owned()]
        );
        assert_eq!(routes[0].middleware.as_deref(), Some("system-faas-auth"));
    }

    #[test]
    fn normalize_routes_rejects_conflicting_roles_for_same_path() {
        let error = normalize_routes(
            vec!["/metrics".to_owned()],
            vec!["/metrics/".to_owned()],
            Vec::new(),
            false,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
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
            false,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
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
            false,
            vec!["/api/missing=1:8".to_owned()],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
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
            false,
            vec!["/api/guest-example=1:0".to_owned()],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
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

    #[test]
    fn normalize_routes_assigns_implicit_volume_to_the_only_route() {
        let routes = normalize_routes(
            vec!["/api/guest-volume".to_owned()],
            Vec::new(),
            Vec::new(),
            false,
            Vec::new(),
            vec!["C:\\\\tachyon_data:/app/data:ro".to_owned()],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("single route volume should normalize");

        assert_eq!(
            routes[0].volumes,
            vec![SealedVolume {
                volume_type: VolumeType::Host,
                host_path: "C:\\\\tachyon_data".to_owned(),
                guest_path: "/app/data".to_owned(),
                readonly: true,
                ttl_seconds: None,
                idle_timeout: None,
                eviction_policy: None,
            }]
        );
    }

    #[test]
    fn normalize_routes_rejects_implicit_volume_when_multiple_routes_exist() {
        let error = normalize_routes(
            vec![
                "/api/guest-example".to_owned(),
                "/api/guest-volume".to_owned(),
            ],
            Vec::new(),
            Vec::new(),
            false,
            Vec::new(),
            vec!["/tmp/tachyon_data:/app/data:rw".to_owned()],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect_err("implicit volume should be rejected when more than one route exists");

        assert!(error
            .to_string()
            .contains("must target a declared sealed route"));
    }

    #[test]
    fn normalize_routes_rejects_writable_user_volume_mounts() {
        let error = normalize_routes(
            vec!["/api/guest-volume".to_owned()],
            Vec::new(),
            Vec::new(),
            false,
            Vec::new(),
            vec!["/tmp/tachyon_data:/app/data:rw".to_owned()],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect_err("user volume mounts should stay read-only");

        assert!(error
            .to_string()
            .contains("cannot request writable host mounts"));
    }

    #[test]
    fn normalize_routes_allows_writable_system_volume_mounts() {
        let routes = normalize_routes(
            vec!["/api/guest-volume".to_owned()],
            vec!["/system/storage-broker".to_owned()],
            Vec::new(),
            false,
            Vec::new(),
            vec!["/system/storage-broker=/tmp/tachyon_data:/app/data:rw".to_owned()],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("system volume mounts should allow rw access");

        let broker = routes
            .iter()
            .find(|route| route.path == "/system/storage-broker")
            .expect("system route should be present");
        assert_eq!(
            broker.volumes,
            vec![SealedVolume {
                volume_type: VolumeType::Host,
                host_path: "/tmp/tachyon_data".to_owned(),
                guest_path: "/app/data".to_owned(),
                readonly: false,
                ttl_seconds: None,
                idle_timeout: None,
                eviction_policy: None,
            }]
        );
    }

    #[test]
    fn normalize_routes_allows_writable_user_ram_volume_mounts() {
        let routes = normalize_routes(
            vec!["/api/guest-volume".to_owned()],
            Vec::new(),
            Vec::new(),
            false,
            Vec::new(),
            vec![
                "/tmp/tachyon_ram:/app/data:rw,type=ram,idle_timeout=50ms,eviction_policy=hibernate"
                    .to_owned(),
            ],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("user RAM volumes should allow writable access");

        assert_eq!(
            routes[0].volumes,
            vec![SealedVolume {
                volume_type: VolumeType::Ram,
                host_path: "/tmp/tachyon_ram".to_owned(),
                guest_path: "/app/data".to_owned(),
                readonly: false,
                ttl_seconds: None,
                idle_timeout: Some("50ms".to_owned()),
                eviction_policy: Some(VolumeEvictionPolicy::Hibernate),
            }]
        );
    }

    #[test]
    fn normalize_routes_accepts_volume_ttl_seconds() {
        let routes = normalize_routes(
            vec!["/api/guest-volume".to_owned()],
            Vec::new(),
            Vec::new(),
            false,
            Vec::new(),
            vec!["/tmp/tachyon_data:/app/data:ro,ttl_seconds=300".to_owned()],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("volume ttl should normalize");

        assert_eq!(
            routes[0].volumes,
            vec![SealedVolume {
                volume_type: VolumeType::Host,
                host_path: "/tmp/tachyon_data".to_owned(),
                guest_path: "/app/data".to_owned(),
                readonly: true,
                ttl_seconds: Some(300),
                idle_timeout: None,
                eviction_policy: None,
            }]
        );
    }

    #[test]
    fn normalize_routes_applies_explicit_targets_in_order() {
        let routes = normalize_routes(
            vec!["/api/checkout".to_owned()],
            Vec::new(),
            Vec::new(),
            false,
            Vec::new(),
            Vec::new(),
            vec![
                "/api/checkout=checkout-v1,weight=90".to_owned(),
                "/api/checkout=checkout-v2,weight=10".to_owned(),
                "/api/checkout=checkout-beta,header=X-Cohort=beta".to_owned(),
            ],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("route targets should normalize");

        assert_eq!(
            routes,
            vec![SealedRoute {
                path: "/api/checkout".to_owned(),
                role: RouteRole::User,
                name: "checkout".to_owned(),
                version: default_route_version(),
                dependencies: BTreeMap::new(),
                requires_credentials: Vec::new(),
                middleware: None,
                env: BTreeMap::new(),
                allowed_secrets: Vec::new(),
                targets: vec![
                    RouteTarget {
                        module: "checkout-v1".to_owned(),
                        weight: 90,
                        websocket: false,
                        match_header: None,
                        requires: vec!["core:wasi".to_owned()],
                    },
                    RouteTarget {
                        module: "checkout-v2".to_owned(),
                        weight: 10,
                        websocket: false,
                        match_header: None,
                        requires: vec!["core:wasi".to_owned()],
                    },
                    RouteTarget {
                        module: "checkout-beta".to_owned(),
                        weight: 0,
                        websocket: false,
                        match_header: Some(HeaderMatch {
                            name: "x-cohort".to_owned(),
                            value: "beta".to_owned(),
                        }),
                        requires: vec!["core:wasi".to_owned()],
                    },
                ],
                resiliency: None,
                min_instances: 0,
                max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
                volumes: Vec::new(),
            }]
        );
    }

    #[test]
    fn normalize_routes_accepts_websocket_targets() {
        let routes = normalize_routes(
            vec!["/ws/echo".to_owned()],
            Vec::new(),
            Vec::new(),
            false,
            Vec::new(),
            Vec::new(),
            vec!["/ws/echo=guest-websocket-echo,websocket=true".to_owned()],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("WebSocket route targets should normalize");

        assert_eq!(
            routes[0].targets,
            vec![RouteTarget {
                module: "guest-websocket-echo".to_owned(),
                weight: 100,
                websocket: true,
                match_header: None,
                requires: vec!["core:wasi".to_owned()],
            }]
        );
    }

    #[test]
    fn normalize_routes_accepts_capability_requirements_on_targets() {
        let routes = normalize_routes(
            vec!["/api/guest-ai".to_owned()],
            Vec::new(),
            Vec::new(),
            false,
            Vec::new(),
            Vec::new(),
            vec!["/api/guest-ai=guest-ai,requires=core:wasi+accel:cuda".to_owned()],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("capability constrained targets should normalize");

        assert_eq!(
            routes[0].targets[0].requires,
            vec!["accel:cuda".to_owned(), "core:wasi".to_owned()]
        );
    }

    #[test]
    fn normalize_routes_rejects_targets_for_unknown_routes() {
        let error = normalize_routes(
            vec!["/api/guest-example".to_owned()],
            Vec::new(),
            Vec::new(),
            false,
            Vec::new(),
            Vec::new(),
            vec!["/api/unknown=guest-loop,weight=100".to_owned()],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect_err("route target should require a declared route");

        assert!(error
            .to_string()
            .contains("must target a declared sealed route"));
    }
}
