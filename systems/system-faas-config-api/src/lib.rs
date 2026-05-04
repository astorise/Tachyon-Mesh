mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

#[allow(dead_code)]
mod routing_contract {
    wit_bindgen::generate!({
        path: "../../wit/config-routing.wit",
        world: "traffic-management-config",
    });
}

#[allow(dead_code)]
mod ai_contract {
    wit_bindgen::generate!({
        path: "../../wit/config-ai.wit",
        world: "ai-orchestration-config",
    });
}

#[allow(dead_code)]
mod assets_contract {
    wit_bindgen::generate!({
        path: "../../wit/config-assets.wit",
        world: "air-gapped-asset-config",
    });
}

#[allow(dead_code)]
mod cache_contract {
    wit_bindgen::generate!({
        path: "../../wit/config-cache.wit",
        world: "distributed-cache-config",
    });
}

#[allow(dead_code)]
mod fleet_contract {
    wit_bindgen::generate!({
        path: "../../wit/config-fleet.wit",
        world: "fleet-profile-config",
    });
}

#[allow(dead_code)]
mod hardware_contract {
    wit_bindgen::generate!({
        path: "../../wit/config-hardware.wit",
        world: "hardware-acceleration-config",
    });
}

#[allow(dead_code)]
mod topology_contract {
    wit_bindgen::generate!({
        path: "../../wit/config-topology.wit",
        world: "mesh-topology-config",
    });
}

#[allow(dead_code)]
mod observability_contract {
    wit_bindgen::generate!({
        path: "../../wit/config-observability.wit",
        world: "observability-compute-config",
    });
}

#[allow(dead_code)]
mod resilience_contract {
    wit_bindgen::generate!({
        path: "../../wit/config-resilience.wit",
        world: "resilience-chaos-config",
    });
}

#[allow(dead_code)]
mod security_contract {
    wit_bindgen::generate!({
        path: "../../wit/config-security.wit",
        world: "security-identity-config",
    });
}

use serde_json::Value;

const BROKER_ROUTE_ENV: &str = "GITOPS_BROKER_ROUTE";
const DEFAULT_BROKER_ROUTE: &str = "/system/gitops-broker";
const ENVIRONMENT_HEADER: &str = "x-tachyon-environment";

struct Component;

pub fn validate_traffic_config<T>(_config: T) -> Result<(), String> {
    Ok(())
}

pub fn validate_ai_config<T>(_config: T) -> Result<(), String> {
    Ok(())
}

pub fn apply_model_deployment<T>(_deployment: T) -> Result<(), String> {
    Ok(())
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
    fn validate_traffic_config_scaffold_accepts_typed_payloads() {
        validate_traffic_config(()).expect("scaffold validator accepts payloads");
    }

    #[test]
    fn ai_config_scaffold_accepts_model_deployments() {
        validate_ai_config(()).expect("AI config scaffold accepts payloads");
        apply_model_deployment(()).expect("AI deployment scaffold accepts payloads");
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
}
