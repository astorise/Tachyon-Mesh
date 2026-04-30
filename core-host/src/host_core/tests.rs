#[cfg(test)]
mod tests {
    include!("tests/support_and_cache.rs");
    include!("tests/integrity_admin.rs");
    include!("tests/reload_and_guest.rs");
    include!("tests/http_router.rs");
    include!("tests/background_connectors.rs");
    include!("tests/telemetry_and_l4.rs");
    include!("tests/l4_tls_quic.rs");
    include!("tests/routing_aliases.rs");
    include!("tests/uds_loop_resilience.rs");
    include!("tests/config_validation.rs");
    include!("tests/rate_limit_models.rs");
    include!("tests/mesh_control_plane.rs");
}
