#[path = "ai_inference/magnetar_runtime.rs"]
mod magnetar_runtime;

use anyhow::{Result, anyhow};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, RwLock,
        atomic::{AtomicU32, Ordering},
    },
};

use crate::{IntegrityConfig, IntegrityInferenceComponentBinding, RouteQos};

pub(crate) use magnetar_runtime::MAGNETAR_PATH_PREFIX;
const COMPONENT_META_JSON: &str = ".tachyon-component.json";
const MOCK_INFERENCE_RESPONSE: &str = "MOCK_COMPONENT_RESPONSE";

pub(crate) fn binding_runs_upstream(binding: &IntegrityInferenceComponentBinding) -> bool {
    let _ = binding;
    false
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum AcceleratorKind {
    #[default]
    Cpu,
    Gpu,
    Npu,
    Tpu,
    Network,
}

impl AcceleratorKind {
    pub(crate) const ALL: [Self; 5] = [Self::Cpu, Self::Gpu, Self::Npu, Self::Tpu, Self::Network];

    pub(crate) fn from_component_placement(device: &crate::ComponentPlacement) -> Self {
        match device {
            crate::ComponentPlacement::Cpu => Self::Cpu,
            crate::ComponentPlacement::Cuda | crate::ComponentPlacement::Metal => Self::Gpu,
            crate::ComponentPlacement::Npu => Self::Npu,
            crate::ComponentPlacement::Tpu => Self::Tpu,
        }
    }

    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Gpu => "gpu",
            Self::Npu => "npu",
            Self::Tpu => "tpu",
            Self::Network => "network",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AcceleratorMemoryResidency {
    HostRam,
    Vram,
    Sram,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InferenceExecutionTelemetry {
    pub(crate) alias: String,
    pub(crate) executed_on: String,
    pub(crate) succeeded: bool,
}

static INFERENCE_TELEMETRY: OnceLock<Mutex<Vec<InferenceExecutionTelemetry>>> = OnceLock::new();

/// Recovers from a poisoned lock while leaving an audit trail, mirroring
/// `telemetry::recover_poisoned`. A poisoned lock means some other thread
/// panicked while holding it, not that the guarded data is corrupt —
/// treating it as fatal here would turn one earlier panic into a
/// permanent source of repeated panics on every later access to the same
/// lock (audit finding TACH-02). Every poison event SHALL be logged so it
/// is never silently swallowed.
fn recover_poisoned<T>(context: &'static str, result: std::sync::LockResult<T>) -> T {
    match result {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!(
                target: "tachyon::ai_inference",
                lock = context,
                "lock was poisoned by an earlier panic; continuing with the recovered guard"
            );
            poisoned.into_inner()
        }
    }
}

fn record_execution(alias: impl Into<String>, executed_on: impl Into<String>, succeeded: bool) {
    let records = INFERENCE_TELEMETRY.get_or_init(|| Mutex::new(Vec::new()));
    let mut records = recover_poisoned("inference_telemetry", records.lock());
    records.push(InferenceExecutionTelemetry {
        alias: alias.into(),
        executed_on: executed_on.into(),
        succeeded,
    });
    if records.len() > 1024 {
        records.remove(0);
    }
}

pub(crate) fn inference_execution_telemetry() -> Vec<InferenceExecutionTelemetry> {
    let records = INFERENCE_TELEMETRY.get_or_init(|| Mutex::new(Vec::new()));
    recover_poisoned("inference_telemetry", records.lock()).clone()
}

/// One decoded event from a Component invocation stream. Opaque on both
/// arms: `Payload` is raw output bytes (never assumed to be UTF-8 text —
/// that interpretation belongs to whichever layer actually needs it, e.g.
/// `guest-openai` or the Magnetar adapter), `Metadata` is whatever tagged
/// key/value pairs the Component's own execution chose to attach mid-stream.
/// Mirrors `wit/accelerator/*.wit`'s `stream-event` variant exactly.
pub(crate) enum StreamEvent<'a> {
    Payload(&'a [u8]),
    Metadata(Vec<(String, String)>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StreamControl {
    Continue,
    Stop,
}

impl StreamControl {
    pub(crate) fn is_stop(self) -> bool {
        matches!(self, Self::Stop)
    }
}

pub(crate) trait StreamSink {
    fn emit(&mut self, event: StreamEvent<'_>) -> StreamControl;

    fn is_live(&mut self) -> bool {
        true
    }
}

impl<F> StreamSink for F
where
    F: FnMut(StreamEvent<'_>) -> StreamControl + ?Sized,
{
    fn emit(&mut self, event: StreamEvent<'_>) -> StreamControl {
        self(event)
    }
}

/// Opaque metadata a Component invocation reports once it completes —
/// whatever tagged key/value pairs the Component's own execution (or its
/// adapter, e.g. Magnetar) chose to attach. Tachyon core does not name or
/// interpret these; see `wit/accelerator/*.wit`'s `invocation-result.metadata`.
#[derive(Clone, Debug, Default)]
pub(crate) struct StreamOutcome {
    pub(crate) metadata: Vec<(String, String)>,
}

/// A Component invocation failure. Mirrors `wit/accelerator/*.wit`'s
/// `invocation-error` record field for field; Tachyon core does not attach
/// any dialect-specific error taxonomy beyond what that opaque contract
/// carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ComponentInvocationError {
    pub(crate) message: String,
    pub(crate) upstream_status: Option<u16>,
    pub(crate) invalid_request: bool,
}

impl ComponentInvocationError {
    pub(crate) fn local(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            upstream_status: None,
            invalid_request: false,
        }
    }

    pub(crate) fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            upstream_status: None,
            invalid_request: true,
        }
    }
}

impl std::fmt::Display for ComponentInvocationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<String> for ComponentInvocationError {
    fn from(message: String) -> Self {
        Self::local(message)
    }
}

/// The result of one buffered Component invocation: opaque output bytes plus
/// whatever opaque metadata tags the Component's execution attached. Mirrors
/// `wit/accelerator/*.wit`'s `invocation-result` record.
#[derive(Clone, Debug, Default)]
pub(crate) struct ComponentInvocationOutcome {
    pub(crate) payload: Vec<u8>,
    pub(crate) metadata: Vec<(String, String)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct QueueTierSnapshot {
    pub(crate) realtime: u32,
    pub(crate) standard: u32,
    pub(crate) batch: u32,
}

#[cfg(test)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SchedulerSnapshot {
    pub(crate) batches_processed: usize,
    pub(crate) requests_processed: usize,
    pub(crate) max_batch_size: usize,
    pub(crate) prefill_steps_processed: usize,
    pub(crate) decode_steps_processed: usize,
    pub(crate) max_active_sequences: usize,
    pub(crate) queued_requests: usize,
    pub(crate) realtime_queued: usize,
    pub(crate) standard_queued: usize,
    pub(crate) batch_queued: usize,
    pub(crate) kv_recompute_preemptions: usize,
    pub(crate) kv_swap_preemptions: usize,
    pub(crate) completed_aliases: Vec<String>,
}

/// A Magnetar Component instance serializes its own execution internally:
/// `LoadedInferenceComponent::invoke_payload_opaque`'s own doc comment in
/// the vendored crate says it blocks until any generation already in
/// flight on that instance finishes. Tachyon holds exactly one instance
/// per alias, so this was previously an invisible property of a
/// dependency — a second concurrent request for the same alias just
/// blocked silently with no signal to Tachyon at all (audit finding
/// TACH-03). This constant makes that ceiling an explicit, Tachyon-owned
/// admission fact: `AliasInFlightGuard` counts requests in flight per
/// alias and logs a structured warning the moment a second one starts
/// before the first finishes, so operators can see the contention Magnetar
/// is silently absorbing instead of only inferring it from tail latency.
/// Deliberately non-rejecting — raising it to a hard reject would turn a
/// currently-succeeding (if serialized) traffic pattern into request
/// failures, which is a bigger behavior change than this audit finding
/// calls for.
pub(crate) const MAGNETAR_MAX_CONCURRENCY_PER_ALIAS: u32 = 1;

/// RAII admission tracker enforcing visibility (not rejection) of
/// [`MAGNETAR_MAX_CONCURRENCY_PER_ALIAS`] for one Magnetar alias. Held for
/// the duration of one `invoke`/`invoke_streaming` call; `Drop` always
/// decrements, including on early return or panic-unwind, so the count
/// can never leak upward under error paths.
struct AliasInFlightGuard {
    in_flight: Arc<AtomicU32>,
}

impl AliasInFlightGuard {
    fn enter(alias: &str, in_flight: &Arc<AtomicU32>) -> Self {
        let previous = in_flight.fetch_add(1, Ordering::AcqRel);
        if previous >= MAGNETAR_MAX_CONCURRENCY_PER_ALIAS {
            tracing::warn!(
                target: "tachyon::ai_inference",
                alias,
                in_flight = previous + 1,
                limit = MAGNETAR_MAX_CONCURRENCY_PER_ALIAS,
                "concurrent Magnetar invocations for this alias exceed the declared per-alias concurrency limit; Magnetar is serializing them internally and this request is blocked behind an in-flight one"
            );
        }
        Self {
            in_flight: Arc::clone(in_flight),
        }
    }
}

impl Drop for AliasInFlightGuard {
    fn drop(&mut self) {
        self.in_flight.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone)]
enum ComponentRuntime {
    Mock {
        accelerator: AcceleratorKind,
    },
    Magnetar {
        runtime: Arc<magnetar_runtime::MagnetarRuntime>,
        /// The generic accelerator class Tachyon itself requested when this
        /// Component was loaded (`AcceleratorKind::from_component_placement`
        /// in `load_binding`). Placement failures fail the load outright —
        /// Magnetar never silently substitutes a different class on success
        /// — so this is exactly what the bound Component is running on,
        /// without reading anything back out of Magnetar's own Provider
        /// advertisement (audit round 5, TACH-01).
        accelerator: AcceleratorKind,
        /// Shared with every clone of this alias's `LoadedInferenceComponent`
        /// entry (never re-created per clone) so it accurately reflects
        /// concurrent callers of the one Magnetar instance this alias
        /// resolves to. See [`AliasInFlightGuard`].
        in_flight: Arc<AtomicU32>,
    },
}

#[derive(Clone)]
struct LoadedInferenceComponent {
    alias: String,
    qos: RouteQos,
    runtime: ComponentRuntime,
}

impl LoadedInferenceComponent {
    fn accelerator(&self) -> AcceleratorKind {
        match &self.runtime {
            ComponentRuntime::Mock { accelerator }
            | ComponentRuntime::Magnetar { accelerator, .. } => *accelerator,
        }
    }

    fn memory_residency(&self) -> AcceleratorMemoryResidency {
        match self.accelerator() {
            AcceleratorKind::Gpu => AcceleratorMemoryResidency::Vram,
            AcceleratorKind::Npu | AcceleratorKind::Tpu => AcceleratorMemoryResidency::Sram,
            AcceleratorKind::Cpu | AcceleratorKind::Network => AcceleratorMemoryResidency::HostRam,
        }
    }
}

#[derive(Clone)]
pub(crate) struct AiInferenceRuntime {
    inference_components: Arc<RwLock<HashMap<String, LoadedInferenceComponent>>>,
    dynamic_components_root: Option<PathBuf>,
    queue_snapshots: Arc<RwLock<HashMap<AcceleratorKind, QueueTierSnapshot>>>,
}

impl AiInferenceRuntime {
    pub(crate) fn from_config(config: &IntegrityConfig) -> Result<Self> {
        assert_no_credential_collisions(
            config
                .routes
                .iter()
                .flat_map(|route| route.inference_components.iter())
                .filter(|binding| binding_runs_upstream(binding))
                .map(|binding| binding.alias.as_str()),
        )
        .map_err(|detail| anyhow!("Integrity Validation Failed: {detail}"))?;

        let mut components = HashMap::new();
        let mut sealed_aliases = HashSet::new();
        for route in &config.routes {
            if route
                .artifact_id
                .as_deref()
                .is_some_and(|artifact_id| !artifact_id.trim().is_empty())
                && route
                    .inference_components
                    .iter()
                    .any(|binding| binding.dynamic || !binding_runs_upstream(binding))
            {
                return Err(anyhow!(
                    "Integrity Validation Failed: route `{}` uses legacy adapter binding with local AI inference; adapters must be implemented by the selected Magnetar Component",
                    route.path
                ));
            }
            for binding in &route.inference_components {
                if !sealed_aliases.insert(binding.alias.clone()) {
                    return Err(anyhow!(
                        "Integrity Validation Failed: component alias `{}` must be globally unique",
                        binding.alias
                    ));
                }
                if binding.dynamic {
                    continue;
                }
                if binding.path.trim().is_empty() {
                    return Err(anyhow!(
                        "Integrity Validation Failed: static component alias `{}` requires a non-empty `path` (set `dynamic: true` for broker-uploaded components)",
                        binding.alias
                    ));
                }
                let component = load_binding(binding)?;
                components.insert(binding.alias.clone(), component);
            }
        }
        Ok(Self {
            inference_components: Arc::new(RwLock::new(components)),
            dynamic_components_root: None,
            queue_snapshots: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub(crate) fn with_dynamic_components_root(mut self, root: Option<PathBuf>) -> Self {
        self.dynamic_components_root = root;
        self
    }

    fn ensure_component_loaded(
        &self,
        alias: &str,
        requested_accelerator: AcceleratorKind,
    ) -> Result<(), String> {
        if self
            .inference_components
            .read()
            .map_err(|_| "component registry lock poisoned".to_owned())?
            .contains_key(alias)
        {
            return Ok(());
        }
        let Some(root) = self.dynamic_components_root.as_ref() else {
            return Err(format!("component alias `{alias}` is not loaded"));
        };
        let component_dir = root.join(alias);
        if !component_dir.is_dir() {
            return Err(format!("component alias `{alias}` is not loaded"));
        }
        let binding = IntegrityInferenceComponentBinding {
            alias: alias.to_owned(),
            path: format!("magnetar:{}", component_dir.display()),
            device: component_placement_for_accelerator(requested_accelerator)?,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        };
        let component = load_binding(&binding).map_err(|error| format!("{error:#}"))?;
        self.inference_components
            .write()
            .map_err(|_| "component registry lock poisoned".to_owned())?
            .entry(alias.to_owned())
            .or_insert(component);
        Ok(())
    }

    pub(crate) fn loaded_component_aliases(&self) -> Vec<String> {
        let registry = recover_poisoned("component_registry", self.inference_components.read());
        let mut aliases = registry.keys().cloned().collect::<Vec<_>>();
        aliases.sort();
        aliases
    }

    pub(crate) fn supports_accelerator(&self, accelerator: AcceleratorKind) -> bool {
        matches!(accelerator, AcceleratorKind::Cpu) || {
            let registry = recover_poisoned("component_registry", self.inference_components.read());
            registry
                .values()
                .any(|component| component.accelerator() == accelerator)
        }
    }

    pub(crate) fn queue_tier_snapshot(&self, accelerator: AcceleratorKind) -> QueueTierSnapshot {
        let snapshots = recover_poisoned("queue_snapshots", self.queue_snapshots.read());
        snapshots.get(&accelerator).copied().unwrap_or_default()
    }

    pub(crate) fn load_inference_component(
        &self,
        alias: &str,
        accelerator: AcceleratorKind,
    ) -> std::result::Result<(), String> {
        self.ensure_component_loaded(alias, accelerator)?;
        let components = self
            .inference_components
            .read()
            .map_err(|_| "component registry lock poisoned".to_owned())?;
        let component = components
            .get(alias)
            .ok_or_else(|| format!("component alias `{alias}` is not loaded"))?;
        if component.accelerator() != accelerator {
            return Err(format!(
                "component alias `{alias}` is loaded for `{}` but `{}` was requested",
                component.accelerator().as_str(),
                accelerator.as_str()
            ));
        }
        Ok(())
    }

    /// Test convenience only: no production call site needs readable text
    /// back, so this is the one place in the production API surface that
    /// still assumes the Component's output is UTF-8. Everything it calls
    /// (and everything `component_hosts.rs` calls directly) stays opaque.
    #[cfg(test)]
    pub(crate) fn compute_component_prompt(
        &self,
        alias: &str,
        payload: &[u8],
    ) -> std::result::Result<String, ComponentInvocationError> {
        self.invoke_loaded_component(alias, payload)
            .and_then(|outcome| {
                String::from_utf8(outcome.payload)
                    .map_err(|error| ComponentInvocationError::local(error.to_string()))
            })
    }

    pub(crate) fn invoke_loaded_component(
        &self,
        alias: &str,
        payload: &[u8],
    ) -> std::result::Result<ComponentInvocationOutcome, ComponentInvocationError> {
        self.ensure_component_loaded(alias, AcceleratorKind::Cpu)
            .map_err(ComponentInvocationError::local)?;
        let component = self
            .inference_components
            .read()
            .map_err(|_| ComponentInvocationError::local("component registry lock poisoned"))?
            .get(alias)
            .cloned()
            .ok_or_else(|| {
                ComponentInvocationError::local(format!("component alias `{alias}` is not loaded"))
            })?;
        let _queue_depth = self.track_queue_depth(component.accelerator(), component.qos);
        let output = invoke_component(&component, payload)?;
        Ok(ComponentInvocationOutcome {
            payload: output.bytes,
            metadata: output.metadata,
        })
    }

    pub(crate) fn stream_loaded_component(
        &self,
        alias: &str,
        payload: &[u8],
        sink: &mut dyn StreamSink,
    ) -> std::result::Result<StreamOutcome, ComponentInvocationError> {
        self.ensure_component_loaded(alias, AcceleratorKind::Cpu)
            .map_err(ComponentInvocationError::local)?;
        let component = self
            .inference_components
            .read()
            .map_err(|_| ComponentInvocationError::local("component registry lock poisoned"))?
            .get(alias)
            .cloned()
            .ok_or_else(|| {
                ComponentInvocationError::local(format!("component alias `{alias}` is not loaded"))
            })?;
        let _queue_depth = self.track_queue_depth(component.accelerator(), component.qos);
        match &component.runtime {
            ComponentRuntime::Mock { .. } => {
                if sink.is_live() {
                    sink.emit(StreamEvent::Payload(MOCK_INFERENCE_RESPONSE.as_bytes()));
                }
                record_execution(&component.alias, component.accelerator().as_str(), true);
                Ok(StreamOutcome {
                    metadata: mock_component_metadata(payload, MOCK_INFERENCE_RESPONSE.as_bytes()),
                })
            }
            ComponentRuntime::Magnetar {
                runtime, in_flight, ..
            } => {
                let _admission = AliasInFlightGuard::enter(&component.alias, in_flight);
                let mut emit = |frame: &[u8]| {
                    if sink.is_live() {
                        sink.emit(StreamEvent::Payload(frame))
                    } else {
                        StreamControl::Stop
                    }
                };
                let result = runtime
                    .invoke_streaming(payload, &mut emit)
                    .map(|metadata| StreamOutcome { metadata })
                    .map_err(magnetar_invocation_error);
                record_execution(
                    &component.alias,
                    component.accelerator().as_str(),
                    result.is_ok(),
                );
                result
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn scheduler_snapshot(&self, _accelerator: AcceleratorKind) -> SchedulerSnapshot {
        SchedulerSnapshot::default()
    }

    #[cfg(test)]
    pub(crate) fn set_queue_depth_for_test(
        &self,
        accelerator: AcceleratorKind,
        qos: RouteQos,
        depth: usize,
    ) {
        let depth = depth.min(u32::MAX as usize) as u32;
        let mut snapshots = self
            .queue_snapshots
            .write()
            .expect("queue snapshots lock poisoned");
        let snapshot = snapshots.entry(accelerator).or_default();
        match qos {
            RouteQos::RealTime => snapshot.realtime = depth,
            RouteQos::Standard => snapshot.standard = depth,
            RouteQos::Batch => snapshot.batch = depth,
        }
    }

    #[cfg(test)]
    pub(crate) fn component_memory_residency(
        &self,
        alias: &str,
    ) -> Option<AcceleratorMemoryResidency> {
        self.inference_components
            .read()
            .expect("component registry lock poisoned")
            .get(alias)
            .map(LoadedInferenceComponent::memory_residency)
    }

    #[cfg(test)]
    pub(crate) fn magnetar_resident_debug(&self, alias: &str) -> Option<(String, usize)> {
        self.inference_components
            .read()
            .expect("component registry lock poisoned")
            .get(alias)
            .and_then(|component| match &component.runtime {
                ComponentRuntime::Magnetar { runtime, .. } => runtime.resident_debug().ok(),
                _ => None,
            })
    }

    fn track_queue_depth(&self, accelerator: AcceleratorKind, qos: RouteQos) -> QueueDepthGuard {
        if matches!(accelerator, AcceleratorKind::Network) {
            return QueueDepthGuard::noop();
        }
        {
            let mut snapshots = recover_poisoned("queue_snapshots", self.queue_snapshots.write());
            adjust_queue_depth(&mut snapshots, accelerator, qos, 1);
        }
        QueueDepthGuard {
            snapshots: Some(Arc::clone(&self.queue_snapshots)),
            accelerator,
            qos,
        }
    }
}

fn magnetar_invocation_error(error: anyhow::Error) -> ComponentInvocationError {
    if magnetar_runtime::is_invalid_component_invocation(&error) {
        ComponentInvocationError::invalid_request(error.to_string())
    } else {
        ComponentInvocationError::local(error.to_string())
    }
}

fn component_placement_for_accelerator(
    accelerator: AcceleratorKind,
) -> Result<crate::ComponentPlacement, String> {
    match accelerator {
        AcceleratorKind::Cpu => Ok(crate::ComponentPlacement::Cpu),
        AcceleratorKind::Gpu => Ok(crate::ComponentPlacement::Cuda),
        AcceleratorKind::Npu => Ok(crate::ComponentPlacement::Npu),
        AcceleratorKind::Tpu => Ok(crate::ComponentPlacement::Tpu),
        AcceleratorKind::Network => {
            Err("network accelerator cannot load local components".to_owned())
        }
    }
}

struct QueueDepthGuard {
    snapshots: Option<Arc<RwLock<HashMap<AcceleratorKind, QueueTierSnapshot>>>>,
    accelerator: AcceleratorKind,
    qos: RouteQos,
}

impl QueueDepthGuard {
    fn noop() -> Self {
        Self {
            snapshots: None,
            accelerator: AcceleratorKind::Cpu,
            qos: RouteQos::Standard,
        }
    }
}

impl Drop for QueueDepthGuard {
    fn drop(&mut self) {
        let Some(snapshots) = &self.snapshots else {
            return;
        };
        let mut snapshots = recover_poisoned("queue_snapshots", snapshots.write());
        adjust_queue_depth(&mut snapshots, self.accelerator, self.qos, -1);
    }
}

fn adjust_queue_depth(
    snapshots: &mut HashMap<AcceleratorKind, QueueTierSnapshot>,
    accelerator: AcceleratorKind,
    qos: RouteQos,
    delta: i32,
) {
    let snapshot = snapshots.entry(accelerator).or_default();
    let slot = match qos {
        RouteQos::RealTime => &mut snapshot.realtime,
        RouteQos::Standard => &mut snapshot.standard,
        RouteQos::Batch => &mut snapshot.batch,
    };
    if delta.is_positive() {
        *slot = slot.saturating_add(delta as u32);
    } else {
        *slot = slot.saturating_sub(delta.unsigned_abs());
    }
}

struct ComponentOutput {
    bytes: Vec<u8>,
    metadata: Vec<(String, String)>,
}

fn load_binding(binding: &IntegrityInferenceComponentBinding) -> Result<LoadedInferenceComponent> {
    let path = binding.path.trim();
    let accelerator = AcceleratorKind::from_component_placement(&binding.device);
    let runtime = if path == "mock" || path.starts_with("mock:") {
        ComponentRuntime::Mock { accelerator }
    } else {
        match magnetar_runtime::MagnetarRuntime::try_load(
            &binding.alias,
            path,
            binding.device.as_str(),
        )? {
            Some(runtime) => ComponentRuntime::Magnetar {
                runtime: Arc::new(runtime),
                accelerator,
                in_flight: Arc::new(AtomicU32::new(0)),
            },
            _ => {
                if magnetar_runtime::is_magnetar_path(path) {
                    return Err(anyhow!(
                        "unsupported Magnetar Component binding `{}` at `{}`: expected an authorized inference Component artifact directory",
                        binding.alias,
                        binding.path
                    ));
                } else {
                    return Err(anyhow!(
                        "unsupported AI binding `{}` at `{}`: local inference accepts explicit mock paths or magnetar Component artifact directories; remote provider protocols must run in a guest or Component",
                        binding.alias,
                        binding.path
                    ));
                }
            }
        }
    };
    Ok(LoadedInferenceComponent {
        alias: binding.alias.clone(),
        qos: binding.qos,
        runtime,
    })
}

fn invoke_component(
    component: &LoadedInferenceComponent,
    payload: &[u8],
) -> std::result::Result<ComponentOutput, ComponentInvocationError> {
    match &component.runtime {
        ComponentRuntime::Mock { .. } => {
            record_execution(&component.alias, component.accelerator().as_str(), true);
            Ok(ComponentOutput {
                bytes: MOCK_INFERENCE_RESPONSE.as_bytes().to_vec(),
                metadata: mock_component_metadata(payload, MOCK_INFERENCE_RESPONSE.as_bytes()),
            })
        }
        ComponentRuntime::Magnetar {
            runtime, in_flight, ..
        } => {
            let _admission = AliasInFlightGuard::enter(&component.alias, in_flight);
            let result = runtime
                .invoke(payload)
                .map_err(magnetar_invocation_error)
                .map(|(bytes, metadata)| ComponentOutput { bytes, metadata });
            record_execution(
                &component.alias,
                component.accelerator().as_str(),
                result.is_ok(),
            );
            result
        }
    }
}

/// Whatever opaque key/value metadata a Component artifact's sidecar
/// (`.tachyon-component.json`) declares, read and returned verbatim.
/// Tachyon does not name, parse, or interpret any key here — a sidecar
/// author writes directly in whatever wire shape the consuming guest or
/// Component expects (e.g. a tool-call dialect under whatever key it
/// reads); only that downstream boundary assigns any key meaning.
///
/// A missing sidecar is a legitimate "declares nothing" and yields an
/// empty bag, but a sidecar that exists and fails to read (permissions,
/// I/O) or fails to parse (invalid JSON, or valid JSON that is not a
/// flat string-keyed object) is an error, not silently treated the same
/// as absence — audit finding TACH-04. Collapsing "absent" and "present
/// but broken" into the same empty result hid real authoring mistakes
/// behind a component that looked like it simply declared nothing.
pub(crate) fn declared_component_metadata(
    root: &Path,
) -> std::result::Result<BTreeMap<String, String>, String> {
    let sidecar_path = root.join(COMPONENT_META_JSON);
    let raw = match std::fs::read(&sidecar_path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => {
            return Err(format!(
                "failed to read component sidecar `{}`: {error}",
                sidecar_path.display()
            ));
        }
    };
    serde_json::from_slice(&raw).map_err(|error| {
        format!(
            "component sidecar `{}` is not valid opaque metadata (expected a flat JSON object of string keys to string values): {error}",
            sidecar_path.display()
        )
    })
}

pub(crate) fn assert_no_credential_collisions<'a>(
    aliases: impl IntoIterator<Item = &'a str>,
) -> std::result::Result<(), String> {
    let mut seen = HashSet::new();
    for alias in aliases {
        let env_name = alias
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() {
                    ch.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect::<String>();
        if !seen.insert(env_name.clone()) {
            return Err(format!(
                "multiple upstream aliases resolve to credential environment variable `{env_name}`"
            ));
        }
    }
    Ok(())
}

/// Test-only stand-in for what a real Component's own tags might look like,
/// in the same opaque `Vec<(String, String)>` shape `MagnetarRuntime::
/// invoke`/`invoke_streaming` hand back untouched from a real Magnetar
/// Component. This is a test double's own fixture data, not core inference
/// logic: it exists solely so integration tests exercising the `mock:`
/// Component through the full host↔guest wire can observe non-empty,
/// input-dependent tags without a real Component present. It is not
/// reachable outside `cargo test` — see the `#[cfg(not(test))]` stub below,
/// which is what every non-test build actually ships.
#[cfg(test)]
fn mock_component_metadata(prompt: &[u8], completion: &[u8]) -> Vec<(String, String)> {
    let prompt_tokens = String::from_utf8_lossy(prompt)
        .split_whitespace()
        .count()
        .max(1);
    let completion_tokens = String::from_utf8_lossy(completion)
        .split_whitespace()
        .count()
        .max(1);
    vec![
        ("prompt_tokens".to_owned(), prompt_tokens.to_string()),
        ("generated_tokens".to_owned(), completion_tokens.to_string()),
    ]
}

/// The `mock:` Component's real (non-test) behavior: a transport double with
/// no metadata at all. Tachyon's core has no component to count tokens for and
/// must not pretend otherwise, so a non-test build reports nothing rather
/// than fabricating plausible-looking numbers.
#[cfg(not(test))]
fn mock_component_metadata(_prompt: &[u8], _completion: &[u8]) -> Vec<(String, String)> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ComponentPlacement, IntegrityRoute};
    use std::{fs, io::Write};

    fn unique_component_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tachyon-magnetar-cutover-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ))
    }

    fn metadata_u32(metadata: &[(String, String)], key: &str) -> Option<u32> {
        metadata
            .iter()
            .find(|(candidate, _)| candidate == key)
            .and_then(|(_, value)| value.parse().ok())
    }

    #[test]
    fn alias_in_flight_guard_counts_concurrent_holders_and_releases_on_drop() {
        let in_flight = Arc::new(AtomicU32::new(0));
        assert_eq!(in_flight.load(Ordering::Acquire), 0);

        let first = AliasInFlightGuard::enter("qwen-alias", &in_flight);
        assert_eq!(
            in_flight.load(Ordering::Acquire),
            1,
            "one in-flight caller should be counted"
        );

        // A second concurrent holder for the same alias exceeds
        // MAGNETAR_MAX_CONCURRENCY_PER_ALIAS (1) — TACH-03 makes this an
        // observable fact (a warning is logged) rather than an invisible
        // block inside Magnetar's own instance mutex, without rejecting it.
        let second = AliasInFlightGuard::enter("qwen-alias", &in_flight);
        assert_eq!(
            in_flight.load(Ordering::Acquire),
            2,
            "a second concurrent caller must still be admitted, only observed"
        );

        drop(second);
        assert_eq!(
            in_flight.load(Ordering::Acquire),
            1,
            "dropping one guard must release exactly its own admission"
        );

        drop(first);
        assert_eq!(
            in_flight.load(Ordering::Acquire),
            0,
            "dropping the last guard must return the counter to zero"
        );
    }

    const HIDDEN_SIZE: u64 = 4;
    const LAYER_COUNT: u64 = 1;
    const ATTENTION_HEAD_COUNT: u64 = 2;
    const KV_HEAD_COUNT: u64 = 2;
    const HEAD_DIMENSION: u64 = 2;
    const INTERMEDIATE_SIZE: u64 = 8;
    const VOCAB_SIZE: u64 = 16;

    fn tensor_value(seed: u64) -> f32 {
        let mut x = seed
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(0x2545_F491_4F6C_DD1D);
        x ^= x >> 33;
        x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
        x ^= x >> 33;
        ((x % 1000) as f32 / 1000.0) - 0.5
    }

    fn tensor_values(name: &str, element_count: u64) -> Vec<f32> {
        let mut hash: u64 = 0xCBF2_9CE4_8422_2325;
        for byte in name.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        }
        (0..element_count)
            .map(|index| tensor_value(hash.wrapping_add(index)))
            .collect()
    }

    fn write_tiny_production_qwen_bundle(path: &Path) {
        fs::create_dir_all(path).expect("fixture dir should be created");
        fs::write(
            path.join("qwen-real.component.wasm"),
            include_bytes!("../../vendor/Magnetar/magnetar-runtime/fixtures/components/qwen-real.component.wasm"),
        )
        .expect("Component artifact should be written");
        fs::write(
            path.join("qwen-real.component.wasm.magnetar-component.yaml"),
            include_bytes!("../../vendor/Magnetar/magnetar-runtime/fixtures/components/qwen-real.component.wasm.magnetar-component.yaml"),
        )
        .expect("Component artifact manifest should be written");
        write_tiny_production_model_data(path);
    }

    /// The same real, independently-compiled Llama Component Magnetar's own
    /// test suite checks in (`loaded_inference_component_load_runs_a_real_
    /// second_architecture_end_to_end`), paired with a Model Artifact whose
    /// `config.json` declares `model_type: "llama"` — Magnetar's own
    /// `magnetar-loader-huggingface` normalizes that directly into
    /// `architecture.family` (astorise/Magnetar#83), and `llama-real`'s own
    /// manifest declares `compatibility.architecture_families: [llama]`
    /// (Tachyon integration audit round 2, TACH-05 fixture bug: this used
    /// to share `write_tiny_production_qwen_bundle`'s `qwen2`-declaring
    /// Model Artifact, which loaded only because Magnetar had no
    /// Component-vs-family compatibility gate yet; it now rejects that
    /// mismatch on purpose). Same tensor shapes and tokenizer as the Qwen
    /// bundle — HIDDEN_SIZE/LAYER_COUNT/etc. describe a generic HF-style
    /// transformer either family accepts; only the declared family differs.
    /// Proves Tachyon's own route -> alias -> `MagnetarRuntime::try_load`
    /// pipeline can load a second, digest-distinct compiled Component
    /// binary, not just a second alias (Tachyon integration audit
    /// MAG-01/MAG-06, #72).
    fn write_tiny_production_llama_bundle(path: &Path) {
        fs::create_dir_all(path).expect("fixture dir should be created");
        fs::write(
            path.join("llama-real.component.wasm"),
            include_bytes!("../../vendor/Magnetar/magnetar-runtime/fixtures/components/llama-real.component.wasm"),
        )
        .expect("Component artifact should be written");
        fs::write(
            path.join("llama-real.component.wasm.magnetar-component.yaml"),
            include_bytes!("../../vendor/Magnetar/magnetar-runtime/fixtures/components/llama-real.component.wasm.magnetar-component.yaml"),
        )
        .expect("Component artifact manifest should be written");
        write_tiny_production_model_data_for_family(path, "LlamaForCausalLM", "llama");
    }

    /// The Model Artifact half of a tiny production-shaped bundle: config,
    /// tokenizer, and safetensors weights. Shared by every fixture Component
    /// this module writes, since each accepts the identical Hugging
    /// Face-shaped directory — only the `*.component.wasm` (and, since
    /// Magnetar's Component-vs-family compatibility gate landed, the
    /// declared `architectures`/`model_type`) differ.
    fn write_tiny_production_model_data(path: &Path) {
        write_tiny_production_model_data_for_family(path, "Qwen2ForCausalLM", "qwen2");
    }

    fn write_tiny_production_model_data_for_family(
        path: &Path,
        architectures: &str,
        model_type: &str,
    ) {
        let config_json = format!(
            r#"{{
                "architectures": ["{architectures}"],
                "model_type": "{model_type}",
                "hidden_size": {HIDDEN_SIZE},
                "intermediate_size": {INTERMEDIATE_SIZE},
                "num_hidden_layers": {LAYER_COUNT},
                "num_attention_heads": {ATTENTION_HEAD_COUNT},
                "num_key_value_heads": {KV_HEAD_COUNT},
                "head_dim": {HEAD_DIMENSION},
                "vocab_size": {VOCAB_SIZE},
                "rms_norm_eps": 1e-6,
                "rope_theta": 10000.0,
                "tie_word_embeddings": false,
                "torch_dtype": "float32",
                "bos_token_id": 0,
                "eos_token_id": 1
            }}"#
        );
        fs::write(path.join("config.json"), config_json).expect("config should be written");
        // Magnetar's `ProductionModelSource` ingestion no longer infers the
        // bundle's ingestor from filesystem heuristics (astorise/Magnetar#75):
        // it requires this sidecar, symmetric to a Component's own
        // `.magnetar-component.yaml`, written by whatever built the bundle —
        // here, this fixture itself. Every fixture this helper backs uses
        // the Hugging Face-shaped directory (config/tokenizer/safetensors).
        fs::write(
            path.join("magnetar-artifact-format.yaml"),
            "artifact_format: huggingface\n",
        )
        .expect("artifact-format sidecar should be written");

        let vocab_entries = [
            "<bos>", "<eos>", "hi", "h", "i", " ", "t", "e", "r", "wo", "ld", "!", "a", "b", "c",
            "d",
        ];
        let mut vocab_json = String::from("{");
        for (id, token) in vocab_entries.iter().take(VOCAB_SIZE as usize).enumerate() {
            if id > 0 {
                vocab_json.push(',');
            }
            vocab_json.push_str(&format!("\"{token}\":{id}"));
        }
        vocab_json.push('}');
        let tokenizer_json = format!(
            r#"{{
                "version": "1.0",
                "truncation": null,
                "padding": null,
                "added_tokens": [
                    {{"id": 0, "content": "<bos>", "special": true, "single_word": false, "lstrip": false, "rstrip": false, "normalized": false}},
                    {{"id": 1, "content": "<eos>", "special": true, "single_word": false, "lstrip": false, "rstrip": false, "normalized": false}}
                ],
                "normalizer": null,
                "pre_tokenizer": null,
                "post_processor": null,
                "decoder": null,
                "model": {{"type": "WordLevel", "vocab": {vocab_json}, "unk_token": "h"}}
            }}"#
        );
        fs::write(path.join("tokenizer.json"), tokenizer_json)
            .expect("tokenizer should be written");
        fs::write(
            path.join("tokenizer_config.json"),
            r#"{"bos_token": "<bos>", "eos_token": "<eos>", "chat_template": "{% for message in messages %}{{ message.role }}: {{ message.content }}\n{% endfor %}assistant: "}"#,
        )
        .expect("tokenizer config should be written");

        let q_dim = ATTENTION_HEAD_COUNT * HEAD_DIMENSION;
        let kv_dim = KV_HEAD_COUNT * HEAD_DIMENSION;
        let mut tensors: Vec<(String, Vec<u64>)> = vec![
            (
                "model.embed_tokens.weight".into(),
                vec![VOCAB_SIZE, HIDDEN_SIZE],
            ),
            ("model.norm.weight".into(), vec![HIDDEN_SIZE]),
            ("lm_head.weight".into(), vec![VOCAB_SIZE, HIDDEN_SIZE]),
        ];
        for layer in 0..LAYER_COUNT {
            tensors.push((
                format!("model.layers.{layer}.input_layernorm.weight"),
                vec![HIDDEN_SIZE],
            ));
            tensors.push((
                format!("model.layers.{layer}.self_attn.q_proj.weight"),
                vec![q_dim, HIDDEN_SIZE],
            ));
            tensors.push((
                format!("model.layers.{layer}.self_attn.k_proj.weight"),
                vec![kv_dim, HIDDEN_SIZE],
            ));
            tensors.push((
                format!("model.layers.{layer}.self_attn.v_proj.weight"),
                vec![kv_dim, HIDDEN_SIZE],
            ));
            tensors.push((
                format!("model.layers.{layer}.self_attn.o_proj.weight"),
                vec![HIDDEN_SIZE, q_dim],
            ));
            tensors.push((
                format!("model.layers.{layer}.post_attention_layernorm.weight"),
                vec![HIDDEN_SIZE],
            ));
            tensors.push((
                format!("model.layers.{layer}.mlp.gate_proj.weight"),
                vec![INTERMEDIATE_SIZE, HIDDEN_SIZE],
            ));
            tensors.push((
                format!("model.layers.{layer}.mlp.up_proj.weight"),
                vec![INTERMEDIATE_SIZE, HIDDEN_SIZE],
            ));
            tensors.push((
                format!("model.layers.{layer}.mlp.down_proj.weight"),
                vec![HIDDEN_SIZE, INTERMEDIATE_SIZE],
            ));
        }

        let mut header = String::from("{");
        let mut data = Vec::new();
        for (name, shape) in &tensors {
            let element_count: u64 = shape.iter().product();
            let values = tensor_values(name, element_count);
            let start = data.len() as u64;
            for value in &values {
                data.extend_from_slice(&value.to_le_bytes());
            }
            let end = data.len() as u64;
            let shape_text = shape
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(",");
            header.push_str(&format!(
                "\"{name}\":{{\"dtype\":\"F32\",\"shape\":[{shape_text}],\"data_offsets\":[{start},{end}]}}",
            ));
            header.push(',');
        }
        header.pop();
        header.push('}');

        let mut file =
            fs::File::create(path.join("model.safetensors")).expect("safetensors should create");
        file.write_all(&(header.len() as u64).to_le_bytes())
            .expect("safetensors header len should write");
        file.write_all(header.as_bytes())
            .expect("safetensors header should write");
        file.write_all(&data)
            .expect("safetensors payload should write");
    }

    static TRUST_ENV_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

    struct QwenInterprocessGuard {
        path: PathBuf,
    }

    /// Restores both trust env vars independently (they're set together by
    /// `trust_tachyon_component_artifact` but tests sometimes clear just one
    /// to isolate a single trust decision — see
    /// `qwen_bundle_cannot_self_authorize_with_embedded_trust_policy`).
    struct TrustStoreEnvGuard {
        _qwen_lock: QwenInterprocessGuard,
        _lock: std::sync::MutexGuard<'static, ()>,
        previous_component: Option<std::ffi::OsString>,
        previous_model: Option<std::ffi::OsString>,
        component_path: PathBuf,
        model_path: PathBuf,
    }

    struct TrustStoreEnvUnsetGuard {
        _qwen_lock: QwenInterprocessGuard,
        _lock: std::sync::MutexGuard<'static, ()>,
        previous_component: Option<std::ffi::OsString>,
        previous_model: Option<std::ffi::OsString>,
    }

    impl Drop for QwenInterprocessGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    impl Drop for TrustStoreEnvGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous_component.take() {
                // FIXME: Audit that the environment access only happens in single-threaded code.
                unsafe {
                    std::env::set_var(
                        magnetar_runtime::TACHYON_COMPONENT_TRUST_STORE_ENV,
                        previous,
                    )
                };
            } else {
                // FIXME: Audit that the environment access only happens in single-threaded code.
                unsafe {
                    std::env::remove_var(magnetar_runtime::TACHYON_COMPONENT_TRUST_STORE_ENV)
                };
            }
            if let Some(previous) = self.previous_model.take() {
                // FIXME: Audit that the environment access only happens in single-threaded code.
                unsafe {
                    std::env::set_var(magnetar_runtime::TACHYON_ARTIFACT_TRUST_STORE_ENV, previous)
                };
            } else {
                // FIXME: Audit that the environment access only happens in single-threaded code.
                unsafe { std::env::remove_var(magnetar_runtime::TACHYON_ARTIFACT_TRUST_STORE_ENV) };
            }
            let _ = std::fs::remove_file(&self.component_path);
            let _ = std::fs::remove_file(&self.model_path);
        }
    }

    impl Drop for TrustStoreEnvUnsetGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous_component.take() {
                // FIXME: Audit that the environment access only happens in single-threaded code.
                unsafe {
                    std::env::set_var(
                        magnetar_runtime::TACHYON_COMPONENT_TRUST_STORE_ENV,
                        previous,
                    )
                };
            }
            if let Some(previous) = self.previous_model.take() {
                // FIXME: Audit that the environment access only happens in single-threaded code.
                unsafe {
                    std::env::set_var(magnetar_runtime::TACHYON_ARTIFACT_TRUST_STORE_ENV, previous)
                };
            }
        }
    }

    fn qwen_interprocess_lock() -> QwenInterprocessGuard {
        let path = std::env::temp_dir().join("tachyon-magnetar-qwen-tests.lock");
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return QwenInterprocessGuard { path },
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::PermissionDenied
                    ) =>
                {
                    let stale = std::fs::metadata(&path)
                        .and_then(|metadata| metadata.modified())
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|age| age > std::time::Duration::from_secs(300));
                    if stale {
                        let _ = std::fs::remove_file(&path);
                    } else {
                        std::thread::sleep(std::time::Duration::from_millis(25));
                    }
                }
                Err(error) => panic!(
                    "failed to acquire Qwen test lock `{}`: {error}",
                    path.display()
                ),
            }
        }
    }

    fn trust_env_lock() -> std::sync::MutexGuard<'static, ()> {
        TRUST_ENV_LOCK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn without_tachyon_component_trust_store() -> TrustStoreEnvUnsetGuard {
        let qwen_lock = qwen_interprocess_lock();
        let lock = trust_env_lock();
        let previous_component =
            std::env::var_os(magnetar_runtime::TACHYON_COMPONENT_TRUST_STORE_ENV);
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var(magnetar_runtime::TACHYON_COMPONENT_TRUST_STORE_ENV) };
        let previous_model = std::env::var_os(magnetar_runtime::TACHYON_ARTIFACT_TRUST_STORE_ENV);
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var(magnetar_runtime::TACHYON_ARTIFACT_TRUST_STORE_ENV) };
        TrustStoreEnvUnsetGuard {
            _qwen_lock: qwen_lock,
            _lock: lock,
            previous_component,
            previous_model,
        }
    }

    /// The real content digest of the single `*.component.wasm` under
    /// `root`, computed the same way production code's `register_inference_
    /// component_artifact` does (`ComponentDigest::sha256` over the raw
    /// bytes) — never a value the fixture invents.
    fn component_wasm_digest(root: &Path) -> String {
        let component_path = std::fs::read_dir(root)
            .expect("fixture root should be readable")
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(".component.wasm"))
            })
            .expect("fixture root should contain a *.component.wasm artifact");
        let component_bytes =
            std::fs::read(&component_path).expect("component artifact should be readable");
        // `::magnetar_runtime` (leading `::`), not `magnetar_runtime` — this
        // module's own `mod magnetar_runtime;` submodule shadows the crate
        // of the same name.
        ::magnetar_runtime::ComponentDigest::sha256(&component_bytes).value
    }

    /// Trusts both the Model Artifact (weights/tokenizer/config bundle) and
    /// the Component (WASM) binary under `path` — two independent trust
    /// decisions since Magnetar's `ArtifactTrustPolicy` split them
    /// (Tachyon integration audit MAG-03); conflating them here would mean
    /// this fixture stops proving what most tests actually need, which is
    /// that a normal, fully-trusted load succeeds.
    fn trust_tachyon_component_artifact(path: &Path) -> TrustStoreEnvGuard {
        let qwen_lock = qwen_interprocess_lock();
        let lock = trust_env_lock();
        let model_digest = magnetar_inference_component::local_bundle_manifest_digest(path)
            .expect("fixture should ingest before writing Tachyon trust policy");
        let component_digest = component_wasm_digest(path);

        let trust_dir = unique_component_dir("component-trust-policy");
        std::fs::create_dir_all(&trust_dir).expect("trust policy dir should be created");

        let component_path = trust_dir.join("tachyon-component-trust.json");
        fs::write(
            &component_path,
            format!(r#"{{"trusted_digests":["{component_digest}"]}}"#),
        )
        .expect("component trust sidecar should be written");

        let model_path = trust_dir.join("tachyon-artifact-trust.json");
        fs::write(
            &model_path,
            format!(r#"{{"trusted_digests":["{model_digest}"]}}"#),
        )
        .expect("model trust sidecar should be written");

        let previous_component =
            std::env::var_os(magnetar_runtime::TACHYON_COMPONENT_TRUST_STORE_ENV);
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe {
            std::env::set_var(
                magnetar_runtime::TACHYON_COMPONENT_TRUST_STORE_ENV,
                &component_path,
            )
        };
        let previous_model = std::env::var_os(magnetar_runtime::TACHYON_ARTIFACT_TRUST_STORE_ENV);
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe {
            std::env::set_var(
                magnetar_runtime::TACHYON_ARTIFACT_TRUST_STORE_ENV,
                &model_path,
            )
        };

        TrustStoreEnvGuard {
            _qwen_lock: qwen_lock,
            _lock: lock,
            previous_component,
            previous_model,
            component_path,
            model_path,
        }
    }

    /// [`trust_tachyon_component_artifact`] generalized to more than one
    /// bundle root, trusting every root's Component (WASM) digest and Model
    /// Artifact digest in one policy. Needed when a single route binds more
    /// than one real Magnetar Component and each must be authorized
    /// independently (Tachyon integration audit MAG-03).
    fn trust_tachyon_component_artifacts(roots: &[&Path]) -> TrustStoreEnvGuard {
        let qwen_lock = qwen_interprocess_lock();
        let lock = trust_env_lock();

        let component_digests = roots
            .iter()
            .map(|root| component_wasm_digest(root))
            .collect::<Vec<_>>();
        let model_digests = roots
            .iter()
            .map(|root| {
                magnetar_inference_component::local_bundle_manifest_digest(root)
                    .expect("fixture should ingest before writing Tachyon trust policy")
            })
            .collect::<Vec<_>>();

        let trust_dir = unique_component_dir("component-trust-policy-multi");
        std::fs::create_dir_all(&trust_dir).expect("trust policy dir should be created");

        let component_path = trust_dir.join("tachyon-component-trust.json");
        fs::write(
            &component_path,
            serde_json::json!({ "trusted_digests": component_digests }).to_string(),
        )
        .expect("component trust sidecar should be written");

        let model_path = trust_dir.join("tachyon-artifact-trust.json");
        fs::write(
            &model_path,
            serde_json::json!({ "trusted_digests": model_digests }).to_string(),
        )
        .expect("model trust sidecar should be written");

        let previous_component =
            std::env::var_os(magnetar_runtime::TACHYON_COMPONENT_TRUST_STORE_ENV);
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe {
            std::env::set_var(
                magnetar_runtime::TACHYON_COMPONENT_TRUST_STORE_ENV,
                &component_path,
            )
        };
        let previous_model = std::env::var_os(magnetar_runtime::TACHYON_ARTIFACT_TRUST_STORE_ENV);
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe {
            std::env::set_var(
                magnetar_runtime::TACHYON_ARTIFACT_TRUST_STORE_ENV,
                &model_path,
            )
        };

        TrustStoreEnvGuard {
            _qwen_lock: qwen_lock,
            _lock: lock,
            previous_component,
            previous_model,
            component_path,
            model_path,
        }
    }

    fn error_chain_contains(error: &anyhow::Error, expected: &str) -> bool {
        error
            .chain()
            .any(|cause| cause.to_string().contains(expected))
    }

    #[test]
    fn magnetar_qwen_binding_generates_through_real_production_cpu_path() {
        let component_dir = unique_component_dir("qwen-runtime");
        write_tiny_production_qwen_bundle(&component_dir);
        let _trust = trust_tachyon_component_artifact(&component_dir);
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.inference_components = vec![IntegrityInferenceComponentBinding {
            alias: "qwen2_5".to_owned(),
            path: format!("magnetar:{}", component_dir.display()),
            device: ComponentPlacement::Cpu,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        }];

        let runtime = AiInferenceRuntime::from_config(&IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        })
        .expect("real Magnetar production Qwen bundle should load");
        let generation = runtime
            .compute_component_prompt("qwen2_5", br#"{"prompt":"hi","max_new_tokens":1}"#)
            .expect("real Magnetar production Qwen should generate");

        assert!(!generation.is_empty());
        let _ = std::fs::remove_dir_all(component_dir);
    }

    #[test]
    fn magnetar_qwen_reuses_resident_model_instance_across_requests() {
        let component_dir = unique_component_dir("qwen-resident-runtime");
        write_tiny_production_qwen_bundle(&component_dir);
        let _trust = trust_tachyon_component_artifact(&component_dir);
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.inference_components = vec![IntegrityInferenceComponentBinding {
            alias: "qwen_resident".to_owned(),
            path: format!("magnetar:{}", component_dir.display()),
            device: ComponentPlacement::Cpu,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        }];

        let runtime = AiInferenceRuntime::from_config(&IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        })
        .expect("real Magnetar production Qwen bundle should load");
        let before = runtime
            .magnetar_resident_debug("qwen_resident")
            .expect("resident model debug should be available");
        runtime
            .compute_component_prompt("qwen_resident", br#"{"prompt":"hi","max_new_tokens":1}"#)
            .expect("first generation should succeed");
        runtime
            .compute_component_prompt("qwen_resident", br#"{"prompt":"hi","max_new_tokens":1}"#)
            .expect("second generation should reuse resident model");
        let after = runtime
            .magnetar_resident_debug("qwen_resident")
            .expect("resident model debug should still be available");

        assert_eq!(before.0, after.0, "ModelInstance identity must be stable");
        assert_eq!(after.1, 1, "weights must be materialized once");
        let _ = std::fs::remove_dir_all(component_dir);
    }

    #[test]
    fn magnetar_qwen_concurrent_requests_share_one_resident_model() {
        let component_dir = unique_component_dir("qwen-resident-concurrent-runtime");
        write_tiny_production_qwen_bundle(&component_dir);
        let _trust = trust_tachyon_component_artifact(&component_dir);
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.inference_components = vec![IntegrityInferenceComponentBinding {
            alias: "qwen_concurrent".to_owned(),
            path: format!("magnetar:{}", component_dir.display()),
            device: ComponentPlacement::Cpu,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        }];

        let runtime = Arc::new(
            AiInferenceRuntime::from_config(&IntegrityConfig {
                routes: vec![route],
                ..IntegrityConfig::default_sealed()
            })
            .expect("real Magnetar production Qwen bundle should load"),
        );
        let before = runtime
            .magnetar_resident_debug("qwen_concurrent")
            .expect("resident model debug should be available");
        let barrier = Arc::new(std::sync::Barrier::new(4));
        let handles = (0..4)
            .map(|_| {
                let runtime = Arc::clone(&runtime);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    runtime
                        .compute_component_prompt(
                            "qwen_concurrent",
                            br#"{"prompt":"hi","max_new_tokens":1}"#,
                        )
                        .expect("concurrent generation should reuse resident model")
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            assert!(!handle.join().expect("thread should not panic").is_empty());
        }
        let after = runtime
            .magnetar_resident_debug("qwen_concurrent")
            .expect("resident model debug should still be available");

        assert_eq!(before.0, after.0, "ModelInstance identity must be stable");
        assert_eq!(
            after.1, 1,
            "concurrent requests must not rematerialize weights"
        );
        let _ = std::fs::remove_dir_all(component_dir);
    }

    #[test]
    fn magnetar_qwen_binding_maps_openai_messages_to_chat_prompt_input() {
        let component_dir = unique_component_dir("qwen-chat-runtime");
        write_tiny_production_qwen_bundle(&component_dir);
        let _trust = trust_tachyon_component_artifact(&component_dir);
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.inference_components = vec![IntegrityInferenceComponentBinding {
            alias: "qwen2_5".to_owned(),
            path: format!("magnetar:{}", component_dir.display()),
            device: ComponentPlacement::Cpu,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        }];

        let runtime = AiInferenceRuntime::from_config(&IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        })
        .expect("real Magnetar production Qwen bundle should load");
        let generation = runtime
            .compute_component_prompt(
                "qwen2_5",
                br#"{"messages":[{"role":"user","content":"hi"}],"temperature":0,"max_new_tokens":1}"#,
            )
            .expect("OpenAI chat messages should reach Magnetar PromptInput::ChatMessages");

        assert!(!generation.is_empty());
        let _ = std::fs::remove_dir_all(component_dir);
    }

    #[test]
    fn magnetar_streaming_stops_when_downstream_sink_disconnects() {
        let component_dir = unique_component_dir("qwen-stream-cancel");
        write_tiny_production_qwen_bundle(&component_dir);
        let _trust = trust_tachyon_component_artifact(&component_dir);
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.inference_components = vec![IntegrityInferenceComponentBinding {
            alias: "qwen-stream".to_owned(),
            path: format!("magnetar:{}", component_dir.display()),
            device: ComponentPlacement::Cpu,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        }];

        let runtime = AiInferenceRuntime::from_config(&IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        })
        .expect("real Magnetar production Qwen bundle should load");

        struct DisconnectingSink {
            content_events: u32,
        }
        impl StreamSink for DisconnectingSink {
            fn emit(&mut self, event: StreamEvent<'_>) -> StreamControl {
                if let StreamEvent::Payload(_) = event {
                    self.content_events += 1;
                }
                StreamControl::Stop
            }

            fn is_live(&mut self) -> bool {
                self.content_events == 0
            }
        }
        let mut sink = DisconnectingSink { content_events: 0 };

        let outcome = runtime
            .stream_loaded_component(
                "qwen-stream",
                br#"{"prompt":"hi","max_new_tokens":16}"#,
                &mut sink,
            )
            .expect("downstream stop should cancel Magnetar streaming cleanly");

        assert_eq!(
            sink.content_events, 1,
            "Tachyon must stop relaying after the downstream stream disconnects"
        );
        assert!(
            metadata_u32(&outcome.metadata, "generated_tokens")
                .map(|completion_tokens| completion_tokens <= 1)
                .unwrap_or(true),
            "cancelled Magnetar stream must not continue to produce the full request"
        );
        let _ = std::fs::remove_dir_all(component_dir);
    }

    #[test]
    fn explicit_magnetar_binding_rejects_non_qwen_model_directory() {
        let _trust = without_tachyon_component_trust_store();
        let component_dir = unique_component_dir("non-qwen");
        write_tiny_production_qwen_bundle(&component_dir);
        std::fs::write(
            component_dir.join("config.json"),
            br#"{"model_type":"llama"}"#,
        )
        .expect("config should be written");
        let error = match load_binding(&IntegrityInferenceComponentBinding {
            alias: "llama".to_owned(),
            path: format!("magnetar:{}", component_dir.display()),
            device: ComponentPlacement::Cpu,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        }) {
            Ok(_) => panic!("Magnetar cutover accepts only Qwen directories"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("Magnetar failed"));
        let _ = std::fs::remove_dir_all(component_dir);
    }

    #[test]
    fn explicit_magnetar_binding_rejects_untrusted_qwen_bundle() {
        // Neither trust store is configured, so this exercises the
        // Component (WASM) trust gate — checked first, before any model
        // ingestion (Tachyon integration audit MAG-03) — not the Model
        // Artifact one specifically; see
        // `qwen_bundle_cannot_self_authorize_with_embedded_trust_policy`
        // for a test that isolates the Model Artifact half.
        let _trust = without_tachyon_component_trust_store();
        let component_dir = unique_component_dir("untrusted-qwen");
        write_tiny_production_qwen_bundle(&component_dir);
        let error = match load_binding(&IntegrityInferenceComponentBinding {
            alias: "qwen-untrusted".to_owned(),
            path: format!("magnetar:{}", component_dir.display()),
            device: ComponentPlacement::Cpu,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        }) {
            Ok(_) => panic!("Magnetar Qwen bundle must require explicit Tachyon trust policy"),
            Err(error) => error,
        };

        assert!(
            error_chain_contains(&error, "no trust policy matched"),
            "unexpected trust rejection error: {error:#}"
        );
        let _ = std::fs::remove_dir_all(component_dir);
    }

    #[test]
    fn explicit_magnetar_binding_rejects_a_trusted_component_with_an_untrusted_model() {
        // The two trust decisions are independent (Tachyon integration
        // audit MAG-03): a Component binary the operator has explicitly
        // trusted must still not authorize loading arbitrary, untrusted
        // model weights through it. Mirrors
        // `qwen_bundle_cannot_self_authorize_with_embedded_trust_policy`
        // (which isolates the Model Artifact half) by isolating the other
        // direction — Component trusted, Model Artifact not.
        let component_dir = unique_component_dir("component-trusted-model-untrusted");
        write_tiny_production_qwen_bundle(&component_dir);
        let _trust = trust_tachyon_component_artifact(&component_dir);
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var(magnetar_runtime::TACHYON_ARTIFACT_TRUST_STORE_ENV) };

        let error = match load_binding(&IntegrityInferenceComponentBinding {
            alias: "qwen-model-untrusted".to_owned(),
            path: format!("magnetar:{}", component_dir.display()),
            device: ComponentPlacement::Cpu,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        }) {
            Ok(_) => panic!(
                "a trusted Component must not authorize loading untrusted model weights through it"
            ),
            Err(error) => error,
        };

        assert!(
            error_chain_contains(&error, "trust rejected"),
            "unexpected trust rejection error: {error:#}"
        );
        let _ = std::fs::remove_dir_all(component_dir);
    }

    #[test]
    fn explicit_magnetar_binding_requires_external_component_artifact() {
        let _trust = without_tachyon_component_trust_store();
        let component_dir = unique_component_dir("missing-component-artifact");
        write_tiny_production_qwen_bundle(&component_dir);
        std::fs::remove_file(component_dir.join("qwen-real.component.wasm"))
            .expect("component artifact should be removable");

        let error = match load_binding(&IntegrityInferenceComponentBinding {
            alias: "component-required".to_owned(),
            path: format!("magnetar:{}", component_dir.display()),
            device: ComponentPlacement::Cpu,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        }) {
            Ok(_) => panic!("Magnetar binding must not use an implicit Component artifact"),
            Err(error) => error,
        };

        assert!(
            error
                .to_string()
                .contains("must contain an explicit `*.component.wasm` artifact"),
            "unexpected missing Component error: {error:#}"
        );
        let _ = std::fs::remove_dir_all(component_dir);
    }

    #[test]
    fn qwen_bundle_cannot_self_authorize_with_embedded_trust_policy() {
        // Component trust stays externally configured throughout (valid
        // and untouched) so the load reaches Model Artifact ingestion at
        // all; this test is specifically about the Model Artifact half of
        // trust (hence asserting "trust rejected", the Model Artifact
        // rejection message — see `magnetar_inference_component`'s
        // `ArtifactTrustPolicy`), not the Component binary half.
        let component_dir = unique_component_dir("self-trusting-qwen");
        write_tiny_production_qwen_bundle(&component_dir);
        let _trust = trust_tachyon_component_artifact(&component_dir);
        let trust_path = std::env::var_os(magnetar_runtime::TACHYON_ARTIFACT_TRUST_STORE_ENV)
            .map(PathBuf::from)
            .expect("test trust policy should be configured");
        std::fs::copy(
            &trust_path,
            component_dir.join(".tachyon-artifact-trust.json"),
        )
        .expect("embedded fake trust policy should be copied");
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var(magnetar_runtime::TACHYON_ARTIFACT_TRUST_STORE_ENV) };

        let error = match load_binding(&IntegrityInferenceComponentBinding {
            alias: "qwen-self-trusting".to_owned(),
            path: format!("magnetar:{}", component_dir.display()),
            device: ComponentPlacement::Cpu,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        }) {
            Ok(_) => panic!("artifact-local trust policy must not self-authorize a Qwen bundle"),
            Err(error) => error,
        };

        assert!(
            error_chain_contains(&error, "trust rejected"),
            "unexpected trust rejection error: {error:#}"
        );
        let _ = std::fs::remove_dir_all(component_dir);
    }

    #[test]
    fn explicit_magnetar_cuda_binding_fails_closed_when_cuda_is_unavailable() {
        if magnetar_inference_component::cuda_provider_available() {
            return;
        }
        let component_dir = unique_component_dir("cuda-qwen");
        write_tiny_production_qwen_bundle(&component_dir);
        let _trust = trust_tachyon_component_artifact(&component_dir);
        let error = match load_binding(&IntegrityInferenceComponentBinding {
            alias: "qwen-cuda".to_owned(),
            path: format!("magnetar:{}", component_dir.display()),
            device: ComponentPlacement::Cuda,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        }) {
            Ok(_) => panic!("explicit CUDA placement must not silently fall back to CPU"),
            Err(error) => error,
        };

        assert!(
            error_chain_contains(&error, "CUDA provider"),
            "unexpected CUDA placement error: {error:#}"
        );
        let _ = std::fs::remove_dir_all(component_dir);
    }

    #[cfg(feature = "magnetar-cuda")]
    #[test]
    #[ignore = "run explicitly on a GPU runner guaranteed to have CUDA available"]
    fn magnetar_cuda_provider_hardware_required_guard() {
        assert!(
            magnetar_inference_component::cuda_provider_available(),
            "GPU CI selected this test but Magnetar CUDA provider is unavailable"
        );
    }

    #[cfg(feature = "magnetar-cuda")]
    #[test]
    #[ignore = "run explicitly on a GPU runner guaranteed to have CUDA available"]
    fn magnetar_qwen_binding_generates_first_token_on_real_cuda_provider() {
        assert!(
            magnetar_inference_component::cuda_provider_available(),
            "GPU CI selected this test but Magnetar CUDA provider is unavailable"
        );
        let component_dir = unique_component_dir("cuda-qwen-runtime");
        write_tiny_production_qwen_bundle(&component_dir);
        let _trust = trust_tachyon_component_artifact(&component_dir);
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.inference_components = vec![IntegrityInferenceComponentBinding {
            alias: "qwen-cuda".to_owned(),
            path: format!("magnetar:{}", component_dir.display()),
            device: ComponentPlacement::Cuda,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        }];

        let runtime = AiInferenceRuntime::from_config(&IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        })
        .expect("real Magnetar production Qwen CUDA bundle should load");
        let generation = runtime
            .compute_component_prompt("qwen-cuda", br#"{"prompt":"hi","max_new_tokens":1}"#)
            .expect("real Magnetar production Qwen should generate one token on CUDA");

        assert!(!generation.is_empty());
        let telemetry = inference_execution_telemetry();
        assert!(
            telemetry.iter().any(|event| event.alias == "qwen-cuda"
                && event.succeeded
                && event.executed_on.to_ascii_lowercase().contains("gpu")),
            "CUDA generation must record execution on the generic GPU accelerator class"
        );
        let _ = std::fs::remove_dir_all(component_dir);
    }

    #[cfg(feature = "magnetar-cuda")]
    #[test]
    #[ignore = "run explicitly on a GPU runner guaranteed to have CUDA available"]
    fn magnetar_qwen_cuda_generates_multiple_tokens_on_real_cuda_provider() {
        assert!(
            magnetar_inference_component::cuda_provider_available(),
            "GPU CI selected this test but Magnetar CUDA provider is unavailable"
        );
        let component_dir = unique_component_dir("cuda-qwen-multitoken");
        write_tiny_production_qwen_bundle(&component_dir);
        let _trust = trust_tachyon_component_artifact(&component_dir);
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.inference_components = vec![IntegrityInferenceComponentBinding {
            alias: "qwen-cuda".to_owned(),
            path: format!("magnetar:{}", component_dir.display()),
            device: ComponentPlacement::Cuda,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        }];

        let runtime = AiInferenceRuntime::from_config(&IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        })
        .expect("real Magnetar production Qwen CUDA bundle should load");
        let generation = runtime
            .invoke_loaded_component("qwen-cuda", br#"{"prompt":"hi","max_new_tokens":16}"#)
            .expect("real Magnetar CUDA should generate multiple tokens device-resident");

        assert!(!generation.payload.is_empty());
        assert_eq!(
            metadata_u32(&generation.metadata, "generated_tokens"),
            Some(16),
            "CUDA multi-token proof must generate exactly the requested token budget"
        );
        let telemetry = inference_execution_telemetry();
        assert!(
            telemetry.iter().any(|event| event.alias == "qwen-cuda"
                && event.succeeded
                && event.executed_on.to_ascii_lowercase().contains("gpu")),
            "CUDA multi-token generation must record execution on the generic GPU accelerator class"
        );
        let _ = std::fs::remove_dir_all(component_dir);
    }

    #[test]
    fn local_non_qwen_huggingface_style_directory_is_rejected() {
        let _trust = without_tachyon_component_trust_store();
        let component_dir = unique_component_dir("non-qwen-hf");
        std::fs::create_dir_all(&component_dir).expect("fixture dir should be created");
        std::fs::write(
            component_dir.join("config.json"),
            br#"{"model_type":"llama"}"#,
        )
        .expect("config should be written");

        let error = match AiInferenceRuntime::from_config(&IntegrityConfig {
            routes: vec![{
                let mut route = IntegrityRoute::user("/api/guest-ai");
                route.inference_components = vec![IntegrityInferenceComponentBinding {
                    alias: "llama".to_owned(),
                    path: component_dir.to_string_lossy().into_owned(),
                    device: ComponentPlacement::Cpu,
                    qos: RouteQos::Standard,
                    dynamic: false,
                    hardware_strategy: Default::default(),
                }];
                route
            }],
            ..IntegrityConfig::default_sealed()
        }) {
            Ok(_) => panic!("non-Qwen Hugging Face-style directories must not load"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("unsupported AI binding"));
        let _ = std::fs::remove_dir_all(component_dir);
    }

    #[test]
    fn local_inference_route_rejects_legacy_adapter_binding() {
        let error = match AiInferenceRuntime::from_config(&IntegrityConfig {
            routes: vec![{
                let mut route = IntegrityRoute::user("/api/guest-ai");
                route.artifact_id = Some("artifact-a".to_owned());
                route.inference_components = vec![IntegrityInferenceComponentBinding {
                    alias: "mock-model".to_owned(),
                    path: "mock".to_owned(),
                    device: ComponentPlacement::Cpu,
                    qos: RouteQos::Standard,
                    dynamic: false,
                    hardware_strategy: Default::default(),
                }];
                route
            }],
            ..IntegrityConfig::default_sealed()
        }) {
            Ok(_) => panic!("legacy adapter bindings must not enter local inference runtime"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("adapter"));
    }

    #[test]
    fn dynamic_openai_placeholders_do_not_collide_as_upstream_credentials() {
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.inference_components = vec![
            IntegrityInferenceComponentBinding {
                alias: "vendor-a".to_owned(),
                path: "openai:http://placeholder.invalid/v1".to_owned(),
                device: ComponentPlacement::Cpu,
                qos: RouteQos::Standard,
                dynamic: true,
                hardware_strategy: Default::default(),
            },
            IntegrityInferenceComponentBinding {
                alias: "vendor_a".to_owned(),
                path: "openai:http://placeholder.invalid/v1".to_owned(),
                device: ComponentPlacement::Cpu,
                qos: RouteQos::Standard,
                dynamic: true,
                hardware_strategy: Default::default(),
            },
        ];

        AiInferenceRuntime::from_config(&IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        })
        .expect("dynamic placeholders should not be validated as upstream credentials");
    }

    #[test]
    fn dynamic_cpu_request_loads_and_executes_reference_cpu() {
        let root = unique_component_dir("dynamic-cpu-models");
        let component_dir = root.join("dynamic-cpu-qwen");
        write_tiny_production_qwen_bundle(&component_dir);
        let _trust = trust_tachyon_component_artifact(&component_dir);
        let runtime = AiInferenceRuntime::from_config(&IntegrityConfig::default_sealed())
            .expect("runtime")
            .with_dynamic_components_root(Some(root.clone()));

        runtime
            .load_inference_component("dynamic-cpu-qwen", AcceleratorKind::Cpu)
            .expect("dynamic CPU model should load through Reference CPU");
        let generation = runtime
            .compute_component_prompt("dynamic-cpu-qwen", br#"{"prompt":"hi","max_new_tokens":1}"#)
            .expect("dynamic CPU model should generate through Magnetar Reference CPU");

        assert!(!generation.is_empty());
        let telemetry = inference_execution_telemetry();
        assert!(
            telemetry
                .iter()
                .any(|event| event.alias == "dynamic-cpu-qwen"
                    && event.succeeded
                    && event.executed_on.to_ascii_lowercase().contains("cpu")),
            "dynamic CPU generation must record execution on the generic CPU accelerator class"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn requested_gpu_rejects_cpu_loaded_model() {
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.inference_components = vec![IntegrityInferenceComponentBinding {
            alias: "cpu-only".to_owned(),
            path: "mock".to_owned(),
            device: ComponentPlacement::Cpu,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        }];
        let runtime = AiInferenceRuntime::from_config(&IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        })
        .expect("runtime");

        let error = runtime
            .load_inference_component("cpu-only", AcceleratorKind::Gpu)
            .expect_err("GPU request must not be satisfied by a CPU-loaded model");

        assert!(
            error.contains("loaded for `cpu` but `gpu` was requested"),
            "unexpected accelerator mismatch error: {error}"
        );
    }

    #[test]
    fn dynamic_gpu_request_does_not_lazy_load_cpu_when_cuda_is_unavailable() {
        if magnetar_inference_component::cuda_provider_available() {
            return;
        }
        let root = unique_component_dir("dynamic-models");
        let component_dir = root.join("dynamic-qwen");
        write_tiny_production_qwen_bundle(&component_dir);
        let _trust = trust_tachyon_component_artifact(&component_dir);
        let runtime = AiInferenceRuntime::from_config(&IntegrityConfig::default_sealed())
            .expect("runtime")
            .with_dynamic_components_root(Some(root.clone()));

        let error = runtime
            .load_inference_component("dynamic-qwen", AcceleratorKind::Gpu)
            .expect_err("dynamic GPU load must fail closed when CUDA placement is unavailable");

        assert!(
            error.contains("CUDA provider"),
            "unexpected dynamic CUDA error: {error}"
        );
        assert!(
            !runtime
                .loaded_component_aliases()
                .iter()
                .any(|alias| alias == "dynamic-qwen"),
            "failed dynamic GPU load must not leave a CPU-loaded model behind"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn loaded_gpu_model_does_not_make_dynamic_gpu_request_fall_back_to_cpu() {
        if magnetar_inference_component::cuda_provider_available() {
            return;
        }
        let root = unique_component_dir("dynamic-models-with-gpu");
        let component_dir = root.join("dynamic-qwen-b");
        write_tiny_production_qwen_bundle(&component_dir);
        let _trust = trust_tachyon_component_artifact(&component_dir);
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.inference_components = vec![IntegrityInferenceComponentBinding {
            alias: "gpu-a".to_owned(),
            path: "mock".to_owned(),
            device: ComponentPlacement::Cuda,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        }];
        let runtime = AiInferenceRuntime::from_config(&IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        })
        .expect("runtime")
        .with_dynamic_components_root(Some(root.clone()));
        assert!(runtime.supports_accelerator(AcceleratorKind::Gpu));

        let error = runtime
            .load_inference_component("dynamic-qwen-b", AcceleratorKind::Gpu)
            .expect_err(
                "dynamic GPU load must not reuse CPU just because another GPU model exists",
            );

        assert!(
            error.contains("CUDA provider"),
            "unexpected dynamic CUDA error: {error}"
        );
        assert!(
            !runtime
                .loaded_component_aliases()
                .iter()
                .any(|alias| alias == "dynamic-qwen-b"),
            "failed dynamic GPU load must not leave a CPU-loaded model behind"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn queue_tier_snapshot_tracks_active_local_execution_depths() {
        let runtime =
            AiInferenceRuntime::from_config(&IntegrityConfig::default_sealed()).expect("runtime");

        {
            let _guard = runtime.track_queue_depth(AcceleratorKind::Gpu, RouteQos::RealTime);
            assert_eq!(
                runtime.queue_tier_snapshot(AcceleratorKind::Gpu),
                QueueTierSnapshot {
                    realtime: 1,
                    standard: 0,
                    batch: 0,
                }
            );
        }

        assert_eq!(
            runtime.queue_tier_snapshot(AcceleratorKind::Gpu),
            QueueTierSnapshot::default()
        );
    }

    #[test]
    fn accelerator_support_is_derived_from_loaded_components() {
        let runtime =
            AiInferenceRuntime::from_config(&IntegrityConfig::default_sealed()).expect("runtime");

        assert!(runtime.supports_accelerator(AcceleratorKind::Cpu));
        assert!(!runtime.supports_accelerator(AcceleratorKind::Gpu));
        assert!(!runtime.supports_accelerator(AcceleratorKind::Npu));
        assert!(!runtime.supports_accelerator(AcceleratorKind::Tpu));
    }

    /// Proves alias-based dispatch on one route: two differently-aliased
    /// `mock:` handles resolve and invoke independently. Does not prove two
    /// *structurally* distinct Components load side by side — `mock:`
    /// bindings never reach `MagnetarRuntime::try_load` at all, so both
    /// aliases here answer with the same canned `MOCK_INFERENCE_RESPONSE`.
    /// See `one_route_loads_two_structurally_distinct_magnetar_components`
    /// for that proof through the real Magnetar path.
    #[test]
    fn one_route_dispatches_two_aliases_to_independent_mock_handles() {
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.inference_components = vec![
            IntegrityInferenceComponentBinding {
                alias: "component-a".to_owned(),
                path: "mock:component-a".to_owned(),
                device: ComponentPlacement::Cpu,
                qos: RouteQos::Standard,
                dynamic: false,
                hardware_strategy: Default::default(),
            },
            IntegrityInferenceComponentBinding {
                alias: "component-b".to_owned(),
                path: "mock:component-b".to_owned(),
                device: ComponentPlacement::Cpu,
                qos: RouteQos::Batch,
                dynamic: false,
                hardware_strategy: Default::default(),
            },
        ];
        let runtime = AiInferenceRuntime::from_config(&IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        })
        .expect("two Components on the same route should load");

        assert_eq!(
            runtime.loaded_component_aliases(),
            vec!["component-a".to_owned(), "component-b".to_owned()]
        );
        assert_eq!(
            runtime
                .compute_component_prompt("component-a", b"ping")
                .expect("component-a should invoke"),
            MOCK_INFERENCE_RESPONSE
        );
        assert_eq!(
            runtime
                .compute_component_prompt("component-b", b"ping")
                .expect("component-b should invoke"),
            MOCK_INFERENCE_RESPONSE
        );
    }

    /// The heterogeneity proof `one_route_dispatches_two_aliases_to_
    /// independent_mock_handles` cannot give: two *structurally* distinct,
    /// independently-compiled Component binaries — Magnetar's own checked-in
    /// Qwen and Llama fixtures, the same pair its own
    /// `loaded_inference_component_load_runs_a_real_second_architecture_
    /// end_to_end` test proves at the crate level — loaded side by side on
    /// one route through Tachyon's real `MagnetarRuntime::try_load` path,
    /// each invoked through its own resident instance.
    #[test]
    fn one_route_loads_two_structurally_distinct_magnetar_components() {
        let qwen_dir = unique_component_dir("two-components-qwen");
        let llama_dir = unique_component_dir("two-components-llama");
        write_tiny_production_qwen_bundle(&qwen_dir);
        write_tiny_production_llama_bundle(&llama_dir);
        let _trust = trust_tachyon_component_artifacts(&[&qwen_dir, &llama_dir]);

        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.inference_components = vec![
            IntegrityInferenceComponentBinding {
                alias: "qwen-real".to_owned(),
                path: format!("magnetar:{}", qwen_dir.display()),
                device: ComponentPlacement::Cpu,
                qos: RouteQos::Standard,
                dynamic: false,
                hardware_strategy: Default::default(),
            },
            IntegrityInferenceComponentBinding {
                alias: "llama-real".to_owned(),
                path: format!("magnetar:{}", llama_dir.display()),
                device: ComponentPlacement::Cpu,
                qos: RouteQos::Standard,
                dynamic: false,
                hardware_strategy: Default::default(),
            },
        ];

        let runtime = AiInferenceRuntime::from_config(&IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        })
        .expect(
            "two structurally distinct, independently-compiled real Magnetar Components \
             should both load on one route",
        );

        assert_eq!(
            runtime.loaded_component_aliases(),
            vec!["llama-real".to_owned(), "qwen-real".to_owned()]
        );

        let qwen_output = runtime
            .compute_component_prompt(
                "qwen-real",
                br#"{"prompt":"hello world","max_new_tokens":1}"#,
            )
            .expect("the Qwen Component should generate through its own resident instance");
        let llama_output = runtime
            .compute_component_prompt(
                "llama-real",
                br#"{"prompt":"hello world","max_new_tokens":1}"#,
            )
            .expect("the Llama Component should generate through its own resident instance");
        assert!(!qwen_output.is_empty());
        assert!(!llama_output.is_empty());

        // The fixture pair really is two independently-compiled binaries,
        // not two copies of the same one — the precondition the rest of this
        // test's proof rests on.
        assert_ne!(
            component_wasm_digest(&qwen_dir),
            component_wasm_digest(&llama_dir),
            "the fixture pair must be independently-compiled, digest-distinct Component binaries"
        );
        // Both loads succeeded under a policy that trusted each digest
        // independently (`trust_tachyon_component_artifacts`): had the
        // runtime quietly deduplicated one binding onto the other's already-
        // loaded Component instead of running `MagnetarRuntime::try_load`
        // for each, the alias reusing a foreign binary would carry a digest
        // neither its own trust entry nor a coincidence could satisfy. Each
        // alias's own resident instance also reports a fresh, single
        // materialization — a shared or reused instance would not.
        let (_, qwen_materializations) = runtime
            .magnetar_resident_debug("qwen-real")
            .expect("Qwen resident debug should be available");
        let (_, llama_materializations) = runtime
            .magnetar_resident_debug("llama-real")
            .expect("Llama resident debug should be available");
        assert_eq!(
            qwen_materializations, 1,
            "the Qwen Component must materialize its own weights exactly once"
        );
        assert_eq!(
            llama_materializations, 1,
            "the Llama Component must materialize its own weights exactly once"
        );

        let telemetry = inference_execution_telemetry();
        assert!(
            telemetry
                .iter()
                .any(|event| event.alias == "qwen-real" && event.succeeded),
            "the Qwen Component's own invocation must be recorded"
        );
        assert!(
            telemetry
                .iter()
                .any(|event| event.alias == "llama-real" && event.succeeded),
            "the Llama Component's own invocation must be recorded"
        );

        let _ = std::fs::remove_dir_all(qwen_dir);
        let _ = std::fs::remove_dir_all(llama_dir);
    }
}
