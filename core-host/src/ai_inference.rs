#[path = "ai_inference/magnetar_runtime.rs"]
mod magnetar_runtime;
#[path = "ai_inference/upstream_openai.rs"]
mod upstream_openai;

use anyhow::{anyhow, Result};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, RwLock},
};
use wasmtime_wasi_nn::{
    witx::WasiNnCtx, Graph as WasiGraph, GraphRegistry, Registry as WasiRegistry,
};

use crate::{IntegrityConfig, IntegrityModelBinding, RouteQos};

pub(crate) const UPSTREAM_SCHEME: &str = "openai:";
pub(crate) use magnetar_runtime::MAGNETAR_PATH_PREFIX;
const MOCK_INFERENCE_RESPONSE: &str = "MOCK_LLM_RESPONSE";

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
    MagnetarArena,
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
                if runtime.provider().device_id.starts_with("CUDA") {
                    AcceleratorKind::Gpu
                } else {
                    AcceleratorKind::Cpu
                }
            }
            ModelRuntime::Upstream(_) => AcceleratorKind::Network,
        }
    }

    fn memory_residency(&self) -> AcceleratorMemoryResidency {
        match self.runtime {
            ModelRuntime::Mock {
                accelerator: AcceleratorKind::Gpu,
            } => AcceleratorMemoryResidency::Vram,
            ModelRuntime::Mock {
                accelerator: AcceleratorKind::Npu | AcceleratorKind::Tpu,
            } => AcceleratorMemoryResidency::Sram,
            ModelRuntime::Mock { .. } | ModelRuntime::Upstream(_) => {
                AcceleratorMemoryResidency::HostRam
            }
            ModelRuntime::Magnetar(_) => AcceleratorMemoryResidency::MagnetarArena,
        }
    }
}

#[derive(Clone)]
pub(crate) struct AiInferenceRuntime {
    models: Arc<RwLock<HashMap<String, LoadedModel>>>,
    dynamic_models_root: Option<PathBuf>,
    queue_snapshots: Arc<RwLock<HashMap<AcceleratorKind, QueueTierSnapshot>>>,
}

impl AiInferenceRuntime {
    pub(crate) fn from_config(config: &IntegrityConfig) -> Result<Self> {
        assert_no_credential_collisions(
            config
                .routes
                .iter()
                .flat_map(|route| route.models.iter())
                .filter(|binding| binding.path.trim().starts_with(UPSTREAM_SCHEME))
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
        WasiNnCtx::new([], WasiRegistry::from(EmptyGraphRegistry))
    }

    pub(crate) fn supports_accelerator(&self, accelerator: AcceleratorKind) -> bool {
        AcceleratorKind::ALL.contains(&accelerator)
    }

    pub(crate) fn queue_tier_snapshot(&self, accelerator: AcceleratorKind) -> QueueTierSnapshot {
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
        match &model.runtime {
            ModelRuntime::Upstream(runtime) => runtime.embed(input).map_err(GenerationError::from),
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
        if adapter_id.is_some() {
            return Err(GenerationError::local(
                "Magnetar cutover does not support LoRA adapter injection",
            ));
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
        advertisements.sort_by(|a, b| a.device_id.cmp(&b.device_id));
        advertisements.dedup_by(|a, b| a.device_id == b.device_id);
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

    fn unique_model_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tachyon-magnetar-cutover-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ))
    }

    fn write_qwen_safetensors_fixture(path: &Path) {
        std::fs::create_dir_all(path).expect("fixture dir should be created");
        std::fs::write(path.join("config.json"), br#"{"model_type":"qwen3"}"#)
            .expect("config should be written");
        std::fs::write(path.join("model.safetensors"), b"weights")
            .expect("weights should be written");
    }

    #[test]
    fn magnetar_qwen_binding_is_rejected_until_real_execution_exists() {
        let model_dir = unique_model_dir("qwen-runtime");
        write_qwen_safetensors_fixture(&model_dir);
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.models = vec![IntegrityModelBinding {
            alias: "qwen35".to_owned(),
            path: format!("magnetar:{}", model_dir.display()),
            device: ModelDevice::Cpu,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        }];

        let error = match AiInferenceRuntime::from_config(&IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        }) {
            Ok(_) => panic!("Qwen safetensors must not be admitted until Magnetar executes them"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("not implemented yet"));
        let _ = std::fs::remove_dir_all(model_dir);
    }

    #[test]
    fn explicit_magnetar_binding_rejects_non_qwen_model_directory() {
        let model_dir = unique_model_dir("non-qwen");
        std::fs::create_dir_all(&model_dir).expect("fixture dir should be created");
        std::fs::write(model_dir.join("config.json"), br#"{"model_type":"llama"}"#)
            .expect("config should be written");
        std::fs::write(model_dir.join("model.safetensors"), b"weights")
            .expect("weights should be written");
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

        assert!(error
            .to_string()
            .contains("expected a Qwen safetensors directory"));
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
}
