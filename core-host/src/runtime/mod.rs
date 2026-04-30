#![allow(dead_code)]

pub(crate) mod instance_pool {
    pub(crate) const MODULE: &str = "runtime::instance_pool";
}

pub(crate) mod wasmtime_engine {
    pub(crate) const MODULE: &str = "runtime::wasmtime_engine";
}

use super::*;

// Extracted runtime bootstrap and in-memory module pool setup.
/// Tachyon deployment's route count so the cache is effectively unbounded for the
/// happy path; the explicit cap is a defense-in-depth ceiling that prevents a
/// runaway manifest from blowing up host RSS.
pub(crate) const INSTANCE_POOL_DEFAULT_CAPACITY: u64 = 256;

/// Idle threshold after which a warm `Arc<Module>` entry is evicted from the
/// in-memory pool. The next request for the module pays a cwasm-cache thaw (read
/// the precompiled bytes from redb + `Module::deserialize`) — significantly
/// faster than a fresh JIT compile, so this approximates the hibernation /
/// scale-to-zero pattern called out by `wasm-ram-hibernation` without giving up
/// the warm-start latency for actively-used modules.
pub(crate) const INSTANCE_POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

pub(crate) fn build_runtime_state(config: IntegrityConfig) -> Result<RuntimeState> {
    let instance_pool = Arc::new(
        moka::sync::Cache::builder()
            .max_capacity(INSTANCE_POOL_DEFAULT_CAPACITY)
            .time_to_idle(INSTANCE_POOL_IDLE_TIMEOUT)
            .build(),
    );
    Ok(RuntimeState {
        engine: build_engine(&config, false)?,
        metered_engine: build_engine(&config, true)?,
        route_registry: Arc::new(RouteRegistry::build(&config)?),
        batch_target_registry: Arc::new(BatchTargetRegistry::build(&config)?),
        concurrency_limits: build_concurrency_limits(&config),
        instance_pool,
        #[cfg(feature = "ai-inference")]
        ai_runtime: Arc::new(ai_inference::AiInferenceRuntime::from_config(&config)?),
        config,
    })
}

// Extracted Wasmtime engine and pooling configuration.
pub(crate) fn build_engine(
    integrity_config: &IntegrityConfig,
    enable_fuel_metering: bool,
) -> Result<Engine> {
    let mut config = Config::new();
    config.consume_fuel(enable_fuel_metering);
    config.wasm_component_model(true);
    config.allocation_strategy(build_pooling_config(integrity_config)?);

    Engine::new(&config)
        .map_err(|error| anyhow!("failed to create Wasmtime engine with pooling enabled: {error}"))
}

pub(crate) fn build_command_engine(_integrity_config: &IntegrityConfig) -> Result<Engine> {
    let mut config = Config::new();
    config.wasm_component_model(true);

    Engine::new(&config)
        .map_err(|error| anyhow!("failed to create Wasmtime engine for batch execution: {error}"))
}

fn build_pooling_config(config: &IntegrityConfig) -> Result<PoolingAllocationConfig> {
    let total_route_concurrency = total_route_concurrency(&config.routes)?;
    let total_min_instances = total_min_instances(&config.routes)?;
    let mut pooling = PoolingAllocationConfig::new();

    pooling.total_component_instances(total_route_concurrency);
    pooling.total_core_instances(
        total_route_concurrency.saturating_mul(POOLING_CORE_INSTANCES_MULTIPLIER),
    );
    pooling.total_memories(total_route_concurrency.saturating_mul(POOLING_MEMORIES_MULTIPLIER));
    pooling.total_tables(total_route_concurrency.saturating_mul(POOLING_TABLES_MULTIPLIER));
    pooling.max_component_instance_size(POOLING_INSTANCE_METADATA_BYTES);
    pooling.max_core_instance_size(POOLING_INSTANCE_METADATA_BYTES);
    pooling.max_core_instances_per_component(POOLING_MAX_CORE_INSTANCES_PER_COMPONENT);
    pooling.max_memories_per_component(POOLING_MAX_MEMORIES_PER_COMPONENT);
    pooling.max_tables_per_component(POOLING_MAX_TABLES_PER_COMPONENT);
    pooling.max_memory_size(config.guest_memory_limit_bytes);
    pooling.max_unused_warm_slots(total_min_instances);

    Ok(pooling)
}
