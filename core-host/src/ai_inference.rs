#[path = "ai_inference/magnetar_runtime.rs"]
mod magnetar_runtime;
#[path = "ai_inference/upstream_openai.rs"]
mod upstream_openai;

use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, OnceLock, RwLock},
    time::Duration,
};
use wasmtime_wasi_nn::{
    witx::WasiNnCtx, Graph as WasiGraph, GraphRegistry, Registry as WasiRegistry,
};

use crate::{IntegrityConfig, IntegrityModelBinding, RouteQos};

pub(crate) const UPSTREAM_SCHEME: &str = "openai:";
pub(crate) use magnetar_runtime::MAGNETAR_PATH_PREFIX;
const MODEL_META_JSON: &str = ".tachyon-model.json";
const MOCK_INFERENCE_RESPONSE: &str = "MOCK_LLM_RESPONSE";
const UPSTREAM_ADMISSION_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) fn binding_runs_upstream(binding: &IntegrityModelBinding) -> bool {
    !binding.dynamic && binding.path.trim().starts_with(UPSTREAM_SCHEME)
}

pub(crate) fn upstream_max_concurrency() -> usize {
    std::env::var("TACHYON_UPSTREAM_MAX_CONCURRENCY")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(32)
}

static HOST_UPSTREAM_ADMISSION: OnceLock<Arc<UpstreamAdmission>> = OnceLock::new();

fn host_upstream_admission() -> Arc<UpstreamAdmission> {
    Arc::clone(HOST_UPSTREAM_ADMISSION.get_or_init(|| Arc::new(UpstreamAdmission::from_env())))
}

struct UpstreamAdmission {
    state: Mutex<UpstreamAdmissionState>,
    released: Condvar,
    capacity: usize,
    max_waiters: usize,
}

#[derive(Default)]
struct UpstreamAdmissionState {
    in_flight: usize,
    waiting: usize,
}

#[derive(Debug)]
enum UpstreamAdmissionError {
    QueueFull { waiting: usize, limit: usize },
    TimedOut { in_flight: usize, limit: usize },
}

impl std::fmt::Display for UpstreamAdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::QueueFull { waiting, limit } => write!(
                f,
                "upstream request queue is full ({waiting} waiting, limit {limit})"
            ),
            Self::TimedOut { in_flight, limit } => write!(
                f,
                "upstream request queue is saturated ({in_flight} in flight, limit {limit}): retry, or raise `TACHYON_UPSTREAM_MAX_CONCURRENCY`"
            ),
        }
    }
}

impl UpstreamAdmission {
    fn new(capacity: usize) -> Self {
        Self {
            state: Mutex::new(UpstreamAdmissionState::default()),
            released: Condvar::new(),
            capacity: capacity.max(1),
            max_waiters: capacity.max(1),
        }
    }

    fn from_env() -> Self {
        Self::new(upstream_max_concurrency())
    }

    fn acquire(&self) -> std::result::Result<UpstreamPermit<'_>, UpstreamAdmissionError> {
        let mut state = self.state.lock().expect("upstream admission lock poisoned");
        if state.in_flight >= self.capacity {
            if state.waiting >= self.max_waiters {
                return Err(UpstreamAdmissionError::QueueFull {
                    waiting: state.waiting,
                    limit: self.max_waiters,
                });
            }
            state.waiting += 1;
            let deadline = std::time::Instant::now() + UPSTREAM_ADMISSION_TIMEOUT;
            loop {
                if state.in_flight < self.capacity {
                    break;
                }
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() {
                    state.waiting -= 1;
                    return Err(UpstreamAdmissionError::TimedOut {
                        in_flight: state.in_flight,
                        limit: self.capacity,
                    });
                }
                let (next, _) = self
                    .released
                    .wait_timeout(state, remaining)
                    .expect("upstream admission lock poisoned");
                state = next;
            }
            state.waiting -= 1;
        }
        state.in_flight += 1;
        Ok(UpstreamPermit { gate: self })
    }

    fn release(&self) {
        let mut state = self.state.lock().expect("upstream admission lock poisoned");
        state.in_flight = state.in_flight.saturating_sub(1);
        drop(state);
        self.released.notify_one();
    }

    fn waiting(&self) -> usize {
        self.state
            .lock()
            .expect("upstream admission lock poisoned")
            .waiting
    }

    #[cfg(test)]
    fn in_flight(&self) -> usize {
        self.state
            .lock()
            .expect("upstream admission lock poisoned")
            .in_flight
    }
}

struct UpstreamPermit<'a> {
    gate: &'a UpstreamAdmission,
}

impl Drop for UpstreamPermit<'_> {
    fn drop(&mut self) {
        self.gate.release();
    }
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

    pub(crate) fn from_model_device(device: &crate::ModelDevice) -> Self {
        match device {
            crate::ModelDevice::Cpu => Self::Cpu,
            crate::ModelDevice::Cuda | crate::ModelDevice::Metal => Self::Gpu,
            crate::ModelDevice::Npu => Self::Npu,
            crate::ModelDevice::Tpu => Self::Tpu,
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

fn record_execution(alias: impl Into<String>, executed_on: impl Into<String>, succeeded: bool) {
    let records = INFERENCE_TELEMETRY.get_or_init(|| Mutex::new(Vec::new()));
    let mut records = records.lock().expect("inference telemetry lock poisoned");
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
    INFERENCE_TELEMETRY
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .expect("inference telemetry lock poisoned")
        .clone()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TokenUsage {
    pub(crate) prompt_tokens: u32,
    pub(crate) completion_tokens: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ToolCall {
    pub(crate) id: Option<String>,
    pub(crate) name: String,
    pub(crate) arguments: String,
}

pub(crate) enum StreamEvent<'a> {
    Content(&'a str),
    Refusal(&'a str),
    ToolCall(ToolCall),
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

#[derive(Clone, Debug, Default)]
pub(crate) struct StreamOutcome {
    pub(crate) usage: Option<TokenUsage>,
    pub(crate) finish_reason: Option<String>,
}

impl StreamOutcome {
    fn usage(usage: Option<TokenUsage>) -> Self {
        Self {
            usage,
            finish_reason: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GenerationError {
    pub(crate) message: String,
    pub(crate) upstream_status: Option<u16>,
    pub(crate) class: Option<String>,
    pub(crate) invalid_request: bool,
}

impl GenerationError {
    pub(crate) fn local(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            upstream_status: None,
            class: None,
            invalid_request: false,
        }
    }

    pub(crate) fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            upstream_status: None,
            class: None,
            invalid_request: true,
        }
    }
}

impl From<upstream_openai::UpstreamError> for GenerationError {
    fn from(error: upstream_openai::UpstreamError) -> Self {
        let upstream_status = error.http_status();
        let invalid_request =
            matches!(error, upstream_openai::UpstreamError::InvalidRequest { .. });
        Self {
            message: error.to_string(),
            upstream_status,
            class: None,
            invalid_request,
        }
    }
}

impl From<UpstreamAdmissionError> for GenerationError {
    fn from(error: UpstreamAdmissionError) -> Self {
        Self {
            message: error.to_string(),
            upstream_status: Some(429),
            class: Some("upstream-admission".to_owned()),
            invalid_request: false,
        }
    }
}

impl std::fmt::Display for GenerationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<String> for GenerationError {
    fn from(message: String) -> Self {
        Self::local(message)
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ComponentGeneration {
    pub(crate) text: String,
    pub(crate) refusal: Option<String>,
    pub(crate) usage: Option<TokenUsage>,
    pub(crate) finish_reason: Option<String>,
    pub(crate) tool_calls: Vec<ToolCall>,
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

struct EmptyGraphRegistry;

impl GraphRegistry for EmptyGraphRegistry {
    fn get(&self, _name: &str) -> Option<&WasiGraph> {
        None
    }

    fn get_mut(&mut self, _name: &str) -> Option<&mut WasiGraph> {
        None
    }
}

#[cfg(test)]
struct MockPreloadedGraphRegistry {
    graphs: HashMap<String, WasiGraph>,
}

#[cfg(test)]
impl MockPreloadedGraphRegistry {
    fn from_aliases(aliases: impl IntoIterator<Item = String>) -> Self {
        use wasmtime_wasi_nn::{
            backend::{BackendError, BackendExecutionContext, BackendGraph, Id, NamedTensor},
            wit::{Tensor as WasiTensor, TensorType as WasiTensorType},
            ExecutionContext, Graph,
        };

        struct MockGraph;
        struct MockCtx;

        impl BackendGraph for MockGraph {
            fn init_execution_context(&self) -> Result<ExecutionContext, BackendError> {
                Ok(ExecutionContext::from(
                    Box::new(MockCtx) as Box<dyn BackendExecutionContext>
                ))
            }
        }

        impl BackendExecutionContext for MockCtx {
            fn set_input(&mut self, _id: Id, _tensor: &WasiTensor) -> Result<(), BackendError> {
                Ok(())
            }

            fn compute(
                &mut self,
                _named: Option<Vec<NamedTensor>>,
            ) -> Result<Option<Vec<NamedTensor>>, BackendError> {
                Ok(None)
            }

            fn get_output(&mut self, _id: Id) -> Result<WasiTensor, BackendError> {
                Ok(WasiTensor {
                    dimensions: vec![MOCK_INFERENCE_RESPONSE.len() as u32],
                    ty: WasiTensorType::U8,
                    data: MOCK_INFERENCE_RESPONSE.as_bytes().to_vec(),
                })
            }
        }

        let graphs = aliases
            .into_iter()
            .map(|alias| {
                let graph = Graph::from(Box::new(MockGraph) as Box<dyn BackendGraph>);
                (alias, graph)
            })
            .collect();
        Self { graphs }
    }
}

#[cfg(test)]
impl GraphRegistry for MockPreloadedGraphRegistry {
    fn get(&self, name: &str) -> Option<&WasiGraph> {
        self.graphs.get(name)
    }

    fn get_mut(&mut self, name: &str) -> Option<&mut WasiGraph> {
        self.graphs.get_mut(name)
    }
}

#[derive(Clone)]
enum ModelRuntime {
    Mock { accelerator: AcceleratorKind },
    Magnetar(Arc<magnetar_runtime::MagnetarRuntime>),
    Upstream(Arc<upstream_openai::UpstreamOpenAiRuntime>),
}

#[derive(Clone)]
struct LoadedModel {
    alias: String,
    qos: RouteQos,
    runtime: ModelRuntime,
}

impl LoadedModel {
    fn accelerator(&self) -> AcceleratorKind {
        match &self.runtime {
            ModelRuntime::Mock { accelerator } => *accelerator,
            ModelRuntime::Magnetar(runtime) => {
                if magnetar_provider_is_cuda(runtime.provider()) {
                    AcceleratorKind::Gpu
                } else {
                    AcceleratorKind::Cpu
                }
            }
            ModelRuntime::Upstream(_) => AcceleratorKind::Network,
        }
    }

    fn memory_residency(&self) -> AcceleratorMemoryResidency {
        match &self.runtime {
            ModelRuntime::Mock {
                accelerator: AcceleratorKind::Gpu,
            } => AcceleratorMemoryResidency::Vram,
            ModelRuntime::Mock {
                accelerator: AcceleratorKind::Npu | AcceleratorKind::Tpu,
            } => AcceleratorMemoryResidency::Sram,
            ModelRuntime::Mock { .. } | ModelRuntime::Upstream(_) => {
                AcceleratorMemoryResidency::HostRam
            }
            ModelRuntime::Magnetar(runtime) => {
                if magnetar_provider_is_cuda(runtime.provider()) {
                    AcceleratorMemoryResidency::Vram
                } else {
                    AcceleratorMemoryResidency::HostRam
                }
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct AiInferenceRuntime {
    models: Arc<RwLock<HashMap<String, LoadedModel>>>,
    dynamic_models_root: Option<PathBuf>,
    queue_snapshots: Arc<RwLock<HashMap<AcceleratorKind, QueueTierSnapshot>>>,
    upstream_admission: Arc<UpstreamAdmission>,
}

impl AiInferenceRuntime {
    pub(crate) fn from_config(config: &IntegrityConfig) -> Result<Self> {
        assert_no_credential_collisions(
            config
                .routes
                .iter()
                .flat_map(|route| route.models.iter())
                .filter(|binding| binding_runs_upstream(binding))
                .map(|binding| binding.alias.as_str()),
        )
        .map_err(|detail| anyhow!("Integrity Validation Failed: {detail}"))?;

        let mut models = HashMap::new();
        let mut sealed_aliases = HashSet::new();
        for route in &config.routes {
            for binding in &route.models {
                if !sealed_aliases.insert(binding.alias.clone()) {
                    return Err(anyhow!(
                        "Integrity Validation Failed: model alias `{}` must be globally unique",
                        binding.alias
                    ));
                }
                if binding.dynamic {
                    continue;
                }
                if binding.path.trim().is_empty() {
                    return Err(anyhow!(
                        "Integrity Validation Failed: static model alias `{}` requires a non-empty `path` (set `dynamic: true` for broker-uploaded models)",
                        binding.alias
                    ));
                }
                let model = load_binding(binding)?;
                models.insert(binding.alias.clone(), model);
            }
        }
        Ok(Self {
            models: Arc::new(RwLock::new(models)),
            dynamic_models_root: None,
            queue_snapshots: Arc::new(RwLock::new(HashMap::new())),
            upstream_admission: host_upstream_admission(),
        })
    }

    pub(crate) fn with_dynamic_models_root(mut self, root: Option<PathBuf>) -> Self {
        self.dynamic_models_root = root;
        self
    }

    fn ensure_model_loaded(&self, alias: &str) -> Result<(), String> {
        if self
            .models
            .read()
            .expect("model registry lock poisoned")
            .contains_key(alias)
        {
            return Ok(());
        }
        let Some(root) = self.dynamic_models_root.as_ref() else {
            return Err(format!("model alias `{alias}` is not loaded"));
        };
        let model_dir = root.join(alias);
        if !model_dir.is_dir() {
            return Err(format!("model alias `{alias}` is not loaded"));
        }
        let binding = IntegrityModelBinding {
            alias: alias.to_owned(),
            path: format!("magnetar:{}", model_dir.display()),
            device: crate::ModelDevice::Cpu,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        };
        let model = load_binding(&binding).map_err(|error| error.to_string())?;
        self.models
            .write()
            .expect("model registry lock poisoned")
            .entry(alias.to_owned())
            .or_insert(model);
        Ok(())
    }

    pub(crate) fn loaded_model_aliases(&self) -> Vec<String> {
        let mut aliases = self
            .models
            .read()
            .expect("model registry lock poisoned")
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        aliases.sort();
        aliases
    }

    pub(crate) fn build_wasi_nn_ctx(&self) -> WasiNnCtx {
        #[cfg(test)]
        let registry = WasiRegistry::from(MockPreloadedGraphRegistry::from_aliases(
            self.models
                .read()
                .expect("model registry lock poisoned")
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
        ));
        #[cfg(not(test))]
        let registry = WasiRegistry::from(EmptyGraphRegistry);
        WasiNnCtx::new([], registry)
    }

    pub(crate) fn supports_accelerator(&self, accelerator: AcceleratorKind) -> bool {
        matches!(accelerator, AcceleratorKind::Cpu)
            || self
                .models
                .read()
                .expect("model registry lock poisoned")
                .values()
                .any(|model| model.accelerator() == accelerator)
    }

    pub(crate) fn queue_tier_snapshot(&self, accelerator: AcceleratorKind) -> QueueTierSnapshot {
        if matches!(accelerator, AcceleratorKind::Network) {
            let waiting = self.upstream_admission.waiting().min(u32::MAX as usize) as u32;
            return QueueTierSnapshot {
                realtime: waiting,
                standard: waiting,
                batch: waiting,
            };
        }
        self.queue_snapshots
            .read()
            .expect("queue snapshots lock poisoned")
            .get(&accelerator)
            .copied()
            .unwrap_or_default()
    }

    pub(crate) fn load_component_model(
        &self,
        alias: &str,
        accelerator: AcceleratorKind,
    ) -> std::result::Result<(), String> {
        if !self.supports_accelerator(accelerator) {
            return Err(format!(
                "{} accelerator is unavailable on this host",
                accelerator.as_str()
            ));
        }
        self.ensure_model_loaded(alias)?;
        let models = self.models.read().expect("model registry lock poisoned");
        let model = models
            .get(alias)
            .ok_or_else(|| format!("model alias `{alias}` is not loaded"))?;
        if matches!(accelerator, AcceleratorKind::Npu | AcceleratorKind::Tpu)
            && model.accelerator() != accelerator
        {
            return Err(format!(
                "model alias `{alias}` requires `{}` but `{}` is requested",
                model.accelerator().as_str(),
                accelerator.as_str()
            ));
        }
        Ok(())
    }

    pub(crate) fn compute_component_prompt(
        &self,
        alias: &str,
        prompt: &str,
    ) -> std::result::Result<String, GenerationError> {
        self.compute_component_prompt_with_adapter(alias, prompt, None)
            .map(|generation| generation.text)
    }

    pub(crate) fn compute_component_prompt_with_adapter(
        &self,
        alias: &str,
        prompt: &str,
        adapter_id: Option<&str>,
    ) -> std::result::Result<ComponentGeneration, GenerationError> {
        if let Some(adapter_id) = adapter_id {
            validate_lora_adapter_id(adapter_id).map_err(GenerationError::local)?;
            return Err(GenerationError::invalid_request(format!(
                "LoRA adapter `{adapter_id}` is not supported by the Magnetar cutover"
            )));
        }
        self.ensure_model_loaded(alias)
            .map_err(GenerationError::local)?;
        let model = self
            .models
            .read()
            .expect("model registry lock poisoned")
            .get(alias)
            .cloned()
            .ok_or_else(|| {
                GenerationError::local(format!("model alias `{alias}` is not loaded"))
            })?;
        let _queue_depth = self.track_queue_depth(model.accelerator(), model.qos);
        let _upstream_permit = matches!(&model.runtime, ModelRuntime::Upstream(_))
            .then(|| self.upstream_admission.acquire())
            .transpose()?;
        let output = execute_model(&model, prompt.as_bytes())?;
        let text = String::from_utf8(output.bytes)
            .map_err(|error| GenerationError::local(error.to_string()))?;
        Ok(ComponentGeneration {
            text,
            refusal: output.refusal,
            usage: output.usage,
            finish_reason: output.finish_reason,
            tool_calls: output.tool_calls,
        })
    }

    pub(crate) fn embed_component_input(
        &self,
        alias: &str,
        input: &str,
    ) -> std::result::Result<Vec<f32>, GenerationError> {
        self.ensure_model_loaded(alias)
            .map_err(GenerationError::local)?;
        let model = self
            .models
            .read()
            .expect("model registry lock poisoned")
            .get(alias)
            .cloned()
            .ok_or_else(|| {
                GenerationError::local(format!("model alias `{alias}` is not loaded"))
            })?;
        let _queue_depth = self.track_queue_depth(model.accelerator(), model.qos);
        match &model.runtime {
            ModelRuntime::Upstream(runtime) => {
                let _permit = self.upstream_admission.acquire()?;
                runtime.embed(input).map_err(GenerationError::from)
            }
            _ => Err(GenerationError::invalid_request(format!(
                "model `{alias}` does not expose dense text embeddings in the Magnetar cutover"
            ))),
        }
    }

    pub(crate) fn stream_component_prompt(
        &self,
        alias: &str,
        prompt: &str,
        adapter_id: Option<&str>,
        sink: &mut dyn StreamSink,
    ) -> std::result::Result<StreamOutcome, GenerationError> {
        if let Some(adapter_id) = adapter_id {
            validate_lora_adapter_id(adapter_id).map_err(GenerationError::local)?;
            return Err(GenerationError::invalid_request(format!(
                "LoRA adapter `{adapter_id}` is not supported by the Magnetar cutover"
            )));
        }
        self.ensure_model_loaded(alias)
            .map_err(GenerationError::local)?;
        let model = self
            .models
            .read()
            .expect("model registry lock poisoned")
            .get(alias)
            .cloned()
            .ok_or_else(|| {
                GenerationError::local(format!("model alias `{alias}` is not loaded"))
            })?;
        let _queue_depth = self.track_queue_depth(model.accelerator(), model.qos);
        let _upstream_permit = matches!(&model.runtime, ModelRuntime::Upstream(_))
            .then(|| self.upstream_admission.acquire())
            .transpose()?;
        match &model.runtime {
            ModelRuntime::Mock { .. } => {
                if sink.is_live() {
                    sink.emit(StreamEvent::Content(MOCK_INFERENCE_RESPONSE));
                }
                record_execution(&model.alias, model.accelerator().as_str(), true);
                Ok(StreamOutcome::usage(Some(mock_token_usage(
                    prompt.as_bytes(),
                    MOCK_INFERENCE_RESPONSE,
                ))))
            }
            ModelRuntime::Magnetar(runtime) => {
                let mut emit = |fragment: &str| {
                    if sink.is_live() {
                        sink.emit(StreamEvent::Content(fragment))
                    } else {
                        StreamControl::Stop
                    }
                };
                let result = runtime
                    .generate_streaming(&[prompt.as_bytes()], &mut emit)
                    .map(|usage| StreamOutcome::usage(Some(usage)))
                    .map_err(|error| GenerationError::local(error.to_string()));
                record_execution(&model.alias, runtime.executed_on(), result.is_ok());
                result
            }
            ModelRuntime::Upstream(runtime) => {
                let result = runtime
                    .generate_streaming(&[prompt.as_bytes()], sink)
                    .map_err(GenerationError::from);
                record_execution(&model.alias, runtime.executed_on(), result.is_ok());
                result
            }
        }
    }

    pub(crate) fn magnetar_capability_advertisements(
        &self,
    ) -> Vec<magnetar_runtime::CapabilityAdvertisement> {
        let mut advertisements = self
            .models
            .read()
            .expect("model registry lock poisoned")
            .values()
            .filter_map(|model| match &model.runtime {
                ModelRuntime::Magnetar(runtime) => Some(runtime.provider().clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        advertisements.sort_by(|a, b| {
            a.provider_name
                .cmp(&b.provider_name)
                .then_with(|| a.device_ids.cmp(&b.device_ids))
        });
        advertisements
            .dedup_by(|a, b| a.provider_name == b.provider_name && a.device_ids == b.device_ids);
        advertisements
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
    pub(crate) fn model_memory_residency(&self, alias: &str) -> Option<AcceleratorMemoryResidency> {
        self.models
            .read()
            .expect("model registry lock poisoned")
            .get(alias)
            .map(LoadedModel::memory_residency)
    }

    fn track_queue_depth(&self, accelerator: AcceleratorKind, qos: RouteQos) -> QueueDepthGuard {
        if matches!(accelerator, AcceleratorKind::Network) {
            return QueueDepthGuard::noop();
        }
        {
            let mut snapshots = self
                .queue_snapshots
                .write()
                .expect("queue snapshots lock poisoned");
            adjust_queue_depth(&mut snapshots, accelerator, qos, 1);
        }
        QueueDepthGuard {
            snapshots: Some(Arc::clone(&self.queue_snapshots)),
            accelerator,
            qos,
        }
    }
}

fn magnetar_provider_is_cuda(provider: &magnetar_runtime::CapabilityAdvertisement) -> bool {
    provider.provider_name.to_ascii_lowercase().contains("cuda")
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
        let mut snapshots = snapshots.write().expect("queue snapshots lock poisoned");
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

struct ModelOutput {
    bytes: Vec<u8>,
    usage: Option<TokenUsage>,
    finish_reason: Option<String>,
    tool_calls: Vec<ToolCall>,
    refusal: Option<String>,
}

fn load_binding(binding: &IntegrityModelBinding) -> Result<LoadedModel> {
    let path = binding.path.trim();
    let runtime = if path == "mock" || path.starts_with("mock:") {
        ModelRuntime::Mock {
            accelerator: AcceleratorKind::from_model_device(&binding.device),
        }
    } else if let Some(runtime) =
        upstream_openai::UpstreamOpenAiRuntime::try_load(&binding.alias, path)?
    {
        ModelRuntime::Upstream(Arc::new(runtime))
    } else if let Some(runtime) =
        magnetar_runtime::MagnetarRuntime::try_load(&binding.alias, path, binding.device.as_str())?
    {
        ModelRuntime::Magnetar(Arc::new(runtime))
    } else if magnetar_runtime::is_magnetar_path(path) {
        return Err(anyhow!(
            "unsupported Magnetar model binding `{}` at `{}`: expected a Qwen safetensors directory",
            binding.alias,
            binding.path
        ));
    } else {
        return Err(anyhow!(
            "unsupported AI model binding `{}` at `{}`: Magnetar cutover accepts explicit mock paths, openai upstream paths, or Qwen safetensors directories",
            binding.alias,
            binding.path
        ));
    };
    Ok(LoadedModel {
        alias: binding.alias.clone(),
        qos: binding.qos,
        runtime,
    })
}

fn execute_model(
    model: &LoadedModel,
    prompt: &[u8],
) -> std::result::Result<ModelOutput, GenerationError> {
    match &model.runtime {
        ModelRuntime::Mock { .. } => {
            record_execution(&model.alias, model.accelerator().as_str(), true);
            Ok(ModelOutput {
                bytes: MOCK_INFERENCE_RESPONSE.as_bytes().to_vec(),
                usage: Some(mock_token_usage(prompt, MOCK_INFERENCE_RESPONSE)),
                finish_reason: None,
                tool_calls: Vec::new(),
                refusal: None,
            })
        }
        ModelRuntime::Magnetar(runtime) => {
            let result = runtime
                .generate(&[prompt])
                .map_err(|error| GenerationError::local(error.to_string()))
                .map(|mut outputs| {
                    let (bytes, usage) = outputs.remove(0);
                    ModelOutput {
                        bytes,
                        usage: Some(usage),
                        finish_reason: None,
                        tool_calls: Vec::new(),
                        refusal: None,
                    }
                });
            record_execution(&model.alias, runtime.executed_on(), result.is_ok());
            result
        }
        ModelRuntime::Upstream(runtime) => {
            let result = runtime
                .generate(&[prompt])
                .map(|generation| ModelOutput {
                    bytes: generation.bytes,
                    usage: generation.usage,
                    finish_reason: generation.finish_reason,
                    tool_calls: generation.tool_calls,
                    refusal: generation.refusal,
                })
                .map_err(GenerationError::from);
            record_execution(&model.alias, runtime.executed_on(), result.is_ok());
            result
        }
    }
}

pub(crate) fn detect_tool_call_parser(path: &Path) -> Option<&'static str> {
    if let Some(declared) = read_declared_tool_call_parser(path) {
        return Some(declared);
    }
    let config = std::fs::read_to_string(path.join("config.json")).ok()?;
    let normalized = config.to_ascii_lowercase();
    if normalized.contains("qwen") && normalized.contains("coder") {
        Some("qwen_coder")
    } else if normalized.contains("qwen") {
        Some("qwen")
    } else {
        None
    }
}

#[derive(Debug, Deserialize)]
struct ModelMeta {
    #[serde(default)]
    tool_call_parser: Option<String>,
}

fn read_declared_tool_call_parser(root: &Path) -> Option<&'static str> {
    let raw = std::fs::read(root.join(MODEL_META_JSON)).ok()?;
    let meta: ModelMeta = serde_json::from_slice(&raw).ok()?;
    declared_tool_call_parser(meta.tool_call_parser.as_deref()?)
}

fn declared_tool_call_parser(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "json" => Some("json"),
        "qwen" => Some("qwen"),
        "qwen_coder" => Some("qwen_coder"),
        "mistral" => Some("mistral"),
        _ => None,
    }
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

fn validate_lora_adapter_id(adapter_id: &str) -> std::result::Result<(), String> {
    if adapter_id.is_empty()
        || adapter_id.contains("..")
        || adapter_id.contains('/')
        || adapter_id.contains('\\')
        || adapter_id.ends_with(".safetensors")
    {
        return Err(format!(
            "adapter id `{adapter_id}` is not a valid identifier: use the adapter name without path separators, traversal, or extension"
        ));
    }
    Ok(())
}

fn mock_token_usage(prompt: &[u8], completion: &str) -> TokenUsage {
    TokenUsage {
        prompt_tokens: String::from_utf8_lossy(prompt)
            .split_whitespace()
            .count()
            .max(1) as u32,
        completion_tokens: completion.split_whitespace().count().max(1) as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IntegrityRoute, ModelDevice};
    use std::{fs, io::Write};

    fn unique_model_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tachyon-magnetar-cutover-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ))
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
        let config_json = format!(
            r#"{{
                "architectures": ["Qwen2ForCausalLM"],
                "model_type": "qwen2",
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
            r#"{"bos_token": "<bos>", "eos_token": "<eos>"}"#,
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

    fn trust_tachyon_qwen_bundle(path: &Path) {
        use ::magnetar_runtime::production_model_ingestion::{
            ProductionModelArtifactIngestor, ProductionModelSource,
        };

        let source = ProductionModelSource::authorized_local_bundle(
            ::magnetar_runtime::ModelArtifactSource::Tachyon("tachyon:test-fixture".to_owned()),
            path.to_path_buf(),
        );
        let ingested = magnetar_loader_huggingface::HuggingFaceIngestor::new()
            .ingest(&source)
            .expect("fixture should ingest before writing Tachyon trust policy");
        fs::write(
            path.join(".tachyon-model-trust.json"),
            format!(
                r#"{{"trusted_digests":["{}"]}}"#,
                ingested.manifest.id.digest.value
            ),
        )
        .expect("trust sidecar should be written");
    }

    #[test]
    fn magnetar_qwen_binding_generates_through_real_production_cpu_path() {
        let model_dir = unique_model_dir("qwen-runtime");
        write_tiny_production_qwen_bundle(&model_dir);
        trust_tachyon_qwen_bundle(&model_dir);
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.models = vec![IntegrityModelBinding {
            alias: "qwen35".to_owned(),
            path: format!("magnetar:{}", model_dir.display()),
            device: ModelDevice::Cpu,
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
            .compute_component_prompt("qwen35", r#"{"prompt":"hi","max_new_tokens":1}"#)
            .expect("real Magnetar production Qwen should generate");

        assert!(!generation.is_empty());
        let _ = std::fs::remove_dir_all(model_dir);
    }

    #[test]
    fn explicit_magnetar_binding_rejects_non_qwen_model_directory() {
        let model_dir = unique_model_dir("non-qwen");
        write_tiny_production_qwen_bundle(&model_dir);
        std::fs::write(model_dir.join("config.json"), br#"{"model_type":"llama"}"#)
            .expect("config should be written");
        let error = match load_binding(&IntegrityModelBinding {
            alias: "llama".to_owned(),
            path: format!("magnetar:{}", model_dir.display()),
            device: ModelDevice::Cpu,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        }) {
            Ok(_) => panic!("Magnetar cutover accepts only Qwen directories"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("Magnetar failed"));
        let _ = std::fs::remove_dir_all(model_dir);
    }

    #[test]
    fn explicit_magnetar_binding_rejects_untrusted_qwen_bundle() {
        let model_dir = unique_model_dir("untrusted-qwen");
        write_tiny_production_qwen_bundle(&model_dir);
        let error = match load_binding(&IntegrityModelBinding {
            alias: "qwen-untrusted".to_owned(),
            path: format!("magnetar:{}", model_dir.display()),
            device: ModelDevice::Cpu,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        }) {
            Ok(_) => panic!("Magnetar Qwen bundle must require explicit Tachyon trust policy"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("trust rejected"));
        let _ = std::fs::remove_dir_all(model_dir);
    }

    #[cfg(not(feature = "magnetar-cuda"))]
    #[test]
    fn explicit_magnetar_cuda_binding_fails_closed_without_cuda_feature() {
        let model_dir = unique_model_dir("cuda-qwen");
        write_tiny_production_qwen_bundle(&model_dir);
        trust_tachyon_qwen_bundle(&model_dir);
        let error = match load_binding(&IntegrityModelBinding {
            alias: "qwen-cuda".to_owned(),
            path: format!("magnetar:{}", model_dir.display()),
            device: ModelDevice::Cuda,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        }) {
            Ok(_) => panic!("explicit CUDA placement must not silently fall back to CPU"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("CUDA provider"));
        let _ = std::fs::remove_dir_all(model_dir);
    }

    #[cfg(feature = "magnetar-cuda")]
    #[test]
    #[ignore = "run explicitly on a GPU runner guaranteed to have CUDA available"]
    fn magnetar_cuda_provider_hardware_required_guard() {
        let provider = magnetar_provider_cuda::CudaProvider::new();
        assert!(
            provider.is_available(),
            "GPU CI selected this test but Magnetar CudaProvider is unavailable"
        );
    }

    #[cfg(feature = "magnetar-cuda")]
    #[test]
    #[ignore = "run explicitly on a GPU runner guaranteed to have CUDA available"]
    fn magnetar_qwen_binding_generates_first_token_on_real_cuda_provider() {
        let provider = magnetar_provider_cuda::CudaProvider::new();
        assert!(
            provider.is_available(),
            "GPU CI selected this test but Magnetar CudaProvider is unavailable"
        );
        let model_dir = unique_model_dir("cuda-qwen-runtime");
        write_tiny_production_qwen_bundle(&model_dir);
        trust_tachyon_qwen_bundle(&model_dir);
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.models = vec![IntegrityModelBinding {
            alias: "qwen-cuda".to_owned(),
            path: format!("magnetar:{}", model_dir.display()),
            device: ModelDevice::Cuda,
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
            .compute_component_prompt("qwen-cuda", r#"{"prompt":"hi","max_new_tokens":1}"#)
            .expect("real Magnetar production Qwen should generate one token on CUDA");

        assert!(!generation.is_empty());
        let telemetry = inference_execution_telemetry();
        assert!(
            telemetry.iter().any(|event| event.alias == "qwen-cuda"
                && event.succeeded
                && event.executed_on.to_ascii_lowercase().contains("cuda")),
            "CUDA generation must record a real Magnetar CUDA provider execution"
        );
        let _ = std::fs::remove_dir_all(model_dir);
    }

    #[cfg(feature = "magnetar-cuda")]
    #[test]
    #[ignore = "run explicitly on a GPU runner guaranteed to have CUDA available"]
    fn magnetar_qwen_cuda_multi_token_request_fails_closed() {
        let provider = magnetar_provider_cuda::CudaProvider::new();
        assert!(
            provider.is_available(),
            "GPU CI selected this test but Magnetar CudaProvider is unavailable"
        );
        let model_dir = unique_model_dir("cuda-qwen-multitoken");
        write_tiny_production_qwen_bundle(&model_dir);
        trust_tachyon_qwen_bundle(&model_dir);
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.models = vec![IntegrityModelBinding {
            alias: "qwen-cuda".to_owned(),
            path: format!("magnetar:{}", model_dir.display()),
            device: ModelDevice::Cuda,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        }];

        let runtime = AiInferenceRuntime::from_config(&IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        })
        .expect("real Magnetar production Qwen CUDA bundle should load");
        let error = runtime
            .compute_component_prompt("qwen-cuda", r#"{"prompt":"hi","max_new_tokens":2}"#)
            .expect_err(
                "multi-token CUDA generation must fail closed until decode is device-resident",
            );

        assert!(
            error.to_string().contains("first-token"),
            "unexpected CUDA multi-token rejection: {error}"
        );
        let _ = std::fs::remove_dir_all(model_dir);
    }

    #[test]
    fn local_non_qwen_candle_style_directory_is_rejected() {
        let model_dir = unique_model_dir("legacy-candle");
        std::fs::create_dir_all(&model_dir).expect("fixture dir should be created");
        std::fs::write(model_dir.join("config.json"), br#"{"model_type":"llama"}"#)
            .expect("config should be written");

        let error = match AiInferenceRuntime::from_config(&IntegrityConfig {
            routes: vec![{
                let mut route = IntegrityRoute::user("/api/guest-ai");
                route.models = vec![IntegrityModelBinding {
                    alias: "llama".to_owned(),
                    path: model_dir.to_string_lossy().into_owned(),
                    device: ModelDevice::Cpu,
                    qos: RouteQos::Standard,
                    dynamic: false,
                    hardware_strategy: Default::default(),
                }];
                route
            }],
            ..IntegrityConfig::default_sealed()
        }) {
            Ok(_) => panic!("legacy Candle directories must not load"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("Magnetar cutover accepts"));
        let _ = std::fs::remove_dir_all(model_dir);
    }

    #[test]
    fn buffered_generation_rejects_lora_adapter_instead_of_ignoring_it() {
        let runtime = AiInferenceRuntime::from_config(&IntegrityConfig {
            routes: vec![{
                let mut route = IntegrityRoute::user("/api/guest-ai");
                route.models = vec![IntegrityModelBinding {
                    alias: "mock-model".to_owned(),
                    path: "mock".to_owned(),
                    device: ModelDevice::Cpu,
                    qos: RouteQos::Standard,
                    dynamic: false,
                    hardware_strategy: Default::default(),
                }];
                route
            }],
            ..IntegrityConfig::default_sealed()
        })
        .expect("runtime");

        let error = runtime
            .compute_component_prompt_with_adapter("mock-model", "hello", Some("adapter-a"))
            .expect_err("adapter injection must not silently use the base model");

        assert!(error.invalid_request);
        assert!(error.to_string().contains("not supported"));
    }

    #[test]
    fn streaming_generation_rejects_lora_adapter_as_invalid_request() {
        let runtime = AiInferenceRuntime::from_config(&IntegrityConfig {
            routes: vec![{
                let mut route = IntegrityRoute::user("/api/guest-ai");
                route.models = vec![IntegrityModelBinding {
                    alias: "mock-model".to_owned(),
                    path: "mock".to_owned(),
                    device: ModelDevice::Cpu,
                    qos: RouteQos::Standard,
                    dynamic: false,
                    hardware_strategy: Default::default(),
                }];
                route
            }],
            ..IntegrityConfig::default_sealed()
        })
        .expect("runtime");
        struct TestSink;
        impl StreamSink for TestSink {
            fn emit(&mut self, _event: StreamEvent<'_>) -> StreamControl {
                StreamControl::Continue
            }
        }
        let mut sink = TestSink;

        let error = runtime
            .stream_component_prompt("mock-model", "hello", Some("adapter-a"), &mut sink)
            .expect_err("streaming adapter injection must be a client error");

        assert!(error.invalid_request);
        assert!(error.to_string().contains("adapter-a"));
    }

    #[test]
    fn dynamic_openai_placeholders_do_not_collide_as_upstream_credentials() {
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.models = vec![
            IntegrityModelBinding {
                alias: "vendor-a".to_owned(),
                path: "openai:http://placeholder.invalid/v1".to_owned(),
                device: ModelDevice::Cpu,
                qos: RouteQos::Standard,
                dynamic: true,
                hardware_strategy: Default::default(),
            },
            IntegrityModelBinding {
                alias: "vendor_a".to_owned(),
                path: "openai:http://placeholder.invalid/v1".to_owned(),
                device: ModelDevice::Cpu,
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
    fn accelerator_support_is_derived_from_loaded_models() {
        let runtime =
            AiInferenceRuntime::from_config(&IntegrityConfig::default_sealed()).expect("runtime");

        assert!(runtime.supports_accelerator(AcceleratorKind::Cpu));
        assert!(!runtime.supports_accelerator(AcceleratorKind::Gpu));
        assert!(!runtime.supports_accelerator(AcceleratorKind::Npu));
        assert!(!runtime.supports_accelerator(AcceleratorKind::Tpu));
    }

    #[test]
    fn upstream_admission_bounds_concurrent_work() {
        let gate = UpstreamAdmission::new(1);
        {
            let mut state = gate.state.lock().expect("admission lock");
            state.in_flight = 1;
            state.waiting = 1;
        }

        match gate.acquire() {
            Err(UpstreamAdmissionError::QueueFull { waiting, limit }) => {
                assert_eq!(waiting, 1);
                assert_eq!(limit, 1);
            }
            _ => panic!("expected queue-full admission error"),
        }

        assert_eq!(gate.in_flight(), 1);
        gate.release();
        assert_eq!(gate.in_flight(), 0);
    }
}
