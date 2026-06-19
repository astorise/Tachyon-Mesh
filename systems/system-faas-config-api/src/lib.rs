mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

// The `*_contract` modules below pull in WIT-generated *types* for config
// domains that this component does not (yet) export directly. Each
// `generate!` invocation embeds a component-type requirement for its own
// world in the compiled object, independent of whether `export!` is called
// for it — so without `stubs`, `wasm-component-ld` fails to encode the
// final component because those exports are declared but never implemented.
// `stubs` auto-generates an `unreachable!()`-bodied `Stub` and calls
// `export!(Stub)` for us, satisfying the linker while keeping these domains
// inert (consistent with their current scaffold-only status). `ai_contract`
// is the exception: its `apply-model-deployment` export is implemented for
// real below, so it is not stubbed.
#[allow(dead_code)]
mod routing_contract {
    wit_bindgen::generate!({
        path: "../../wit/config-routing.wit",
        world: "traffic-management-config",
        stubs,
    });
}

mod ai_contract {
    wit_bindgen::generate!({
        path: "../../wit/config-ai.wit",
        world: "ai-orchestration-config",
    });

    use super::AiConfigComponent;
    export!(AiConfigComponent);
}

#[allow(dead_code)]
mod assets_contract {
    wit_bindgen::generate!({
        path: "../../wit/config-assets.wit",
        world: "air-gapped-asset-config",
        stubs,
    });
}

#[allow(dead_code)]
mod cache_contract {
    wit_bindgen::generate!({
        path: "../../wit/config-cache.wit",
        world: "distributed-cache-config",
        stubs,
    });
}

#[allow(dead_code)]
mod fleet_contract {
    wit_bindgen::generate!({
        path: "../../wit/config-fleet.wit",
        world: "fleet-profile-config",
        stubs,
    });
}

#[allow(dead_code)]
mod hardware_contract {
    wit_bindgen::generate!({
        path: "../../wit/config-hardware.wit",
        world: "hardware-acceleration-config",
        stubs,
    });
}

#[allow(dead_code)]
mod topology_contract {
    wit_bindgen::generate!({
        path: "../../wit/config-topology.wit",
        world: "mesh-topology-config",
        stubs,
    });
}

#[allow(dead_code)]
mod observability_contract {
    wit_bindgen::generate!({
        path: "../../wit/config-observability.wit",
        world: "observability-compute-config",
        stubs,
    });
}

#[allow(dead_code)]
mod resilience_contract {
    wit_bindgen::generate!({
        path: "../../wit/config-resilience.wit",
        world: "resilience-chaos-config",
        stubs,
    });
}

#[allow(dead_code)]
mod security_contract {
    wit_bindgen::generate!({
        path: "../../wit/config-security.wit",
        world: "security-identity-config",
        stubs,
    });
}

#[allow(dead_code)]
mod storage_contract {
    wit_bindgen::generate!({
        path: "../../wit/config-storage.wit",
        world: "storage-state-config",
        stubs,
    });
}

#[allow(dead_code)]
mod workloads_contract {
    wit_bindgen::generate!({
        path: "../../wit/config-workloads.wit",
        world: "workload-orchestration-config",
        stubs,
    });
}

#[allow(dead_code)]
mod rbac_contract {
    wit_bindgen::generate!({
        path: "../../wit/config-rbac.wit",
        world: "control-plane-rbac-config",
        stubs,
    });
}

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Structured validation types ───────────────────────────────────────────────

/// A single validation error produced during a manifest dry-run.
/// `path` follows JSONPointer notation (e.g. `routes/0/maxConcurrency`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    /// JSONPointer-style path to the offending field (e.g. `routes/0/minRamMb`).
    pub path: String,
    /// Human-readable description of the problem.
    pub message: String,
    /// Machine-readable error code (e.g. `INVALID_TYPE`, `MISSING_FIELD`,
    /// `CONSTRAINT_VIOLATION`).
    pub error_code: String,
}

/// Result returned by the manifest dry-run validation pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DryRunResult {
    /// `true` when the payload passes all validation checks.
    pub valid: bool,
    /// Ordered list of validation errors; empty when `valid` is `true`.
    pub errors: Vec<ValidationError>,
    /// Optional diff snapshot showing what would change if the manifest
    /// were applied. `None` when validation fails or no diff is available.
    pub diff: Option<Value>,
}

impl DryRunResult {
    /// Convenience constructor for a successful validation with no errors.
    pub fn ok(diff: Option<Value>) -> Self {
        Self {
            valid: true,
            errors: Vec::new(),
            diff,
        }
    }

    /// Convenience constructor for a failed validation.
    pub fn fail(errors: Vec<ValidationError>) -> Self {
        Self {
            valid: false,
            errors,
            diff: None,
        }
    }
}

/// Validate a raw manifest JSON value against the known `IntegrityConfig`
/// structural constraints. Returns a [`DryRunResult`] describing any errors.
///
/// This performs structural, not semantic, validation: required fields must
/// be present and have the correct JSON type. Full semantic validation
/// (e.g. signature verification, version ordering) is performed by the
/// `core-host` when the manifest is submitted.
pub fn validate_manifest_payload(payload: &Value) -> DryRunResult {
    let mut errors: Vec<ValidationError> = Vec::new();

    let obj = match payload.as_object() {
        Some(o) => o,
        None => {
            return DryRunResult::fail(vec![ValidationError {
                path: "/".to_owned(),
                message: "manifest must be a JSON object".to_owned(),
                error_code: "INVALID_TYPE".to_owned(),
            }]);
        }
    };

    // Required top-level string fields.
    for field in &["hostAddress", "resourceLimitResponse"] {
        match obj.get(*field) {
            None => errors.push(ValidationError {
                path: format!("/{field}"),
                message: format!("`{field}` is required"),
                error_code: "MISSING_FIELD".to_owned(),
            }),
            Some(v) if !v.is_string() => errors.push(ValidationError {
                path: format!("/{field}"),
                message: format!("`{field}` must be a string"),
                error_code: "INVALID_TYPE".to_owned(),
            }),
            _ => {}
        }
    }

    // Required top-level integer fields.
    for field in &["maxStdoutBytes", "guestFuelBudget", "guestMemoryLimitBytes"] {
        match obj.get(*field) {
            None => errors.push(ValidationError {
                path: format!("/{field}"),
                message: format!("`{field}` is required"),
                error_code: "MISSING_FIELD".to_owned(),
            }),
            Some(v) if !v.is_number() => errors.push(ValidationError {
                path: format!("/{field}"),
                message: format!("`{field}` must be an integer"),
                error_code: "INVALID_TYPE".to_owned(),
            }),
            _ => {}
        }
    }

    // `routes` must be an array.
    match obj.get("routes") {
        None => errors.push(ValidationError {
            path: "/routes".to_owned(),
            message: "`routes` array is required".to_owned(),
            error_code: "MISSING_FIELD".to_owned(),
        }),
        Some(v) if !v.is_array() => errors.push(ValidationError {
            path: "/routes".to_owned(),
            message: "`routes` must be an array".to_owned(),
            error_code: "INVALID_TYPE".to_owned(),
        }),
        Some(Value::Array(routes)) => {
            for (i, route) in routes.iter().enumerate() {
                if let Some(route_obj) = route.as_object() {
                    for field in &["path", "version"] {
                        if route_obj.get(*field).is_none_or(|v| !v.is_string()) {
                            errors.push(ValidationError {
                                path: format!("/routes/{i}/{field}"),
                                message: format!("route `{field}` must be a non-empty string"),
                                error_code: if route_obj.contains_key(*field) {
                                    "INVALID_TYPE"
                                } else {
                                    "MISSING_FIELD"
                                }
                                .to_owned(),
                            });
                        }
                    }
                    // maxConcurrency must be a positive integer if present.
                    if let Some(v) = route_obj.get("maxConcurrency") {
                        if v.as_u64().is_none_or(|n| n == 0) {
                            errors.push(ValidationError {
                                path: format!("/routes/{i}/maxConcurrency"),
                                message: "`maxConcurrency` must be a positive integer".to_owned(),
                                error_code: "CONSTRAINT_VIOLATION".to_owned(),
                            });
                        }
                    }
                    validate_canary_metrics(route_obj.get("canary"), i, &mut errors);
                }
            }
        }
        _ => {}
    }

    if errors.is_empty() {
        DryRunResult::ok(None)
    } else {
        DryRunResult::fail(errors)
    }
}

fn validate_canary_metrics(
    canary: Option<&Value>,
    route_index: usize,
    errors: &mut Vec<ValidationError>,
) {
    let Some(canary) = canary.and_then(Value::as_object) else {
        return;
    };
    let Some(metrics) = canary.get("metrics") else {
        return;
    };
    let Some(metrics) = metrics.as_array() else {
        errors.push(ValidationError {
            path: format!("/routes/{route_index}/canary/metrics"),
            message: "`canary.metrics` must be an array".to_owned(),
            error_code: "INVALID_TYPE".to_owned(),
        });
        return;
    };

    for (metric_index, metric) in metrics.iter().enumerate() {
        let Some(metric) = metric.as_object() else {
            errors.push(ValidationError {
                path: format!("/routes/{route_index}/canary/metrics/{metric_index}"),
                message: "canary metric threshold must be an object".to_owned(),
                error_code: "INVALID_TYPE".to_owned(),
            });
            continue;
        };
        if metric
            .get("name")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            errors.push(ValidationError {
                path: format!("/routes/{route_index}/canary/metrics/{metric_index}/name"),
                message: "canary metric `name` must be a non-empty string".to_owned(),
                error_code: "CONSTRAINT_VIOLATION".to_owned(),
            });
        }
        let threshold = metric.get("threshold").and_then(Value::as_str);
        if threshold.is_none_or(|value| parse_metric_threshold(value).is_none()) {
            errors.push(ValidationError {
                path: format!("/routes/{route_index}/canary/metrics/{metric_index}/threshold"),
                message:
                    "canary metric `threshold` must use '<', '<=', '>' or '>=' with a numeric value"
                        .to_owned(),
                error_code: "CONSTRAINT_VIOLATION".to_owned(),
            });
        }
    }
}

fn parse_metric_threshold(input: &str) -> Option<(&str, f64)> {
    let trimmed = input.trim();
    let operator = [">=", "<=", ">", "<"]
        .into_iter()
        .find(|operator| trimmed.starts_with(operator))?;
    let raw_value = trimmed[operator.len()..].trim();
    let numeric = raw_value.trim_end_matches('%').trim().parse::<f64>().ok()?;
    Some((operator, numeric))
}

const BROKER_ROUTE_ENV: &str = "GITOPS_BROKER_ROUTE";
const DEFAULT_BROKER_ROUTE: &str = "/system/gitops-broker";
const ENVIRONMENT_HEADER: &str = "x-tachyon-environment";

struct Component;

struct AiConfigComponent;

pub fn validate_traffic_config<T>(_config: T) -> Result<(), String> {
    Ok(())
}

pub fn validate_ai_config<T>(_config: T) -> Result<(), String> {
    Ok(())
}

pub fn apply_model_deployment<T>(_deployment: T) -> Result<(), String> {
    Ok(())
}

/// Maps a WIT `hardware-strategy` record into the shared, hardware-agnostic
/// `parallel_topology::ParallelExecutionPlan`, then runs `validate_plan_shape`
/// against it. This component has no access to live hardware topology (it is
/// a Wasm guest), so only structural shape is checked here; hardware-aware
/// checks (`validate_parallel_topology`) run in `core-host` once the plan is
/// actually dispatched against discovered devices.
fn validate_hardware_strategy(
    strategy: &ai_contract::exports::tachyon::ai_config::config_ai::HardwareStrategy,
) -> Result<(), String> {
    use ai_contract::exports::tachyon::ai_config::config_ai::GpuDistribution;
    use parallel_topology::{ParallelExecutionPlan, ParallelStrategy};

    let parallel_strategy = match strategy.distribution_mode {
        GpuDistribution::Single => ParallelStrategy::None,
        GpuDistribution::TensorParallelism => ParallelStrategy::TensorParallel,
        GpuDistribution::PipelineParallelism => ParallelStrategy::PipelineParallel,
        GpuDistribution::ExpertParallelism => ParallelStrategy::ExpertParallel,
    };

    let plan = ParallelExecutionPlan {
        strategy: parallel_strategy,
        device_ids: strategy.device_ids.clone(),
        stage_layer_ranges: strategy.stage_layer_ranges.clone(),
        expert_device_map: strategy.expert_device_map.clone(),
        required_vram_bytes_per_device: 0,
        pipeline_depth: strategy.pipeline_depth,
    };

    parallel_topology::validate_plan_shape(&plan)
}

impl ai_contract::exports::tachyon::ai_config::config_ai::Guest for AiConfigComponent {
    fn validate_ai_config(
        config: ai_contract::exports::tachyon::ai_config::config_ai::AiConfiguration,
    ) -> Result<(), String> {
        for deployment in &config.deployments {
            validate_hardware_strategy(&deployment.strategy)?;
        }
        Ok(())
    }

    fn get_ai_config(
    ) -> Result<ai_contract::exports::tachyon::ai_config::config_ai::AiConfiguration, String> {
        Err("AI configuration store is not wired in this component yet".to_owned())
    }

    fn apply_model_deployment(
        deployment: ai_contract::exports::tachyon::ai_config::config_ai::ModelDeployment,
    ) -> Result<(), String> {
        validate_hardware_strategy(&deployment.strategy)
    }
}

pub fn validate_asset_config<T>(_config: T) -> Result<(), String> {
    Ok(())
}

pub fn apply_asset_bundle<T>(_bundle: T) -> Result<(), String> {
    Ok(())
}

pub fn validate_cache_config<T>(_config: T) -> Result<(), String> {
    Ok(())
}

pub fn apply_cache_config<T>(_config: T) -> Result<(), String> {
    Ok(())
}

pub fn validate_fleet_config<T>(_config: T) -> Result<(), String> {
    Ok(())
}

pub fn apply_fleet_profile<T>(_profile: T) -> Result<(), String> {
    Ok(())
}

pub fn fleet_profile_matches_node<T, U>(_profile: T, _node: U) -> bool {
    true
}

pub fn validate_hardware_config<T>(_config: T) -> Result<(), String> {
    Ok(())
}

pub fn update_hardware<T>(_config: T) -> Result<(), String> {
    Ok(())
}

pub fn validate_topology_config<T>(_config: T) -> Result<(), String> {
    Ok(())
}

pub fn apply_topology_config<T>(_config: T) -> Result<(), String> {
    Ok(())
}

pub fn validate_ops_config<T>(_config: T) -> Result<(), String> {
    Ok(())
}

pub fn apply_compute_quota<T>(_quota: T) -> Result<(), String> {
    Ok(())
}

pub fn update_telemetry<T>(_config: T) -> Result<(), String> {
    Ok(())
}

pub fn validate_resilience_config<T>(_config: T) -> Result<(), String> {
    Ok(())
}

pub fn apply_resilience_policy<T>(_policy: T) -> Result<(), String> {
    Ok(())
}

pub fn delete_resilience_policy(_name: &str) -> Result<(), String> {
    Ok(())
}

pub fn validate_security_config<T>(_config: T) -> Result<(), String> {
    Ok(())
}

pub fn apply_rate_limit<T>(_limit: T) -> Result<(), String> {
    Ok(())
}

pub fn delete_rate_limit(_name: &str) -> Result<(), String> {
    Ok(())
}

pub fn validate_storage_config<T>(_config: T) -> Result<(), String> {
    Ok(())
}

pub fn apply_wasi_volume<T>(_volume: T) -> Result<(), String> {
    Ok(())
}

pub fn apply_s3_backend<T>(_backend: T) -> Result<(), String> {
    Ok(())
}

pub fn apply_kv_partition<T>(_partition: T) -> Result<(), String> {
    Ok(())
}

pub fn validate_workload_config<T>(_config: T) -> Result<(), String> {
    Ok(())
}

pub fn apply_workload<T>(_spec: T) -> Result<(), String> {
    Ok(())
}

pub fn validate_rbac_config<T>(_config: T) -> Result<(), String> {
    Ok(())
}

pub fn evaluate_access<T, U>(
    _subject_claims: T,
    _action: U,
    _domain: &str,
    _resource_labels: T,
) -> Result<bool, String> {
    Ok(true)
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        if !req.method.eq_ignore_ascii_case("POST") && !req.method.eq_ignore_ascii_case("PUT") {
            return response(405, "Method Not Allowed");
        }

        let environment = match validate_config_request(&req.body) {
            Ok(environment) => environment,
            Err(error) => return response(400, error),
        };

        match forward_to_broker(&environment, &req.body) {
            Ok(response) => response,
            Err(error) => response(502, error),
        }
    }
}

fn validate_config_request(body: &[u8]) -> Result<String, String> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|error| format!("request body must be JSON: {error}"))?;
    if !value.is_object() {
        return Err("configuration request must be a JSON object".to_owned());
    }

    let environment = value
        .get("environment")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("dev");
    validate_environment(environment)?;
    Ok(environment.to_owned())
}

fn validate_environment(environment: &str) -> Result<(), String> {
    if environment
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        Ok(())
    } else {
        Err(format!(
            "environment `{environment}` may only contain ASCII letters, digits, '-', '_' or '.'"
        ))
    }
}

fn forward_to_broker(
    environment: &str,
    body: &[u8],
) -> Result<bindings::exports::tachyon::mesh::handler::Response, String> {
    let broker_route = normalize_route(&env_or_default(BROKER_ROUTE_ENV, DEFAULT_BROKER_ROUTE))?;
    let broker_url = format!("http://mesh{broker_route}");
    let broker_response = bindings::tachyon::mesh::outbound_http::send_request(
        "POST",
        &broker_url,
        &[
            ("content-type".to_owned(), "application/json".to_owned()),
            (ENVIRONMENT_HEADER.to_owned(), environment.to_owned()),
        ],
        body,
    )?;

    Ok(bindings::exports::tachyon::mesh::handler::Response {
        status: broker_response.status,
        headers: broker_response.headers,
        body: broker_response.body,
        trailers: vec![],
    })
}

fn env_or_default(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_owned())
}

fn normalize_route(route: &str) -> Result<String, String> {
    let trimmed = route.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("broker route must not be empty".to_owned());
    }
    Ok(if trimmed.starts_with('/') {
        trimmed.to_owned()
    } else {
        format!("/{trimmed}")
    })
}

fn response(
    status: u16,
    body: impl Into<Vec<u8>>,
) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status,
        headers: vec![],
        body: body.into(),
        trailers: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_config_request_defaults_environment() {
        assert_eq!(
            validate_config_request(br#"{"config":{"routes":[]}}"#).expect("valid request"),
            "dev"
        );
    }

    #[test]
    fn validate_config_request_rejects_invalid_environment() {
        let error = validate_config_request(br#"{"environment":"prod/main"}"#)
            .expect_err("slash should be rejected");

        assert!(error.contains("environment"));
    }

    #[test]
    fn manifest_validation_accepts_canary_custom_metric_thresholds() {
        let result = validate_manifest_payload(&serde_json::json!({
            "hostAddress": "0.0.0.0:8080",
            "maxStdoutBytes": 65536,
            "guestFuelBudget": 500000,
            "guestMemoryLimitBytes": 67108864,
            "resourceLimitResponse": "limited",
            "routes": [{
                "path": "/checkout",
                "version": "1.1.0",
                "maxConcurrency": 10,
                "canary": {
                    "nextVersion": "checkout-v2",
                    "maxErrorRate": 0.01,
                    "metrics": [{
                        "name": "piano.checkout_conversion",
                        "threshold": "> 0.12"
                    }]
                }
            }]
        }));

        assert!(result.valid, "{:?}", result.errors);
    }

    #[test]
    fn manifest_validation_rejects_invalid_canary_metric_thresholds() {
        let result = validate_manifest_payload(&serde_json::json!({
            "hostAddress": "0.0.0.0:8080",
            "maxStdoutBytes": 65536,
            "guestFuelBudget": 500000,
            "guestMemoryLimitBytes": 67108864,
            "resourceLimitResponse": "limited",
            "routes": [{
                "path": "/checkout",
                "version": "1.1.0",
                "canary": {
                    "metrics": [{
                        "name": "",
                        "threshold": "around 12"
                    }]
                }
            }]
        }));

        assert!(!result.valid);
        assert_eq!(result.errors.len(), 2);
    }

    #[test]
    fn validate_traffic_config_scaffold_accepts_typed_payloads() {
        validate_traffic_config(()).expect("scaffold validator accepts payloads");
    }

    #[test]
    fn ai_config_scaffold_accepts_model_deployments() {
        validate_ai_config(()).expect("AI config scaffold accepts payloads");
        apply_model_deployment(()).expect("AI deployment scaffold accepts payloads");
    }

    fn hardware_strategy(
        distribution_mode: ai_contract::exports::tachyon::ai_config::config_ai::GpuDistribution,
        device_ids: Vec<u32>,
        stage_layer_ranges: Vec<(u32, u32)>,
        expert_device_map: Vec<(u32, u32)>,
        pipeline_depth: u32,
    ) -> ai_contract::exports::tachyon::ai_config::config_ai::HardwareStrategy {
        ai_contract::exports::tachyon::ai_config::config_ai::HardwareStrategy {
            multi_gpu: !device_ids.is_empty(),
            distribution_mode,
            device_ids,
            stage_layer_ranges,
            expert_device_map,
            pipeline_depth,
        }
    }

    #[test]
    fn single_gpu_strategy_always_validates() {
        use ai_contract::exports::tachyon::ai_config::config_ai::GpuDistribution;

        let strategy = hardware_strategy(GpuDistribution::Single, vec![], vec![], vec![], 0);
        validate_hardware_strategy(&strategy).expect("single-device plan is always valid");
    }

    #[test]
    fn tensor_parallel_strategy_rejects_a_single_device() {
        use ai_contract::exports::tachyon::ai_config::config_ai::GpuDistribution;

        let strategy = hardware_strategy(
            GpuDistribution::TensorParallelism,
            vec![0],
            vec![],
            vec![],
            0,
        );
        validate_hardware_strategy(&strategy).expect_err("tensor-parallel requires >= 2 devices");
    }

    #[test]
    fn pipeline_parallel_strategy_accepts_contiguous_stage_ranges() {
        use ai_contract::exports::tachyon::ai_config::config_ai::GpuDistribution;

        let strategy = hardware_strategy(
            GpuDistribution::PipelineParallelism,
            vec![0, 1],
            vec![(0, 5), (6, 10)],
            vec![],
            4,
        );
        validate_hardware_strategy(&strategy).expect("contiguous pipeline stages are valid");
    }

    #[test]
    fn pipeline_parallel_strategy_rejects_non_contiguous_stage_ranges() {
        use ai_contract::exports::tachyon::ai_config::config_ai::GpuDistribution;

        let strategy = hardware_strategy(
            GpuDistribution::PipelineParallelism,
            vec![0, 1],
            vec![(0, 5), (7, 10)],
            vec![],
            4,
        );
        validate_hardware_strategy(&strategy)
            .expect_err("non-contiguous pipeline stages must be rejected");
    }

    #[test]
    fn expert_parallel_strategy_rejects_out_of_bounds_device_index() {
        use ai_contract::exports::tachyon::ai_config::config_ai::GpuDistribution;

        let strategy = hardware_strategy(
            GpuDistribution::ExpertParallelism,
            vec![0, 1],
            vec![],
            vec![(0, 0), (1, 5)],
            0,
        );
        validate_hardware_strategy(&strategy)
            .expect_err("expert-device-map index must reference a declared device-id");
    }

    #[test]
    fn asset_config_scaffold_accepts_bundles() {
        validate_asset_config(()).expect("asset config scaffold accepts payloads");
        apply_asset_bundle(()).expect("asset bundle scaffold accepts payloads");
    }

    #[test]
    fn cache_config_scaffold_accepts_cache_configs() {
        validate_cache_config(()).expect("cache config scaffold accepts payloads");
        apply_cache_config(()).expect("cache apply scaffold accepts payloads");
    }

    #[test]
    fn fleet_config_scaffold_accepts_profiles() {
        validate_fleet_config(()).expect("fleet config scaffold accepts payloads");
        apply_fleet_profile(()).expect("fleet profile scaffold accepts payloads");
        assert!(fleet_profile_matches_node((), ()));
    }

    #[test]
    fn hardware_config_scaffold_accepts_updates() {
        validate_hardware_config(()).expect("hardware config scaffold accepts payloads");
        update_hardware(()).expect("hardware update scaffold accepts payloads");
    }

    #[test]
    fn topology_config_scaffold_accepts_updates() {
        validate_topology_config(()).expect("topology config scaffold accepts payloads");
        apply_topology_config(()).expect("topology update scaffold accepts payloads");
    }

    #[test]
    fn observability_config_scaffold_accepts_updates() {
        validate_ops_config(()).expect("ops config scaffold accepts payloads");
        apply_compute_quota(()).expect("quota scaffold accepts payloads");
        update_telemetry(()).expect("telemetry scaffold accepts payloads");
    }

    #[test]
    fn resilience_config_scaffold_accepts_policies() {
        validate_resilience_config(()).expect("resilience config scaffold accepts payloads");
        apply_resilience_policy(()).expect("resilience policy scaffold accepts payloads");
        delete_resilience_policy("default").expect("resilience delete scaffold accepts names");
    }

    #[test]
    fn security_config_scaffold_accepts_rate_limits() {
        validate_security_config(()).expect("security config scaffold accepts payloads");
        apply_rate_limit(()).expect("rate-limit scaffold accepts payloads");
        delete_rate_limit("default").expect("rate-limit delete scaffold accepts names");
    }

    #[test]
    fn storage_config_scaffold_accepts_resources() {
        validate_storage_config(()).expect("storage config scaffold accepts payloads");
        apply_wasi_volume(()).expect("volume scaffold accepts payloads");
        apply_s3_backend(()).expect("s3 scaffold accepts payloads");
        apply_kv_partition(()).expect("kv scaffold accepts payloads");
    }

    #[test]
    fn workload_config_scaffold_accepts_specs() {
        validate_workload_config(()).expect("workload config scaffold accepts payloads");
        apply_workload(()).expect("workload scaffold accepts payloads");
    }

    #[test]
    fn rbac_config_scaffold_allows_access_by_default() {
        validate_rbac_config(()).expect("rbac config scaffold accepts payloads");
        assert!(evaluate_access((), (), "config-routing", ())
            .expect("rbac evaluation scaffold accepts payloads"));
    }
}
