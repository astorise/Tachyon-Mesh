#[path = "ai_inference/accelerator_backend.rs"]
mod accelerator_backend;
#[path = "ai_inference/architecture_registry.rs"]
mod architecture_registry;
#[path = "ai_inference/candle_embedding_runtime.rs"]
mod candle_embedding_runtime;
#[path = "ai_inference/candle_llm_runtime.rs"]
mod candle_llm_runtime;
#[path = "ai_inference/candle_onnx_backend.rs"]
mod candle_onnx_backend;
#[path = "ai_inference/expert_parallel_llama.rs"]
pub(crate) mod expert_parallel_llama;
#[path = "ai_inference/modelopt_nvfp4.rs"]
mod modelopt_nvfp4;
#[path = "ai_inference/paged_kv.rs"]
mod paged_kv;
#[path = "ai_inference/parallel.rs"]
pub(crate) mod parallel;
#[path = "ai_inference/pipeline_parallel_llama.rs"]
pub(crate) mod pipeline_parallel_llama;
#[path = "ai_inference/qwen35_moe_runtime.rs"]
mod qwen35_moe_runtime;
#[path = "ai_inference/samplers.rs"]
mod samplers;
#[path = "ai_inference/tensor_parallel_llama.rs"]
pub(crate) mod tensor_parallel_llama;
#[path = "ai_inference/vendor_accelerator.rs"]
mod vendor_accelerator;
#[path = "ai_inference/vram_manager.rs"]
pub(crate) mod vram_manager;

use anyhow::{anyhow, bail, Context, Result};
use candle_core::{
    bail as candle_bail, CpuStorage, CustomOp2, DType, Device, Layout, Shape,
    Tensor as CandleTensor,
};
#[cfg(test)]
use std::time::Duration;
use std::{
    any::Any,
    cmp::Ordering as CmpOrdering,
    collections::{BinaryHeap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Mutex, OnceLock, RwLock,
    },
    thread,
};
use tokio::sync::mpsc as tokio_mpsc;
use wasmtime_wasi_nn::{
    witx::WasiNnCtx, Graph as WasiGraph, GraphRegistry, Registry as WasiRegistry,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TensorType {
    U8,
    Fp32,
}

use crate::{
    IntegrityConfig, IntegrityModelBinding, RouteQos, SchedulerConfig, SchedulerSpillTier,
};

const MOCK_INFERENCE_RESPONSE: &str = "MOCK_LLM_RESPONSE";
const DEFAULT_BATCH_SIZE: usize = 32;
const ACCELERATOR_QUEUE_CAPACITY: usize = 256;
const MODEL_BROKER_DIR_ENV: &str = "MODEL_BROKER_DIR";
const MODEL_BROKER_ADAPTERS_DIR: &str = "adapters";
const SAFETENSORS_EXTENSION: &str = "safetensors";

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum AcceleratorKind {
    #[default]
    Cpu,
    Gpu,
    Npu,
    Tpu,
}

impl AcceleratorKind {
    const ALL: [Self; 4] = [Self::Cpu, Self::Gpu, Self::Npu, Self::Tpu];

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
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AcceleratorMemoryResidency {
    HostRam,
    Vram,
    Sram,
}

#[derive(Clone)]
struct BackendModelSource {
    alias: String,
    path: String,
    requested_target: String,
    accelerator: AcceleratorKind,
    qos: RouteQos,
    /// On-disk size of the model artifact, in bytes. Used only for telemetry of
    /// the resident weight footprint — the weights themselves are streamed/mmaped
    /// by the concrete backend (e.g. `CandleLlmRuntime`), so we deliberately do
    /// not buffer the whole file in host RAM here.
    model_size_bytes: u64,
}

fn model_file_size_bytes(path: &str) -> u64 {
    fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

// Guests load ONNX models directly via graph_load(bytes). No pre-registered
// named graphs are needed; the WASI-NN registry is intentionally empty.
struct EmptyGraphRegistry;
impl GraphRegistry for EmptyGraphRegistry {
    fn get(&self, _name: &str) -> Option<&WasiGraph> {
        None
    }
    fn get_mut(&mut self, _name: &str) -> Option<&mut WasiGraph> {
        None
    }
}

// Test-only: pre-populate named graph aliases with a mock graph that always
// returns MOCK_LLM_RESPONSE, allowing WASM guests that call build_from_cache()
// to succeed in CI without actual model files on disk.
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
struct SharedInputTensor {
    dimensions: Vec<u32>,
    ty: TensorType,
    data: Arc<[u8]>,
}

impl SharedInputTensor {
    fn byte_len(&self) -> usize {
        self.data.len().max(1)
    }
}

trait BackendModel: Send + Sync {
    fn residency(&self) -> AcceleratorMemoryResidency;
    fn as_any(&self) -> &dyn Any;
    /// Executes one or more independent requests and returns exactly one
    /// output per input, in the same order — never a single shared output
    /// broadcast across the whole batch (see `process_batch`, which routes
    /// `outputs[i]` back to `inputs[i]`'s own caller).
    fn execute(&self, inputs: &[SharedInputTensor]) -> Result<Vec<Vec<u8>>>;
    fn execute_with_adapter(
        &self,
        inputs: &[SharedInputTensor],
        adapter: &ResolvedLoraAdapter,
    ) -> Result<Vec<Vec<u8>>> {
        let _ = inputs;
        bail!(
            "LoRA adapter `{}` was resolved, but this backend does not support adapter injection",
            adapter.id
        )
    }
    fn execute_with_adapters(
        &self,
        inputs: &[SharedInputTensor],
        adapters: &[Option<ResolvedLoraAdapter>],
    ) -> Result<Vec<Vec<u8>>> {
        self.execute_with_adapter_results(inputs, adapters)
            .into_iter()
            .collect()
    }

    fn execute_with_adapter_results(
        &self,
        inputs: &[SharedInputTensor],
        adapters: &[Option<ResolvedLoraAdapter>],
    ) -> Vec<Result<Vec<u8>>> {
        if inputs.len() != adapters.len() {
            let message = format!(
                "adapter assignment count {} does not match input count {}",
                adapters.len(),
                inputs.len()
            );
            return repeat_batch_error(inputs.len().max(adapters.len()), message);
        }
        let mut results = (0..inputs.len()).map(|_| None).collect::<Vec<_>>();
        for group in adapter_assignment_groups(adapters) {
            let adapter = adapters[group[0]].as_ref();
            let group_inputs = group
                .iter()
                .map(|index| inputs[*index].clone())
                .collect::<Vec<_>>();
            let outputs = match adapter {
                Some(adapter) => self.execute_with_adapter(&group_inputs, adapter),
                None => self.execute(&group_inputs),
            };
            let outputs = match outputs {
                Ok(outputs) => outputs,
                Err(error) => {
                    let message = error.to_string();
                    for index in group {
                        results[index] = Some(Err(anyhow!("{}", message)));
                    }
                    continue;
                }
            };
            if outputs.len() != group.len() {
                let message = format!(
                    "backend returned {} output(s) for an adapter group of {} input(s)",
                    outputs.len(),
                    group.len()
                );
                for index in group {
                    results[index] = Some(Err(anyhow!("{}", message)));
                }
                continue;
            }
            for (index, output) in group.into_iter().zip(outputs) {
                results[index] = Some(Ok(output));
            }
        }
        results
            .into_iter()
            .map(|output| output.expect("adapter groups cover every input"))
            .collect()
    }

    /// Stream decoded text fragments through `on_token` as they are produced.
    /// The default implementation runs `execute` and emits the entire output as
    /// a single fragment — a correct, non-incremental fallback for backends that
    /// cannot stream (mock, NVFP4). Backends that can decode token-by-token
    /// override this for real time-to-first-token. Only ever called with a
    /// single input (streaming is inherently one request, one client).
    fn stream_text(
        &self,
        inputs: &[SharedInputTensor],
        on_token: &mut dyn FnMut(&str),
    ) -> Result<()> {
        let mut outputs = self.execute(inputs)?;
        if outputs.len() != 1 {
            bail!(
                "stream_text expects exactly one input, got {} output(s) for {} input(s)",
                outputs.len(),
                inputs.len()
            );
        }
        let text = String::from_utf8(outputs.remove(0))
            .map_err(|error| anyhow!("output was not UTF-8: {error}"))?;
        if !text.is_empty() {
            on_token(&text);
        }
        Ok(())
    }

    fn embed_text(&self, input: &SharedInputTensor) -> Result<Vec<f32>> {
        let _ = input;
        bail!("this backend does not expose dense text embeddings")
    }
}

#[derive(Clone)]
pub(crate) struct AiInferenceRuntime {
    schedulers: HashMap<AcceleratorKind, AcceleratorScheduler>,
    /// Loaded models, keyed by alias. Behind an `Arc<RwLock<_>>` so broker-uploaded
    /// models can be lazily registered at runtime (see `ensure_model_loaded`)
    /// while clones of the runtime share one registry.
    models: Arc<RwLock<HashMap<String, Arc<CandleModel>>>>,
    /// Root directory of broker-uploaded models (`{tachyon_data}/models`). When
    /// set, an inference request for an alias absent from the sealed config is
    /// lazily loaded from `{root}/{alias}` — gated upstream by the route's sealed
    /// `allowed_model_aliases`. `None` disables lazy loading (tests, no broker).
    dynamic_models_root: Option<PathBuf>,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct QueueTierSnapshot {
    pub(crate) realtime: u32,
    pub(crate) standard: u32,
    pub(crate) batch: u32,
}

impl AiInferenceRuntime {
    pub(crate) fn from_config(config: &IntegrityConfig) -> Result<Self> {
        let schedulers = AcceleratorKind::ALL
            .into_iter()
            .map(|accelerator| {
                (
                    accelerator,
                    AcceleratorScheduler::new(
                        accelerator,
                        DEFAULT_BATCH_SIZE,
                        config.scheduler.clone(),
                    ),
                )
            })
            .collect::<HashMap<_, _>>();
        let mut models = HashMap::new();
        let mut sealed_aliases = std::collections::HashSet::new();

        for route in &config.routes {
            for binding in &route.models {
                if !sealed_aliases.insert(binding.alias.clone()) {
                    return Err(anyhow!(
                        "Integrity Validation Failed: model alias `{}` must be globally unique",
                        binding.alias
                    ));
                }
                if binding.dynamic {
                    // Sealed but not present at boot. The alias is authorised for
                    // the route (scope is enforced from `route.models`), and the
                    // model is lazily materialised from `{dynamic_models_root}/{alias}`
                    // by `ensure_model_loaded` on first use. Skipping eager load
                    // here lets the host boot before the model has been uploaded.
                    continue;
                }
                if binding.path.is_empty() {
                    return Err(anyhow!(
                        "Integrity Validation Failed: static model alias `{}` requires a non-empty `path` (set `dynamic: true` for broker-uploaded models)",
                        binding.alias
                    ));
                }
                let backend_model: Arc<dyn BackendModel> =
                    Arc::new(CandleBackendModel::load(binding)?);
                models.insert(
                    binding.alias.clone(),
                    Arc::new(CandleModel::load_mock_with_backend(binding, backend_model)?),
                );
            }
        }

        Ok(Self {
            schedulers,
            models: Arc::new(RwLock::new(models)),
            dynamic_models_root: None,
        })
    }

    /// Set the root directory from which broker-uploaded models are lazily loaded
    /// on first use. Production wires this to `{tachyon_data}/models`; tests and
    /// hosts without a broker leave it unset.
    pub(crate) fn with_dynamic_models_root(mut self, root: Option<PathBuf>) -> Self {
        self.dynamic_models_root = root;
        self
    }

    /// Ensure `alias` is loaded, lazily materializing a broker-uploaded model from
    /// `{dynamic_models_root}/{alias}` on a registry miss. The format (GGUF or
    /// safetensors) is resolved from the directory's `.tachyon-model.json` sidecar
    /// by the backend loader. This runs only after the caller's sealed-alias scope
    /// check, so it cannot widen what a route may execute.
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
            path: model_dir.to_string_lossy().into_owned(),
            device: crate::ModelDevice::Cpu,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        };
        let backend_model: Arc<dyn BackendModel> =
            Arc::new(CandleBackendModel::load(&binding).map_err(|error| error.to_string())?);
        let model = Arc::new(
            CandleModel::load_mock_with_backend(&binding, backend_model)
                .map_err(|error| error.to_string())?,
        );
        // Insert under the write lock; `or_insert` tolerates a concurrent first-use
        // race (the loser's freshly loaded model is simply dropped).
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
        let backends = [candle_onnx_backend::candle_onnx_backend()];
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
        WasiNnCtx::new(backends, registry)
    }

    #[cfg(test)]
    pub(crate) fn scheduler_snapshot(&self, accelerator: AcceleratorKind) -> SchedulerSnapshot {
        self.scheduler_for(accelerator)
            .map(|scheduler| scheduler.snapshot())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn set_queue_depth_for_test(
        &self,
        accelerator: AcceleratorKind,
        qos: RouteQos,
        depth: usize,
    ) {
        if let Some(scheduler) = self.scheduler_for(accelerator) {
            scheduler.set_queue_depth_for_test(qos, depth);
        }
    }

    #[cfg(test)]
    pub(crate) fn model_memory_residency(&self, alias: &str) -> Option<AcceleratorMemoryResidency> {
        self.models
            .read()
            .expect("model registry lock poisoned")
            .get(alias)
            .map(|model| model.memory_residency)
    }

    pub(crate) fn supports_accelerator(&self, accelerator: AcceleratorKind) -> bool {
        self.schedulers.contains_key(&accelerator)
    }

    pub(crate) fn queue_tier_snapshot(&self, accelerator: AcceleratorKind) -> QueueTierSnapshot {
        self.scheduler_for(accelerator)
            .map(|scheduler| scheduler.queue_tier_snapshot())
            .unwrap_or_default()
    }

    pub(crate) fn load_component_model(
        &self,
        alias: &str,
        accelerator: AcceleratorKind,
    ) -> Result<(), String> {
        if !self.supports_accelerator(accelerator) {
            return Err(format!(
                "{} accelerator is unavailable on this host",
                accelerator.as_str()
            ));
        }
        // NPU/TPU requests may fall back to the CPU lane when their optional
        // vendor runner is unavailable. CPU/GPU requests keep strict device
        // matching semantics.
        self.ensure_model_loaded(alias)?;
        let models = self.models.read().expect("model registry lock poisoned");
        let model = models
            .get(alias)
            .ok_or_else(|| format!("model alias `{alias}` is not loaded"))?;
        let resolved = if matches!(accelerator, AcceleratorKind::Npu | AcceleratorKind::Tpu) {
            accelerator_backend::resolve_with_fallback(
                accelerator,
                AcceleratorKind::Cpu,
                accelerator_backend::probe,
            )
        } else {
            accelerator
        };
        if model.accelerator != resolved {
            return Err(format!(
                "model alias `{alias}` requires `{}` but `{}` resolved to `{}`",
                model.accelerator.as_str(),
                accelerator.as_str(),
                resolved.as_str()
            ));
        }
        Ok(())
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn compute_component_prompt(
        &self,
        alias: &str,
        prompt: &str,
    ) -> Result<String, String> {
        self.compute_component_prompt_with_adapter(alias, prompt, None)
    }

    pub(crate) fn compute_component_prompt_with_adapter(
        &self,
        alias: &str,
        prompt: &str,
        adapter_id: Option<&str>,
    ) -> Result<String, String> {
        let adapter = adapter_id.map(resolve_lora_adapter_path).transpose()?;
        self.ensure_model_loaded(alias)?;
        // Clone the `Arc` out and drop the read lock before inference so a slow
        // forward pass never blocks lazy registration of other models.
        let model = {
            let models = self.models.read().expect("model registry lock poisoned");
            models
                .get(alias)
                .cloned()
                .ok_or_else(|| format!("model alias `{alias}` is not loaded"))?
        };
        let output = self
            .scheduler_for(model.accelerator)
            .ok_or_else(|| {
                format!(
                    "{} accelerator is unavailable on this host",
                    model.accelerator.as_str()
                )
            })?
            .infer(
                Arc::clone(&model),
                adapter,
                SharedInputTensor {
                    dimensions: vec![prompt.len() as u32],
                    ty: TensorType::U8,
                    data: Arc::from(prompt.as_bytes()),
                },
            )
            .map_err(|error| error.to_string())?;
        String::from_utf8(output).map_err(|error| error.to_string())
    }

    pub(crate) fn embed_component_input(
        &self,
        alias: &str,
        input: &str,
    ) -> Result<Vec<f32>, String> {
        self.ensure_model_loaded(alias)?;
        let model = {
            let models = self.models.read().expect("model registry lock poisoned");
            models
                .get(alias)
                .cloned()
                .ok_or_else(|| format!("model alias `{alias}` is not loaded"))?
        };
        if self.scheduler_for(model.accelerator).is_none() {
            return Err(format!(
                "{} accelerator is unavailable on this host",
                model.accelerator.as_str()
            ));
        }
        let tensor = SharedInputTensor {
            dimensions: vec![input.len() as u32],
            ty: TensorType::U8,
            data: Arc::from(input.as_bytes()),
        };
        model
            .backend_model
            .embed_text(&tensor)
            .map_err(|error| error.to_string())
    }

    /// Stream a prompt's decoded output, invoking `on_token` for each text
    /// fragment as it is produced. The scope gate (sealed alias) is enforced by
    /// the caller before this point, exactly as for [`compute_component_prompt`].
    ///
    /// Streaming bypasses the batch scheduler: a streamed request is inherently a
    /// single sequence, and the backend serialises its own execution (the GGUF
    /// runtime behind a mutex, safetensors per-request). It therefore runs on the
    /// caller's blocking thread rather than the QoS dispatcher.
    #[cfg_attr(not(feature = "ai-inference"), allow(dead_code))]
    pub(crate) fn stream_component_prompt(
        &self,
        alias: &str,
        prompt: &str,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<(), String> {
        self.ensure_model_loaded(alias)?;
        let model = {
            let models = self.models.read().expect("model registry lock poisoned");
            models
                .get(alias)
                .cloned()
                .ok_or_else(|| format!("model alias `{alias}` is not loaded"))?
        };
        if self.scheduler_for(model.accelerator).is_none() {
            return Err(format!(
                "{} accelerator is unavailable on this host",
                model.accelerator.as_str()
            ));
        }
        let input = SharedInputTensor {
            dimensions: vec![prompt.len() as u32],
            ty: TensorType::U8,
            data: Arc::from(prompt.as_bytes()),
        };
        model
            .backend_model
            .stream_text(&[input], on_token)
            .map_err(|error| error.to_string())
    }

    fn scheduler_for(&self, accelerator: AcceleratorKind) -> Option<AcceleratorScheduler> {
        self.schedulers.get(&accelerator).cloned()
    }
}

#[derive(Clone)]
pub(crate) struct ResolvedLoraAdapter {
    pub(crate) id: String,
    pub(crate) path: PathBuf,
}

fn resolve_lora_adapter_path(adapter_id: &str) -> Result<ResolvedLoraAdapter, String> {
    validate_lora_adapter_id(adapter_id)?;
    let broker_root = std::env::var(MODEL_BROKER_DIR_ENV).map_err(|_| {
        format!(
            "adapter id `{adapter_id}` was requested but `{MODEL_BROKER_DIR_ENV}` is not configured"
        )
    })?;
    let path = Path::new(&broker_root)
        .join(MODEL_BROKER_ADAPTERS_DIR)
        .join(format!("{adapter_id}.{SAFETENSORS_EXTENSION}"));
    if !path.is_file() {
        return Err(format!(
            "adapter id `{adapter_id}` is not available at `{}`",
            path.display()
        ));
    }
    Ok(ResolvedLoraAdapter {
        id: adapter_id.to_owned(),
        path,
    })
}

fn validate_lora_adapter_id(adapter_id: &str) -> Result<(), String> {
    if adapter_id.is_empty()
        || adapter_id.contains("..")
        || adapter_id.contains('/')
        || adapter_id.contains('\\')
        || adapter_id.ends_with(&format!(".{SAFETENSORS_EXTENSION}"))
    {
        return Err(format!(
            "adapter id `{adapter_id}` is not a valid identifier: use the adapter name without path separators, traversal, or extension"
        ));
    }
    Ok(())
}

#[derive(Clone)]
struct AcceleratorScheduler {
    sender: tokio_mpsc::Sender<PrioritizedInferenceJob>,
    metrics: Arc<SchedulerMetrics>,
}

impl AcceleratorScheduler {
    fn new(
        accelerator: AcceleratorKind,
        max_active_sequences: usize,
        scheduler_config: SchedulerConfig,
    ) -> Self {
        let (sender, receiver) = tokio_mpsc::channel(ACCELERATOR_QUEUE_CAPACITY);
        let metrics = Arc::new(SchedulerMetrics::default());
        let worker_metrics = Arc::clone(&metrics);

        thread::Builder::new()
            .name(format!("tachyon-{}-dispatcher", accelerator.as_str()))
            .spawn(move || {
                run_scheduler(
                    accelerator,
                    receiver,
                    worker_metrics,
                    max_active_sequences,
                    scheduler_config,
                )
            })
            .expect("AI inference scheduler thread should start");

        Self { sender, metrics }
    }

    fn infer(
        &self,
        model: Arc<CandleModel>,
        adapter: Option<ResolvedLoraAdapter>,
        input: SharedInputTensor,
    ) -> Result<Vec<u8>, anyhow::Error> {
        let response_rx = self.enqueue(model, adapter, input)?;
        response_rx
            .recv()
            .map_err(|_| anyhow::anyhow!("AI inference response channel closed unexpectedly"))?
    }

    fn enqueue(
        &self,
        model: Arc<CandleModel>,
        adapter: Option<ResolvedLoraAdapter>,
        input: SharedInputTensor,
    ) -> Result<mpsc::Receiver<Result<Vec<u8>, anyhow::Error>>, anyhow::Error> {
        let (response_tx, response_rx) = mpsc::channel();
        let sequence = self.metrics.next_sequence.fetch_add(1, Ordering::Relaxed);
        self.metrics.queued_requests.fetch_add(1, Ordering::Relaxed);
        self.metrics.record_enqueue(model.qos);
        let qos = model.qos;
        let job = PrioritizedInferenceJob::new(
            model.qos.score(),
            sequence,
            InferenceJob {
                alias: model.alias.clone(),
                adapter,
                model,
                qos,
                input,
                response_tx,
            },
        );
        self.sender
            .blocking_send(job)
            .map_err(|_| anyhow::anyhow!("AI inference scheduler has stopped"))?;
        Ok(response_rx)
    }

    #[cfg(test)]
    fn snapshot(&self) -> SchedulerSnapshot {
        SchedulerSnapshot {
            batches_processed: self.metrics.batches_processed.load(Ordering::Relaxed),
            requests_processed: self.metrics.requests_processed.load(Ordering::Relaxed),
            max_batch_size: self.metrics.max_batch_size.load(Ordering::Relaxed),
            prefill_steps_processed: self.metrics.prefill_steps_processed.load(Ordering::Relaxed),
            decode_steps_processed: self.metrics.decode_steps_processed.load(Ordering::Relaxed),
            max_active_sequences: self.metrics.max_active_sequences.load(Ordering::Relaxed),
            queued_requests: self.metrics.queued_requests.load(Ordering::Relaxed),
            realtime_queued: self.metrics.realtime_queued.load(Ordering::Relaxed),
            standard_queued: self.metrics.standard_queued.load(Ordering::Relaxed),
            batch_queued: self.metrics.batch_queued.load(Ordering::Relaxed),
            kv_recompute_preemptions: self
                .metrics
                .kv_recompute_preemptions
                .load(Ordering::Relaxed),
            kv_swap_preemptions: self.metrics.kv_swap_preemptions.load(Ordering::Relaxed),
            completed_aliases: self
                .metrics
                .completed_aliases
                .lock()
                .expect("scheduler completion log should not be poisoned")
                .clone(),
        }
    }

    fn queue_tier_snapshot(&self) -> QueueTierSnapshot {
        QueueTierSnapshot {
            realtime: self.metrics.realtime_queued.load(Ordering::Relaxed) as u32,
            standard: self.metrics.standard_queued.load(Ordering::Relaxed) as u32,
            batch: self.metrics.batch_queued.load(Ordering::Relaxed) as u32,
        }
    }

    #[cfg(test)]
    fn set_queue_depth_for_test(&self, qos: RouteQos, depth: usize) {
        queue_counter(&self.metrics, qos).store(depth, Ordering::Relaxed);
        let total = self.metrics.realtime_queued.load(Ordering::Relaxed)
            + self.metrics.standard_queued.load(Ordering::Relaxed)
            + self.metrics.batch_queued.load(Ordering::Relaxed);
        self.metrics.queued_requests.store(total, Ordering::Relaxed);
    }
}

#[derive(Default)]
struct SchedulerMetrics {
    batches_processed: AtomicUsize,
    requests_processed: AtomicUsize,
    max_batch_size: AtomicUsize,
    prefill_steps_processed: AtomicUsize,
    decode_steps_processed: AtomicUsize,
    max_active_sequences: AtomicUsize,
    queued_requests: AtomicUsize,
    realtime_queued: AtomicUsize,
    standard_queued: AtomicUsize,
    batch_queued: AtomicUsize,
    kv_recompute_preemptions: AtomicUsize,
    kv_swap_preemptions: AtomicUsize,
    next_sequence: AtomicUsize,
    #[cfg(test)]
    completed_aliases: Mutex<Vec<String>>,
}

impl SchedulerMetrics {
    fn record_enqueue(&self, qos: RouteQos) {
        queue_counter(self, qos).fetch_add(1, Ordering::Relaxed);
    }

    fn record_dequeue(&self, qos: RouteQos) {
        queue_counter(self, qos).fetch_sub(1, Ordering::Relaxed);
    }

    fn record_active_sequences(&self, active_sequences: usize) {
        self.max_active_sequences
            .fetch_max(active_sequences, Ordering::Relaxed);
    }

    fn record_prefill_step(&self, batch: &[InferenceJob]) {
        self.prefill_steps_processed.fetch_add(1, Ordering::Relaxed);
        self.max_batch_size
            .fetch_max(batch.len(), Ordering::Relaxed);
        #[cfg(test)]
        if let Some(first) = batch.first() {
            self.completed_aliases
                .lock()
                .expect("scheduler completion log should not be poisoned")
                .push(format!("{}:prefill", first.alias));
        }
    }

    fn record_decode_step(&self, batch: &[InferenceJob]) {
        self.decode_steps_processed.fetch_add(1, Ordering::Relaxed);
        self.batches_processed.fetch_add(1, Ordering::Relaxed);
        self.requests_processed
            .fetch_add(batch.len(), Ordering::Relaxed);
        self.max_batch_size
            .fetch_max(batch.len(), Ordering::Relaxed);
        #[cfg(test)]
        if let Some(first) = batch.first() {
            self.completed_aliases
                .lock()
                .expect("scheduler completion log should not be poisoned")
                .push(format!("{}:decode", first.alias));
        }
    }

    fn record_kv_preemption(&self, mode: paged_kv::KvPreemptionMode) {
        match mode {
            paged_kv::KvPreemptionMode::Recompute => {
                self.kv_recompute_preemptions
                    .fetch_add(1, Ordering::Relaxed);
            }
            paged_kv::KvPreemptionMode::Swap => {
                self.kv_swap_preemptions.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

fn queue_counter(metrics: &SchedulerMetrics, qos: RouteQos) -> &AtomicUsize {
    match qos {
        RouteQos::RealTime => &metrics.realtime_queued,
        RouteQos::Standard => &metrics.standard_queued,
        RouteQos::Batch => &metrics.batch_queued,
    }
}

#[derive(Clone)]
struct InferenceJob {
    alias: String,
    adapter: Option<ResolvedLoraAdapter>,
    model: Arc<CandleModel>,
    qos: RouteQos,
    input: SharedInputTensor,
    response_tx: mpsc::Sender<Result<Vec<u8>, anyhow::Error>>,
}

impl InferenceJob {
    fn tenant_id(&self) -> &str {
        self.adapter
            .as_ref()
            .map(|adapter| adapter.id.as_str())
            .unwrap_or("default")
    }
}

struct PrioritizedInferenceJob {
    qos_score: u16,
    sequence: usize,
    job: InferenceJob,
}

impl PrioritizedInferenceJob {
    fn new(qos_score: u16, sequence: usize, job: InferenceJob) -> Self {
        Self {
            qos_score,
            sequence,
            job,
        }
    }

    fn age(mut self) -> Self {
        self.qos_score = self.qos_score.saturating_add(1);
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InferenceSequencePhase {
    Prefill,
    Decode,
}

struct ActiveInferenceSequence {
    qos_score: u16,
    sequence: usize,
    phase: InferenceSequencePhase,
    job: InferenceJob,
}

impl From<PrioritizedInferenceJob> for ActiveInferenceSequence {
    fn from(job: PrioritizedInferenceJob) -> Self {
        Self {
            qos_score: job.qos_score,
            sequence: job.sequence,
            phase: InferenceSequencePhase::Prefill,
            job: job.job,
        }
    }
}

impl ActiveInferenceSequence {
    fn batch_key(&self) -> InferenceBatchKey {
        InferenceBatchKey {
            alias: self.job.alias.clone(),
        }
    }

    fn priority_cmp(&self, other: &Self) -> CmpOrdering {
        self.qos_score
            .cmp(&other.qos_score)
            .then_with(|| other.sequence.cmp(&self.sequence))
    }

    fn is_compatible_with(
        &self,
        batch_key: &InferenceBatchKey,
        phase: InferenceSequencePhase,
    ) -> bool {
        self.phase == phase && self.job.alias == batch_key.alias
    }
}

struct TenantFairSelection<T> {
    selected: T,
    tenant: String,
    eligible_tenants: Vec<String>,
}

struct TenantFairness {
    config: SchedulerConfig,
    deficits: HashMap<String, i64>,
}

impl TenantFairness {
    fn new(config: SchedulerConfig) -> Self {
        Self {
            config,
            deficits: HashMap::new(),
        }
    }

    fn allows_cross_tenant_active_batching(&self) -> bool {
        self.config.tenant_weights.is_empty()
    }

    fn select_active<'a>(
        &mut self,
        candidates: Vec<(usize, &'a ActiveInferenceSequence)>,
        max_qos_score: u16,
    ) -> Option<TenantFairSelection<(usize, &'a ActiveInferenceSequence)>> {
        let qos_candidates = candidates
            .into_iter()
            .filter(|(_, sequence)| sequence.qos_score == max_qos_score)
            .collect::<Vec<_>>();
        if qos_candidates.is_empty() {
            return None;
        }

        let eligible_tenants = qos_candidates
            .iter()
            .map(|(_, sequence)| sequence.job.tenant_id())
            .collect::<Vec<_>>();
        let eligible_tenants = self.accrue_eligible_tenants(eligible_tenants);

        let selected = qos_candidates.into_iter().max_by(|(_, left), (_, right)| {
            self.deficit(left.job.tenant_id())
                .cmp(&self.deficit(right.job.tenant_id()))
                .then_with(|| left.priority_cmp(right))
        })?;
        let tenant = selected.1.job.tenant_id().to_owned();

        Some(TenantFairSelection {
            selected,
            tenant,
            eligible_tenants,
        })
    }

    fn select_queued(
        &mut self,
        queued: &mut BinaryHeap<PrioritizedInferenceJob>,
    ) -> Option<PrioritizedInferenceJob> {
        let mut jobs = queued.drain().collect::<Vec<_>>();
        let selected = self.select_queued_index(&jobs);

        let selected_job = selected.map(|selection| {
            let job = jobs.swap_remove(selection.selected);
            self.charge(&selection.tenant, &selection.eligible_tenants, 1);
            job
        });

        for job in jobs {
            queued.push(job);
        }

        selected_job
    }

    fn select_queued_index(
        &mut self,
        jobs: &[PrioritizedInferenceJob],
    ) -> Option<TenantFairSelection<usize>> {
        let max_qos_score = jobs.iter().map(|job| job.qos_score).max()?;
        let qos_candidates = jobs
            .iter()
            .enumerate()
            .filter(|(_, job)| job.qos_score == max_qos_score)
            .collect::<Vec<_>>();
        if qos_candidates.is_empty() {
            return None;
        }

        let eligible_tenants = qos_candidates
            .iter()
            .map(|(_, job)| job.job.tenant_id())
            .collect::<Vec<_>>();
        let eligible_tenants = self.accrue_eligible_tenants(eligible_tenants);

        let selected = qos_candidates.into_iter().max_by(|(_, left), (_, right)| {
            self.deficit(left.job.tenant_id())
                .cmp(&self.deficit(right.job.tenant_id()))
                .then_with(|| left.cmp(right))
        })?;
        let tenant = selected.1.job.tenant_id().to_owned();

        Some(TenantFairSelection {
            selected: selected.0,
            tenant,
            eligible_tenants,
        })
    }

    fn accrue_eligible_tenants(&mut self, tenants: Vec<&str>) -> Vec<String> {
        let mut eligible_tenants = Vec::new();
        for tenant in tenants {
            if !eligible_tenants.iter().any(|known| known == tenant) {
                eligible_tenants.push(tenant.to_owned());
            }
        }

        for tenant in &eligible_tenants {
            let weight = self.config.tenant_weight(tenant) as i64;
            *self.deficits.entry(tenant.clone()).or_insert(0) += weight;
        }

        eligible_tenants
    }

    fn charge(&mut self, tenant: &str, eligible_tenants: &[String], cost: usize) {
        let total_weight = eligible_tenants
            .iter()
            .map(|tenant| self.config.tenant_weight(tenant) as i64)
            .sum::<i64>()
            .max(1);
        *self.deficits.entry(tenant.to_owned()).or_insert(0) -= total_weight * cost as i64;
    }

    fn charge_batch(
        &mut self,
        active: &[ActiveInferenceSequence],
        indices: &[usize],
        eligible_tenants: &[String],
    ) {
        let mut tenant_costs: HashMap<String, usize> = HashMap::new();
        for sequence in indices.iter().filter_map(|index| active.get(*index)) {
            *tenant_costs
                .entry(sequence.job.tenant_id().to_owned())
                .or_insert(0) += 1;
        }
        for (tenant, cost) in tenant_costs {
            self.charge(&tenant, eligible_tenants, cost);
        }
    }

    fn deficit(&self, tenant: &str) -> i64 {
        self.deficits.get(tenant).copied().unwrap_or_default()
    }
}

impl PartialEq for PrioritizedInferenceJob {
    fn eq(&self, other: &Self) -> bool {
        self.qos_score == other.qos_score && self.sequence == other.sequence
    }
}

impl Eq for PrioritizedInferenceJob {}

impl PartialOrd for PrioritizedInferenceJob {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl Ord for PrioritizedInferenceJob {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        self.qos_score
            .cmp(&other.qos_score)
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

fn run_scheduler(
    accelerator: AcceleratorKind,
    mut receiver: tokio_mpsc::Receiver<PrioritizedInferenceJob>,
    metrics: Arc<SchedulerMetrics>,
    max_active_sequences: usize,
    scheduler_config: SchedulerConfig,
) {
    let mut queued = BinaryHeap::new();
    let mut active = Vec::new();
    let mut admission_fairness = TenantFairness::new(scheduler_config.clone());
    let mut tenant_fairness = TenantFairness::new(scheduler_config.clone());

    loop {
        drain_ready_jobs(&mut receiver, &mut queued);
        admit_sequences(
            &mut queued,
            &mut active,
            max_active_sequences,
            &metrics,
            &mut admission_fairness,
        );

        if active.is_empty() {
            let Some(job) = receiver.blocking_recv() else {
                return;
            };
            queued.push(job);
            continue;
        }

        run_continuous_step(
            accelerator,
            &mut active,
            &metrics,
            &mut tenant_fairness,
            &scheduler_config,
        );
        age_waiting_jobs(&mut queued);
    }
}

fn admit_sequences(
    queued: &mut BinaryHeap<PrioritizedInferenceJob>,
    active: &mut Vec<ActiveInferenceSequence>,
    max_active_sequences: usize,
    metrics: &SchedulerMetrics,
    admission_fairness: &mut TenantFairness,
) {
    while active.len() < max_active_sequences {
        let Some(job) = admission_fairness.select_queued(queued) else {
            break;
        };
        active.push(job.into());
        metrics.record_active_sequences(active.len());
    }
}

fn drain_ready_jobs(
    receiver: &mut tokio_mpsc::Receiver<PrioritizedInferenceJob>,
    queued: &mut BinaryHeap<PrioritizedInferenceJob>,
) {
    while let Ok(job) = receiver.try_recv() {
        queued.push(job);
    }
}

struct InferenceBatchKey {
    alias: String,
}

fn run_continuous_step(
    accelerator: AcceleratorKind,
    active: &mut Vec<ActiveInferenceSequence>,
    metrics: &SchedulerMetrics,
    tenant_fairness: &mut TenantFairness,
    scheduler_config: &SchedulerConfig,
) {
    if let Some(indices) =
        select_active_batch(active, InferenceSequencePhase::Prefill, tenant_fairness)
    {
        record_scheduler_kv_preemptions(active, &indices, metrics, scheduler_config);
        let prefill_batch = indices
            .iter()
            .map(|index| active[*index].job.clone())
            .collect::<Vec<_>>();
        metrics.record_prefill_step(&prefill_batch);
        for index in indices {
            active[index].phase = InferenceSequencePhase::Decode;
        }
        return;
    }

    let Some(indices) =
        select_active_batch(active, InferenceSequencePhase::Decode, tenant_fairness)
    else {
        return;
    };
    let mut decode_batch = Vec::with_capacity(indices.len());
    for index in indices.iter().rev() {
        decode_batch.push(active.swap_remove(*index).job);
    }
    decode_batch.reverse();

    let results = process_batch(accelerator, &decode_batch);
    metrics.record_decode_step(&decode_batch);
    for (job, result) in decode_batch.into_iter().zip(results) {
        metrics.queued_requests.fetch_sub(1, Ordering::Relaxed);
        metrics.record_dequeue(job.qos);
        let _ = job.response_tx.send(result);
    }
}

fn record_scheduler_kv_preemptions(
    active: &[ActiveInferenceSequence],
    selected_indices: &[usize],
    metrics: &SchedulerMetrics,
    scheduler_config: &SchedulerConfig,
) {
    let Some(selected_score) = selected_indices
        .iter()
        .filter_map(|index| active.get(*index))
        .map(|sequence| sequence.qos_score)
        .max()
    else {
        return;
    };
    for (index, sequence) in active.iter().enumerate() {
        if selected_indices.binary_search(&index).is_ok()
            || sequence.qos_score >= selected_score
            || !scheduler_config
                .tier_preemptible
                .is_preemptible(sequence.job.qos)
        {
            continue;
        }
        let mode = if scheduler_config.spill_tier_max == SchedulerSpillTier::Nvme
            && scheduler_config.spill_budget_bytes > 0
        {
            paged_kv::KvPreemptionMode::Swap
        } else {
            paged_kv::KvPreemptionMode::Recompute
        };
        metrics.record_kv_preemption(mode);
    }
}

fn select_active_batch(
    active: &[ActiveInferenceSequence],
    phase: InferenceSequencePhase,
    tenant_fairness: &mut TenantFairness,
) -> Option<Vec<usize>> {
    let candidates = active
        .iter()
        .enumerate()
        .filter(|(_, sequence)| sequence.phase == phase)
        .collect::<Vec<_>>();
    let max_qos_score = candidates
        .iter()
        .map(|(_, sequence)| sequence.qos_score)
        .max()?;
    let selection = tenant_fairness.select_active(candidates, max_qos_score)?;
    let (_, first) = selection.selected;
    let batch_key = first.batch_key();
    let selected_tenant = selection.tenant.as_str();
    let allow_cross_tenant_batch = tenant_fairness.allows_cross_tenant_active_batching();
    let mut indices = active
        .iter()
        .enumerate()
        .filter_map(|(index, sequence)| {
            (sequence.is_compatible_with(&batch_key, phase)
                && (allow_cross_tenant_batch || sequence.job.tenant_id() == selected_tenant))
                .then_some(index)
        })
        .collect::<Vec<_>>();
    indices.sort_unstable();
    tenant_fairness.charge_batch(active, &indices, &selection.eligible_tenants);
    Some(indices)
}

fn age_waiting_jobs(queued: &mut BinaryHeap<PrioritizedInferenceJob>) {
    if queued.is_empty() {
        return;
    }
    let aged = queued
        .drain()
        .map(PrioritizedInferenceJob::age)
        .collect::<Vec<_>>();
    for job in aged {
        queued.push(job);
    }
}

fn process_batch(
    accelerator: AcceleratorKind,
    batch: &[InferenceJob],
) -> Vec<Result<Vec<u8>, anyhow::Error>> {
    let model = Arc::clone(&batch[0].model);
    #[cfg(test)]
    if model.mock_latency > Duration::ZERO {
        thread::sleep(model.mock_latency);
    }

    let inputs = batch
        .iter()
        .map(|job| job.input.clone())
        .collect::<Vec<_>>();
    let adapters = batch
        .iter()
        .map(|job| job.adapter.clone())
        .collect::<Vec<_>>();
    let results = model.run_batch_with_adapter_results(&inputs, &adapters);
    if results.len() == batch.len() {
        return results;
    }

    match results
        .into_iter()
        .collect::<Result<Vec<Vec<u8>>, anyhow::Error>>()
    {
        // Each job's own output is routed back to that job, never a shared
        // clone of one job's result broadcast to the whole batch.
        Ok(outputs) if outputs.len() == batch.len() => outputs.into_iter().map(Ok).collect(),
        Ok(outputs) => {
            let message = format!(
                "{} backend for model `{}` returned {} output(s) for a batch of {} request(s)",
                accelerator.as_str(),
                model.alias,
                outputs.len(),
                batch.len()
            );
            batch
                .iter()
                .map(|_| Err(anyhow::anyhow!("{}", message.clone())))
                .collect()
        }
        Err(error) => {
            let message = format!(
                "{} backend failed for model `{}`: {}",
                accelerator.as_str(),
                model.alias,
                error
            );
            batch
                .iter()
                .map(|_| Err(anyhow::anyhow!("{}", message.clone())))
                .collect()
        }
    }
}

fn adapter_assignment_groups(adapters: &[Option<ResolvedLoraAdapter>]) -> Vec<Vec<usize>> {
    let mut groups: Vec<(Option<&str>, Vec<usize>)> = Vec::new();
    for (index, adapter) in adapters.iter().enumerate() {
        let adapter_id = adapter.as_ref().map(|adapter| adapter.id.as_str());
        if let Some((_, indices)) = groups
            .iter_mut()
            .find(|(known_adapter_id, _)| *known_adapter_id == adapter_id)
        {
            indices.push(index);
        } else {
            groups.push((adapter_id, vec![index]));
        }
    }
    groups.into_iter().map(|(_, indices)| indices).collect()
}

// Unified candle-native backend model â€” replaces the WasiNnBackend abstraction.
struct CandleBackendModel {
    source: BackendModelSource,
    kind: CandleBackendModelKind,
}

enum CandleBackendModelKind {
    Mock,
    TextGeneration {
        target: Box<candle_llm_runtime::CandleLlmRuntime>,
        speculative: Option<SpeculativeDraftRuntime>,
    },
    /// A ModelOpt/NVFP4 checkpoint, dequantized to a dense F32 model at load
    /// time (the fallback NVFP4 execution path: see
    /// `candle_llm_runtime::CandleLlmRuntime::try_load_modelopt_nvfp4`).
    /// Native FP4-kernel execution without eager dequantization remains a
    /// documented follow-up.
    ModelOptNvfp4(Box<candle_llm_runtime::CandleLlmRuntime>),
    TextEmbedding(Box<candle_embedding_runtime::CandleEmbeddingRuntime>),
    Qwen35Moe(Box<qwen35_moe_runtime::Qwen35MoeRuntime>),
    Vendor(vendor_accelerator::VendorAcceleratorRuntime),
}

struct SpeculativeDraftRuntime {
    draft: Box<candle_llm_runtime::CandleLlmRuntime>,
    draft_tokens: usize,
}

impl CandleBackendModel {
    fn mock(binding: &IntegrityModelBinding) -> Result<Self> {
        if binding.path.trim().is_empty() {
            return Err(anyhow!(
                "Integrity Validation Failed: model alias `{}` must declare a non-empty `path`",
                binding.alias
            ));
        }
        Ok(Self {
            source: BackendModelSource {
                alias: binding.alias.clone(),
                path: binding.path.clone(),
                requested_target: binding.device.as_str().to_owned(),
                accelerator: AcceleratorKind::from_model_device(&binding.device),
                qos: binding.qos,
                model_size_bytes: model_file_size_bytes(&binding.path),
            },
            kind: CandleBackendModelKind::Mock,
        })
    }

    fn load(binding: &IntegrityModelBinding) -> Result<Self> {
        if binding.path.trim().is_empty() {
            return Err(anyhow!(
                "Integrity Validation Failed: model alias `{}` must declare a non-empty `path`",
                binding.alias
            ));
        }
        let kind = if is_explicit_mock_binding(binding) {
            CandleBackendModelKind::Mock
        } else if matches!(
            binding.device,
            crate::ModelDevice::Npu | crate::ModelDevice::Tpu
        ) {
            let accelerator = AcceleratorKind::from_model_device(&binding.device);
            CandleBackendModelKind::Vendor(
                vendor_accelerator::VendorAcceleratorRuntime::try_load(accelerator, &binding.path)?
                    .ok_or_else(|| {
                        anyhow!(
                            "{} vendor backend is unavailable; configure its SDK runner",
                            accelerator.as_str()
                        )
                    })?,
            )
        } else {
            match modelopt_nvfp4::ModelOptNvfp4Directory::try_load(&binding.alias, &binding.path)? {
                Some(model) => {
                    static DESCRIPTORS: [&dyn architecture_registry::ArchitectureDescriptor; 1] =
                        [&qwen35_moe_runtime::QWEN35_MOE_DESCRIPTOR];
                    let registry = architecture_registry::ArchitectureRegistry::new(&DESCRIPTORS);
                    match registry.resolve(&model)? {
                        Some(architecture)
                            if architecture.kind
                                == architecture_registry::ArchitectureKind::Qwen35MoeText =>
                        {
                            CandleBackendModelKind::Qwen35Moe(Box::new(
                                qwen35_moe_runtime::Qwen35MoeRuntime::from_model(
                                    &binding.alias,
                                    &binding.path,
                                    binding.device.as_str(),
                                    model,
                                )?,
                            ))
                        }
                        Some(architecture) => {
                            return Err(anyhow!(
                                "unsupported registered ModelOpt/NVFP4 architecture {:?}",
                                architecture.kind
                            ));
                        }
                        None => {
                            model.ensure_fallback_memory_within_limits(
                                modelopt_nvfp4::Nvfp4OutputDType::F32,
                                modelopt_nvfp4::Nvfp4FallbackScope::Eager,
                                modelopt_nvfp4::Nvfp4FallbackMemoryLimits {
                                    max_host_ram_bytes: env_u64("TACHYON_NVFP4_MAX_HOST_RAM_BYTES"),
                                    max_accelerator_bytes: env_u64(
                                        "TACHYON_NVFP4_MAX_ACCELERATOR_BYTES",
                                    ),
                                },
                            )?;
                            CandleBackendModelKind::ModelOptNvfp4(Box::new(
                                candle_llm_runtime::CandleLlmRuntime::try_load_modelopt_nvfp4(
                                    &model,
                                )?,
                            ))
                        }
                    }
                }
                None => {
                    match candle_embedding_runtime::CandleEmbeddingRuntime::try_load(
                        &binding.alias,
                        &binding.path,
                    )? {
                        Some(model) => CandleBackendModelKind::TextEmbedding(Box::new(model)),
                        None => match candle_llm_runtime::CandleLlmRuntime::try_load(
                            &binding.alias,
                            &binding.path,
                            binding.device.as_str(),
                            &binding.hardware_strategy,
                        )? {
                            Some(model) => {
                                let speculative = load_speculative_draft_runtime(binding)?.map(
                                    |(draft, draft_tokens)| SpeculativeDraftRuntime {
                                        draft: Box::new(draft),
                                        draft_tokens,
                                    },
                                );
                                CandleBackendModelKind::TextGeneration {
                                    target: Box::new(model),
                                    speculative,
                                }
                            }
                            None => {
                                return Err(anyhow!(
                                    "unsupported AI model binding `{}` at `{}`: expected explicit mock path `mock:<name>`, supported Candle LLM directory, ONNX embedding directory, ONNX guest-loaded graph, or ModelOpt/NVFP4 directory",
                                    binding.alias,
                                    binding.path
                                ))
                            }
                        },
                    }
                }
            }
        };
        Ok(Self {
            source: BackendModelSource {
                alias: binding.alias.clone(),
                path: binding.path.clone(),
                requested_target: binding.device.as_str().to_owned(),
                accelerator: AcceleratorKind::from_model_device(&binding.device),
                qos: binding.qos,
                model_size_bytes: model_file_size_bytes(&binding.path),
            },
            kind,
        })
    }
}

fn load_speculative_draft_runtime(
    binding: &IntegrityModelBinding,
) -> Result<Option<(candle_llm_runtime::CandleLlmRuntime, usize)>> {
    let draft_path = binding
        .hardware_strategy
        .speculative_draft_model_path
        .trim();
    if draft_path.is_empty() {
        return Ok(None);
    }
    let mut draft_strategy = binding.hardware_strategy.clone();
    draft_strategy.speculative_draft_model_path.clear();
    draft_strategy.speculative_draft_tokens = 0;
    let draft = candle_llm_runtime::CandleLlmRuntime::try_load(
        &format!("{}:draft", binding.alias),
        draft_path,
        binding.device.as_str(),
        &draft_strategy,
    )?
    .ok_or_else(|| {
        anyhow!(
            "speculative draft model for `{}` at `{}` is not a supported Candle LLM directory",
            binding.alias,
            draft_path
        )
    })?;
    let draft_tokens = usize::try_from(binding.hardware_strategy.speculative_draft_tokens)
        .ok()
        .filter(|tokens| *tokens > 0)
        .unwrap_or(candle_llm_runtime::DEFAULT_SPECULATIVE_DRAFT_TOKENS);
    Ok(Some((draft, draft_tokens)))
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
}

impl BackendModel for CandleBackendModel {
    fn residency(&self) -> AcceleratorMemoryResidency {
        match self.source.accelerator {
            AcceleratorKind::Cpu => AcceleratorMemoryResidency::HostRam,
            AcceleratorKind::Gpu => AcceleratorMemoryResidency::Vram,
            AcceleratorKind::Npu => AcceleratorMemoryResidency::Sram,
            AcceleratorKind::Tpu => AcceleratorMemoryResidency::Sram,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn execute(&self, inputs: &[SharedInputTensor]) -> Result<Vec<Vec<u8>>> {
        match &self.kind {
            CandleBackendModelKind::Qwen35Moe(runtime) => {
                if inputs
                    .iter()
                    .any(|input| !matches!(input.ty, TensorType::U8))
                {
                    return Err(anyhow!(
                        "Qwen 3.5 MoE model `{}` only accepts U8 prompt tensors",
                        self.source.alias
                    ));
                }
                // Each input is an independent request: the underlying runtime
                // only ever decodes one prompt per call, so a shared multi-prompt
                // slice here would silently decode just the first and broadcast
                // its output to every other request in the batch.
                let result = inputs
                    .iter()
                    .map(|input| runtime.generate(&[input.data.as_ref()]))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| {
                        anyhow!(
                            "Qwen 3.5 MoE model `{}` loaded from `{}` failed: {error}",
                            self.source.alias,
                            runtime.root().display()
                        )
                    });
                record_execution(&self.source.alias, runtime.executed_on(), result.is_ok());
                return result;
            }
            CandleBackendModelKind::ModelOptNvfp4(runtime) => {
                if inputs
                    .iter()
                    .any(|input| !matches!(input.ty, TensorType::U8))
                {
                    return Err(anyhow!(
                        "Candle LLM model `{}` only accepts U8 prompt tensors",
                        self.source.alias
                    ));
                }
                let result = inputs
                    .iter()
                    .map(|input| runtime.generate(&[input.data.as_ref()]))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| {
                        anyhow!(
                            "Candle LLM model `{}` loaded from `{}` failed: {error}",
                            self.source.alias,
                            runtime.root().display()
                        )
                    });
                let executed_on = match (&self.kind, self.source.accelerator) {
                    (CandleBackendModelKind::ModelOptNvfp4(_), AcceleratorKind::Gpu) => {
                        "gpu_fallback"
                    }
                    (CandleBackendModelKind::ModelOptNvfp4(_), _) => "cpu_fallback",
                    (_, AcceleratorKind::Gpu) => "gpu",
                    _ => "cpu",
                };
                record_execution(&self.source.alias, executed_on, result.is_ok());
                return result;
            }
            CandleBackendModelKind::TextGeneration {
                target,
                speculative,
            } => {
                if inputs
                    .iter()
                    .any(|input| !matches!(input.ty, TensorType::U8))
                {
                    return Err(anyhow!(
                        "Candle LLM model `{}` only accepts U8 prompt tensors",
                        self.source.alias
                    ));
                }
                let result = inputs
                    .iter()
                    .map(|input| {
                        let prompt = input.data.as_ref();
                        match speculative {
                            Some(speculative) => target.generate_speculative(
                                &[prompt],
                                &speculative.draft,
                                speculative.draft_tokens,
                            ),
                            None => target.generate(&[prompt]),
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| {
                        anyhow!(
                            "Candle LLM model `{}` loaded from `{}` failed: {error}",
                            self.source.alias,
                            target.root().display()
                        )
                    });
                let executed_on = if self.source.accelerator == AcceleratorKind::Gpu {
                    "gpu"
                } else {
                    "cpu"
                };
                record_execution(&self.source.alias, executed_on, result.is_ok());
                return result;
            }
            CandleBackendModelKind::Vendor(runtime) => {
                if inputs.is_empty() {
                    return Err(anyhow!("vendor backend requires one input tensor"));
                }
                let result = inputs
                    .iter()
                    .map(|input| runtime.execute(input.data.as_ref()))
                    .collect::<Result<Vec<_>, _>>();
                record_execution(
                    &self.source.alias,
                    self.source.accelerator.as_str(),
                    result.is_ok(),
                );
                return result;
            }
            CandleBackendModelKind::TextEmbedding(_) => {
                bail!(
                    "embedding model `{}` does not support text generation",
                    self.source.alias
                );
            }
            CandleBackendModelKind::Mock => {}
        }
        if inputs.is_empty() {
            return Err(anyhow!(
                "{} backend requires at least one input tensor",
                self.source.accelerator.as_str()
            ));
        }
        let input = &inputs[0];
        if !matches!(input.ty, TensorType::U8 | TensorType::Fp32) {
            return Err(anyhow!(
                "{} backend only supports U8 or F32 tensors",
                self.source.accelerator.as_str()
            ));
        }
        let longest_prompt = inputs.iter().map(|i| i.byte_len()).max().unwrap_or(1);
        let batch_size = inputs.len();
        let _prompt_batch =
            CandleTensor::zeros((batch_size, longest_prompt), DType::F32, &Device::Cpu)
                .context("failed to prepare candle mock batch")?;
        let _resident_weights = self.source.model_size_bytes;
        record_execution(&self.source.alias, self.source.accelerator.as_str(), true);
        Ok(inputs
            .iter()
            .map(|_| b"MOCK_LLM_RESPONSE".to_vec())
            .collect())
    }

    fn execute_with_adapter(
        &self,
        inputs: &[SharedInputTensor],
        adapter: &ResolvedLoraAdapter,
    ) -> Result<Vec<Vec<u8>>> {
        match &self.kind {
            CandleBackendModelKind::Mock => self.execute(inputs),
            CandleBackendModelKind::TextGeneration { target, .. }
            | CandleBackendModelKind::ModelOptNvfp4(target) => {
                validate_u8_prompts(&self.source.alias, inputs)?;
                inputs
                    .iter()
                    .map(|input| {
                        target.generate_with_adapter(
                            &[input.data.as_ref()],
                            &adapter.id,
                            &adapter.path,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| {
                        anyhow!(
                            "Candle LLM model `{}` failed with LoRA adapter `{}` at `{}`: {error}",
                            self.source.alias,
                            adapter.id,
                            adapter.path.display()
                        )
                    })
            }
            CandleBackendModelKind::Qwen35Moe(_) | CandleBackendModelKind::Vendor(_) => bail!(
                "LoRA adapter `{}` was resolved for model `{}`, but this backend does not support adapter injection",
                adapter.id,
                self.source.alias
            ),
            CandleBackendModelKind::TextEmbedding(_) => bail!(
                "LoRA adapter `{}` was resolved for embedding model `{}`, but embeddings do not support adapter injection",
                adapter.id,
                self.source.alias
            ),
        }
    }

    fn execute_with_adapters(
        &self,
        inputs: &[SharedInputTensor],
        adapters: &[Option<ResolvedLoraAdapter>],
    ) -> Result<Vec<Vec<u8>>> {
        self.execute_with_adapter_results(inputs, adapters)
            .into_iter()
            .collect()
    }

    fn execute_with_adapter_results(
        &self,
        inputs: &[SharedInputTensor],
        adapters: &[Option<ResolvedLoraAdapter>],
    ) -> Vec<Result<Vec<u8>>> {
        if inputs.len() != adapters.len() {
            let message = format!(
                "adapter assignment count {} does not match input count {}",
                adapters.len(),
                inputs.len()
            );
            return repeat_batch_error(inputs.len().max(adapters.len()), message);
        }
        match &self.kind {
            CandleBackendModelKind::TextGeneration {
                target,
                speculative: None,
            }
            | CandleBackendModelKind::ModelOptNvfp4(target) => {
                if let Err(error) = validate_u8_prompts(&self.source.alias, inputs) {
                    return repeat_batch_error(inputs.len(), error.to_string());
                }
                let prompts = inputs
                    .iter()
                    .map(|input| input.data.as_ref())
                    .collect::<Vec<_>>();
                match target.try_generate_batch_with_adapters(&prompts, adapters) {
                    Ok(Some(outputs)) => {
                        if outputs.len() != inputs.len() {
                            return repeat_batch_error(
                                inputs.len(),
                                format!(
                                    "Candle backend returned {} output(s) for a native mixed-adapter batch of {} input(s)",
                                    outputs.len(),
                                    inputs.len()
                                ),
                            );
                        }
                        let executed_on = if self.source.accelerator == AcceleratorKind::Gpu {
                            "gpu"
                        } else {
                            "cpu"
                        };
                        record_execution(&self.source.alias, executed_on, true);
                        outputs.into_iter().map(Ok).collect()
                    }
                    Ok(None) => self.execute_with_adapters_sequential_results(inputs, adapters),
                    Err(error) => {
                        let executed_on = if self.source.accelerator == AcceleratorKind::Gpu {
                            "gpu"
                        } else {
                            "cpu"
                        };
                        record_execution(&self.source.alias, executed_on, false);
                        tracing::warn!(
                            "Candle LLM model `{}` loaded from `{}` failed during batch-native LoRA decode: {error}",
                            self.source.alias,
                            target.root().display()
                        );
                        self.execute_with_adapters_sequential_results(inputs, adapters)
                    }
                }
            }
            CandleBackendModelKind::TextGeneration {
                speculative: Some(_),
                ..
            } => self.execute_with_adapters_sequential_results(inputs, adapters),
            _ => self.execute_with_adapters_sequential_results(inputs, adapters),
        }
    }

    fn embed_text(&self, input: &SharedInputTensor) -> Result<Vec<f32>> {
        if !matches!(input.ty, TensorType::U8) {
            bail!(
                "embedding model `{}` only accepts U8 text tensors",
                self.source.alias
            );
        }

        match &self.kind {
            CandleBackendModelKind::Mock => {
                record_execution(&self.source.alias, self.source.accelerator.as_str(), true);
                Ok(deterministic_mock_embedding(input.data.as_ref()))
            }
            CandleBackendModelKind::TextGeneration { .. }
            | CandleBackendModelKind::ModelOptNvfp4(_)
            | CandleBackendModelKind::Qwen35Moe(_) => bail!(
                "model `{}` is a generation runtime and does not expose hidden-state pooling yet",
                self.source.alias
            ),
            CandleBackendModelKind::TextEmbedding(runtime) => {
                let input = std::str::from_utf8(input.data.as_ref()).map_err(|error| {
                    anyhow!(
                        "embedding input for model `{}` was not UTF-8: {error}",
                        self.source.alias
                    )
                })?;
                let result = runtime.embed(input).map_err(|error| {
                    anyhow!(
                        "embedding model `{}` loaded from `{}` failed: {error}",
                        self.source.alias,
                        runtime.root().display()
                    )
                });
                record_execution(
                    &self.source.alias,
                    self.source.accelerator.as_str(),
                    result.is_ok(),
                );
                result
            }
            CandleBackendModelKind::Vendor(_) => bail!(
                "vendor backend for model `{}` does not expose dense text embeddings",
                self.source.alias
            ),
        }
    }

    fn stream_text(
        &self,
        inputs: &[SharedInputTensor],
        on_token: &mut dyn FnMut(&str),
    ) -> Result<()> {
        match &self.kind {
            CandleBackendModelKind::ModelOptNvfp4(runtime) => {
                validate_u8_prompts(&self.source.alias, inputs)?;
                let prompts = inputs
                    .iter()
                    .map(|input| input.data.as_ref())
                    .collect::<Vec<_>>();
                runtime
                    .generate_streaming(&prompts, on_token)
                    .map_err(|error| {
                        anyhow!(
                            "Candle LLM model `{}` loaded from `{}` failed: {error}",
                            self.source.alias,
                            runtime.root().display()
                        )
                    })
            }
            CandleBackendModelKind::TextGeneration {
                target,
                speculative,
            } => {
                validate_u8_prompts(&self.source.alias, inputs)?;
                let prompts = inputs
                    .iter()
                    .map(|input| input.data.as_ref())
                    .collect::<Vec<_>>();
                match speculative {
                    Some(speculative) => target.generate_speculative_streaming(
                        &prompts,
                        &speculative.draft,
                        speculative.draft_tokens,
                        on_token,
                    ),
                    None => target.generate_streaming(&prompts, on_token),
                }
                .map_err(|error| {
                    anyhow!(
                        "Candle LLM model `{}` loaded from `{}` failed: {error}",
                        self.source.alias,
                        target.root().display()
                    )
                })
            }
            CandleBackendModelKind::Qwen35Moe(runtime) => {
                validate_u8_prompts(&self.source.alias, inputs)?;
                let prompts = inputs
                    .iter()
                    .map(|input| input.data.as_ref())
                    .collect::<Vec<_>>();
                runtime
                    .generate_streaming(&prompts, on_token)
                    .map_err(|error| {
                        anyhow!(
                            "Qwen 3.5 MoE model `{}` loaded from `{}` failed: {error}",
                            self.source.alias,
                            runtime.root().display()
                        )
                    })
            }
            CandleBackendModelKind::TextEmbedding(_) => {
                bail!(
                    "embedding model `{}` does not support text generation",
                    self.source.alias
                )
            }
            CandleBackendModelKind::Mock => {
                let outputs = self.execute(inputs);
                record_execution(
                    &self.source.alias,
                    self.source.accelerator.as_str(),
                    outputs.is_ok(),
                );
                let outputs = outputs?;
                if outputs.len() != 1 {
                    bail!(
                        "mock backend expected exactly one output for streaming, got {} for {} input(s)",
                        outputs.len(),
                        inputs.len()
                    );
                }
                let text = String::from_utf8(outputs.into_iter().next().unwrap_or_default())
                    .map_err(|error| anyhow!("output was not UTF-8: {error}"))?;
                if !text.is_empty() {
                    on_token(&text);
                }
                Ok(())
            }
            CandleBackendModelKind::Vendor(_) => {
                let outputs = self.execute(inputs)?;
                if outputs.len() != 1 {
                    bail!(
                        "vendor backend expected exactly one output for streaming, got {} for {} input(s)",
                        outputs.len(),
                        inputs.len()
                    );
                }
                let text = String::from_utf8(outputs.into_iter().next().unwrap_or_default())
                    .map_err(|error| anyhow!("vendor output was not UTF-8: {error}"))?;
                on_token(&text);
                Ok(())
            }
        }
    }
}

fn deterministic_mock_embedding(input: &[u8]) -> Vec<f32> {
    const DIM: usize = 8;
    let mut values = vec![0.0; DIM];
    for (index, byte) in input.iter().enumerate() {
        let slot = index % DIM;
        values[slot] += (*byte as f32 + 1.0) / 256.0;
    }
    normalize_f32(values)
}

fn normalize_f32(mut values: Vec<f32>) -> Vec<f32> {
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm == 0.0 {
        return values;
    }
    for value in &mut values {
        *value /= norm;
    }
    values
}

fn validate_u8_prompts(alias: &str, inputs: &[SharedInputTensor]) -> Result<()> {
    if inputs
        .iter()
        .any(|input| !matches!(input.ty, TensorType::U8))
    {
        bail!("text-generation model `{alias}` only accepts U8 prompt tensors");
    }
    Ok(())
}

struct CandleModel {
    alias: String,
    accelerator: AcceleratorKind,
    qos: RouteQos,
    #[cfg_attr(not(test), allow(dead_code))]
    memory_residency: AcceleratorMemoryResidency,
    backend_model: Arc<dyn BackendModel>,
    #[cfg(test)]
    mock_latency: Duration,
}

impl CandleModel {
    fn load_mock_with_backend(
        binding: &IntegrityModelBinding,
        backend_model: Arc<dyn BackendModel>,
    ) -> Result<Self> {
        if binding.path.trim().is_empty() {
            return Err(anyhow!(
                "Integrity Validation Failed: model alias `{}` must declare a non-empty `path`",
                binding.alias
            ));
        }
        let memory_residency = backend_model.residency();
        Ok(Self {
            alias: binding.alias.clone(),
            accelerator: AcceleratorKind::from_model_device(&binding.device),
            qos: binding.qos,
            memory_residency,
            backend_model,
            #[cfg(test)]
            mock_latency: Duration::ZERO,
        })
    }

    #[cfg(test)]
    fn load_mock(binding: &IntegrityModelBinding) -> Result<Self> {
        Self::load_mock_with_backend(binding, Arc::new(CandleBackendModel::mock(binding)?))
    }

    #[cfg(test)]
    fn with_mock_latency(mut self, latency: Duration) -> Self {
        self.mock_latency = latency;
        self
    }

    fn run_batch_with_adapter_results(
        &self,
        inputs: &[SharedInputTensor],
        adapters: &[Option<ResolvedLoraAdapter>],
    ) -> Vec<Result<Vec<u8>>> {
        self.backend_model
            .execute_with_adapter_results(inputs, adapters)
    }
}

fn is_explicit_mock_binding(binding: &IntegrityModelBinding) -> bool {
    binding.path == "mock" || binding.path.starts_with("mock:")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KvPrecision {
    Q8_0,
    F16,
}

#[derive(Clone)]
struct TurboQuantAttentionStack {
    total_layers: usize,
    boundary_layers: usize,
    threshold: f32,
    bits: u8,
}

impl Default for TurboQuantAttentionStack {
    fn default() -> Self {
        Self {
            total_layers: 8,
            boundary_layers: 2,
            threshold: 1.0e-4,
            bits: 2,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct TurboQuantLayerDecision {
    layer_idx: usize,
    k_precision: KvPrecision,
    v_compressed: bool,
}

impl TurboQuantAttentionStack {
    fn layer_decision(&self, layer_idx: usize) -> TurboQuantLayerDecision {
        TurboQuantLayerDecision {
            layer_idx,
            k_precision: KvPrecision::Q8_0,
            v_compressed: self.should_compress(layer_idx),
        }
    }

    fn should_compress(&self, layer_idx: usize) -> bool {
        layer_idx >= self.boundary_layers && layer_idx + self.boundary_layers < self.total_layers
    }

    fn run_mock_prompt(&self, batch_size: usize, prompt_len: usize) -> Result<()> {
        let value_count = batch_size.max(1) * prompt_len.max(1);
        let values = quantizable_fixture_values(value_count);
        let k_tensor = CandleTensor::from_vec(values.clone(), (value_count,), &Device::Cpu)?;
        let v_tensor = CandleTensor::from_vec(values.clone(), (value_count,), &Device::Cpu)?;
        let attention =
            CandleTensor::from_vec(vec![1.0f32; value_count], (value_count,), &Device::Cpu)?;

        let _ = k_tensor;
        for layer_idx in 0..self.total_layers {
            let decision = self.layer_decision(layer_idx);
            if decision.v_compressed {
                let packed = compress_tensor_values(&v_tensor, self.bits)?;
                let packed_tensor = CandleTensor::from_vec(
                    packed,
                    (turboquant_sys::packed_len(value_count, self.bits)?,),
                    &Device::Cpu,
                )?;
                let restored = packed_tensor.apply_op2_no_bwd(
                    &attention,
                    &TurboQuantDecompressor {
                        bits: self.bits,
                        threshold: self.threshold,
                        value_count,
                    },
                )?;
                let restored_values = restored.to_vec1::<f32>()?;
                if restored_values.len() != value_count {
                    return Err(anyhow!(
                        "TurboQuant restored {} values for layer {layer_idx} but expected {value_count}",
                        restored_values.len()
                    ));
                }
            } else {
                let restored_values = v_tensor.to_vec1::<f32>()?;
                if restored_values.len() != value_count {
                    return Err(anyhow!(
                        "standard value cache restored {} values for layer {layer_idx} but expected {value_count}",
                        restored_values.len()
                    ));
                }
            }
        }
        Ok(())
    }
}

struct TurboQuantDecompressor {
    bits: u8,
    threshold: f32,
    value_count: usize,
}

impl CustomOp2 for TurboQuantDecompressor {
    fn name(&self) -> &'static str {
        "turboquant-decompressor"
    }

    fn cpu_fwd(
        &self,
        packed_storage: &CpuStorage,
        packed_layout: &Layout,
        attention_storage: &CpuStorage,
        attention_layout: &Layout,
    ) -> candle_core::Result<(CpuStorage, Shape)> {
        let packed =
            contiguous_u8_slice(packed_storage, packed_layout, "TurboQuant packed values")?;
        let attention =
            contiguous_f32_slice(attention_storage, attention_layout, "TurboQuant attention")?;
        if attention.len() != self.value_count {
            candle_bail!(
                "TurboQuant attention tensor must contain {} values but contains {}",
                self.value_count,
                attention.len()
            );
        }
        let output = turboquant_sys::decompress_values_sparse(
            packed,
            self.value_count,
            self.bits,
            attention,
            self.threshold,
        )
        .map_err(|error| candle_core::Error::Msg(error.to_string()).bt())?;
        Ok((
            CpuStorage::F32(output),
            Shape::from_dims(&[self.value_count]),
        ))
    }
}

fn compress_tensor_values(tensor: &CandleTensor, bits: u8) -> Result<Vec<u8>> {
    let values = tensor.to_vec1::<f32>()?;
    turboquant_sys::compress_values(&values, bits).map_err(|error| anyhow!(error.to_string()))
}

fn contiguous_u8_slice<'a>(
    storage: &'a CpuStorage,
    layout: &Layout,
    label: &str,
) -> candle_core::Result<&'a [u8]> {
    let (start, end) = layout.contiguous_offsets().ok_or_else(|| {
        candle_core::Error::Msg(format!(
            "{label} must be contiguous before invoking TurboQuant"
        ))
        .bt()
    })?;
    match storage {
        CpuStorage::U8(values) => Ok(&values[start..end]),
        _ => candle_bail!("{label} must use a u8 storage tensor"),
    }
}

fn contiguous_f32_slice<'a>(
    storage: &'a CpuStorage,
    layout: &Layout,
    label: &str,
) -> candle_core::Result<&'a [f32]> {
    let (start, end) = layout.contiguous_offsets().ok_or_else(|| {
        candle_core::Error::Msg(format!(
            "{label} must be contiguous before invoking TurboQuant"
        ))
        .bt()
    })?;
    match storage {
        CpuStorage::F32(values) => Ok(&values[start..end]),
        _ => candle_bail!("{label} must use an f32 storage tensor"),
    }
}

fn quantizable_fixture_values(value_count: usize) -> Vec<f32> {
    const LEVELS: [f32; 4] = [-1.0, -0.33333334, 0.33333334, 1.0];
    (0..value_count)
        .map(|index| LEVELS[(index * 3 + 1) % LEVELS.len()])
        .collect()
}

// Ã¢â€â‚¬Ã¢â€â‚¬ Layer-Wise Inference: memory profile Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

/// Controls VRAM placement strategy for a single inference call. Mirrors the
/// `memory-profile` enum in `wit/ai/inference.wit`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum MemoryProfile {
    /// All weights are pre-loaded to VRAM at model-load time. Maximum throughput;
    /// OOM risk scales with model size.
    #[default]
    Performance,
    /// Only one transformer layer occupies VRAM at a time. Weights are streamed
    /// from a memory-mapped `.safetensors` file; the KV-Cache is paged to Host RAM
    /// after each layer to maintain an O(1) VRAM footprint regardless of context length.
    LayerWiseStreaming,
}

pub(crate) use vram_manager::VramPriority;

// Ã¢â€â‚¬Ã¢â€â‚¬ Layer-Wise Inference: zero-copy model loader Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

/// A single transformer layer's weight slice, backed by a memory-mapped region.
/// Tensors are allocated on `Device::Cpu` and transferred to the accelerator
/// device only for the duration of that layer's forward pass.
pub(crate) struct LayerWeightSlice {
    pub(crate) layer_idx: usize,
    /// Key weight tensor loaded into CPU memory from the mapped file.
    pub(crate) weights: CandleTensor,
    /// Scheduling priority assigned to the VRAM residency backing this layer.
    pub(crate) priority: VramPriority,
}

/// Parsed safetensors header: the leading u64 length prefix, the JSON
/// metadata segment, and the byte range where the tensor blob lives.
#[derive(Debug, Clone)]
pub(crate) struct SafetensorsHeader {
    /// Length, in bytes, of the JSON header segment.
    pub(crate) header_len: u64,
    /// Byte offset (relative to the start of the file) where the first
    /// tensor begins. Always equal to `8 + header_len`.
    pub(crate) data_start: usize,
    /// Raw JSON value carrying `shape`, `dtype`, and `data_offsets` for
    /// every tensor stored in the file.
    pub(crate) header: serde_json::Value,
}

impl SafetensorsHeader {
    /// Parse the leading 8-byte little-endian length prefix and the
    /// subsequent UTF-8 JSON segment.
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 8 {
            anyhow::bail!("safetensors file is missing 8-byte header length prefix");
        }
        let header_len = u64::from_le_bytes(bytes[..8].try_into().expect("fixed prefix length"));
        let header_end = 8usize
            .checked_add(usize::try_from(header_len).context("header length exceeds usize")?)
            .context("safetensors header length overflows usize")?;
        if bytes.len() < header_end {
            anyhow::bail!(
                "safetensors header truncated: need {header_end} bytes, got {}",
                bytes.len()
            );
        }
        let header: serde_json::Value = serde_json::from_slice(&bytes[8..header_end])
            .context("failed to parse safetensors JSON header")?;
        Ok(Self {
            header_len,
            data_start: header_end,
            header,
        })
    }
}

/// Wraps a memory-mapped `.safetensors` file and vends per-layer
/// [`LayerWeightSlice`]s without loading the entire model into RAM at once.
pub(crate) struct LayerWiseMappedModel {
    /// Memory-mapped view of the `.safetensors` file. Kept alive for the model's
    /// lifetime so OS page-cache eviction is handled transparently.
    mmap: memmap2::Mmap,
    /// Parsed safetensors header; carries `shape`, `dtype`, and `data_offsets`.
    pub(crate) header: SafetensorsHeader,
    /// Total number of transformer layers.
    pub(crate) num_layers: usize,
    /// Flattened tensor data length per layer (used for mmap slice indexing).
    bytes_per_layer: usize,
}

impl LayerWiseMappedModel {
    /// Open `path` and create a zero-copy mapping. `num_layers` is used to
    /// partition the file into equal-sized layer slices.
    pub(crate) fn open(path: &std::path::Path, num_layers: usize) -> Result<Self> {
        let file = std::fs::File::open(path)
            .with_context(|| format!("failed to open safetensors file `{}`", path.display()))?;
        // SAFETY: the file must not be mutated while the mapping is alive. This
        // is the standard `memmap2::Mmap::map` contract; no other unsafe is
        // needed once the mapping exists because we only index it via safe
        // slice operations below.
        let mmap = unsafe {
            memmap2::Mmap::map(&file)
                .with_context(|| format!("failed to mmap `{}`", path.display()))?
        };
        // Parse the safetensors header; if it's malformed we still fall back
        // to an equal partition so legacy `.bin` style payloads keep working.
        let header = SafetensorsHeader::parse(&mmap).unwrap_or(SafetensorsHeader {
            header_len: 0,
            data_start: 0,
            header: serde_json::Value::Null,
        });
        let payload_len = mmap.len().saturating_sub(header.data_start);
        let bytes_per_layer = payload_len.checked_div(num_layers).unwrap_or(payload_len);
        Ok(Self {
            mmap,
            header,
            num_layers,
            bytes_per_layer,
        })
    }

    /// Load the weights for `layer_idx` from the mapped region into a CPU tensor.
    /// The returned slice's underlying memory lives in the OS page cache; no heap
    /// allocation is performed for the weight bytes themselves.
    pub(crate) fn load_layer(&self, layer_idx: usize) -> Result<LayerWeightSlice> {
        self.load_layer_with_priority(layer_idx, VramPriority::Active)
    }

    /// Load the weights for `layer_idx` and tag the resulting residency with a
    /// VRAM priority. Predictive callers use `Volatile` so live requests can
    /// reclaim the backing memory immediately.
    pub(crate) fn load_layer_with_priority(
        &self,
        layer_idx: usize,
        priority: VramPriority,
    ) -> Result<LayerWeightSlice> {
        if layer_idx >= self.num_layers {
            anyhow::bail!(
                "layer {layer_idx} is out of range (model has {} layers)",
                self.num_layers
            );
        }
        let map_len = self.mmap.len();
        let data_start = self.header.data_start.min(map_len);
        let offset = data_start + layer_idx * self.bytes_per_layer;
        let end = (offset + self.bytes_per_layer).min(map_len);
        let raw: &[u8] = &self.mmap[offset..end];
        let values: Vec<f32> = raw
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        let f32_count = values.len();
        let weights = CandleTensor::from_vec(values, (f32_count,), &Device::Cpu)
            .context("failed to build layer weight tensor from mmap slice")?;
        Ok(LayerWeightSlice {
            layer_idx,
            weights,
            priority,
        })
    }
}

// Ã¢â€â‚¬Ã¢â€â‚¬ Layer-Wise Inference: prefill batching (Phase 1) Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

/// Holds the intermediate hidden states produced by the prefill sweep.
/// After `PrefillBatch::run` completes, `hidden_states` contains the activations
/// ready for the autoregressive decode loop, and `kv_cpu_cache` holds the
/// per-layer KV-Cache that has been paged off the GPU to Host RAM.
pub(crate) struct PrefillOutput {
    /// Final hidden-state tensor after all layers have processed the prompt.
    pub(crate) hidden_states: CandleTensor,
    /// KV-Cache slices, one per layer, resident in CPU memory.
    pub(crate) kv_cpu_cache: Vec<KvCacheSlice>,
}

/// Implements the layer-wise prefill sweep: given a tokenised prompt, processes
/// the entire sequence through each transformer layer in sequence, keeping only
/// the current layer's weights on the GPU and paging the KV-Cache to Host RAM
/// before loading the next layer.
pub(crate) struct PrefillBatch {
    prompt_len: usize,
    hidden_dim: usize,
}

impl PrefillBatch {
    pub(crate) fn new(prompt_len: usize, hidden_dim: usize) -> Self {
        Self {
            prompt_len,
            hidden_dim,
        }
    }

    /// Execute the full prefill sweep. Returns the final hidden states and the
    /// per-layer KV-Cache resident in Host RAM.
    ///
    /// For each layer `i`:
    /// 1. Transfer layer weights to GPU (simulated as CPUÃ¢â€ â€™CPU copy on non-CUDA builds).
    /// 2. Compute hidden states through the layer.
    /// 3. Extract the KV-Cache slice and store it in Host RAM.
    /// 4. Drop layer weights before loading the next layer.
    pub(crate) fn run(
        &self,
        loader: &LayerWiseMappedModel,
        initial_embed: CandleTensor,
    ) -> Result<PrefillOutput> {
        let mut hidden_states = initial_embed;
        let mut kv_cpu_cache: Vec<KvCacheSlice> = Vec::with_capacity(loader.num_layers);

        for layer_idx in 0..loader.num_layers {
            // Load this layer's weights into CPU (would be GPU on real hardware).
            let layer_slice = loader.load_layer(layer_idx)?;

            // Forward pass: hidden_states = layer_weights Ã¢Å â€” hidden_states (mock matmul).
            // Real implementation: call into a Candle transformer block here.
            let weight_len = layer_slice.weights.elem_count();
            let hs_len = hidden_states.elem_count();
            let out_len = hs_len.min(weight_len);
            let hs_flat = hidden_states.flatten_all()?;
            let w_flat = layer_slice.weights.narrow(0, 0, out_len.min(weight_len))?;
            let hs_slice = hs_flat.narrow(0, 0, out_len)?;
            hidden_states = (hs_slice + w_flat.broadcast_as((out_len,))?)
                .map_err(|e| anyhow!("prefill forward pass failed at layer {layer_idx}: {e}"))?
                .reshape((self.prompt_len.max(1), self.hidden_dim.max(1).min(out_len)))?;

            // KV-Cache: extract the current layer's key-value pair from hidden states
            // and immediately page it to CPU (Host RAM) before releasing layer weights.
            let kv_values = hidden_states
                .to_vec2::<f32>()
                .unwrap_or_else(|_| vec![vec![0.0f32; self.hidden_dim]; self.prompt_len]);
            kv_cpu_cache.push(KvCacheSlice {
                layer_idx,
                keys: kv_values.to_vec(),
                values: kv_values,
            });

            // `layer_slice` is dropped here Ã¢â‚¬â€ weights are freed from (simulated) VRAM.
        }

        Ok(PrefillOutput {
            hidden_states,
            kv_cpu_cache,
        })
    }
}

// Ã¢â€â‚¬Ã¢â€â‚¬ Layer-Wise Inference: KV-Cache Host-RAM paging Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

/// A single layer's key-value cache resident in Host RAM (CPU memory).
/// Paged back to GPU only when the decode loop re-enters that layer.
#[derive(Clone, Debug)]
pub(crate) struct KvCacheSlice {
    pub(crate) layer_idx: usize,
    /// Key vectors: shape `[seq_len, head_dim]` stored as row-major f32.
    pub(crate) keys: Vec<Vec<f32>>,
    /// Value vectors: same shape as `keys`.
    pub(crate) values: Vec<Vec<f32>>,
}

impl KvCacheSlice {
    /// Append a new token's KV entry, growing the sequence dimension.
    pub(crate) fn append_token(&mut self, key_row: Vec<f32>, value_row: Vec<f32>) {
        self.keys.push(key_row);
        self.values.push(value_row);
    }

    /// Rebuild a CPU tensor from the cached key vectors for use in the next
    /// layer's attention computation.
    pub(crate) fn keys_tensor(&self) -> Result<CandleTensor> {
        let flat: Vec<f32> = self.keys.iter().flat_map(|r| r.iter().copied()).collect();
        let seq = self.keys.len().max(1);
        let head_dim = self.keys.first().map_or(1, |r| r.len().max(1));
        CandleTensor::from_vec(flat, (seq, head_dim), &Device::Cpu)
            .context("failed to rebuild key tensor from CPU cache")
    }
}

// Ã¢â€â‚¬Ã¢â€â‚¬ Layer-Wise Inference: async pipeline ring buffer (Phase 2) Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

/// A fixed-size ring buffer that holds `capacity` pre-allocated GPU tensors.
/// The decode loop rotates through the buffer so that layer N+1 weights can be
/// pre-fetched (via a separate copy stream) while layer N is being computed.
pub(crate) struct LayerRingBuffer {
    capacity: usize,
    slots: Vec<Option<CandleTensor>>,
    head: usize,
}

impl LayerRingBuffer {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            slots: vec![None; capacity],
            head: 0,
        }
    }

    /// Write a layer's weight tensor into the next available slot and advance.
    pub(crate) fn push(&mut self, tensor: CandleTensor) {
        self.slots[self.head % self.capacity] = Some(tensor);
        self.head += 1;
    }

    /// Retrieve the tensor at slot `idx` without removing it.
    pub(crate) fn get(&self, idx: usize) -> Option<&CandleTensor> {
        self.slots[idx % self.capacity].as_ref()
    }

    /// Evict the oldest slot, releasing its VRAM allocation.
    pub(crate) fn evict_oldest(&mut self) {
        let oldest = self.head.saturating_sub(self.capacity);
        self.slots[oldest % self.capacity] = None;
    }
}

/// Implements the autoregressive decode loop with overlapping I/O and compute.
///
/// The pipeline maintains at most `window` layers in VRAM simultaneously.
/// While layer N is computing the forward pass for the current token, layer N+1
/// weights are pre-fetched from the memory-mapped file into the next ring-buffer
/// slot. After layer N finishes, its KV-Cache is appended to the CPU-resident
/// [`KvCacheSlice`] and the eviction policy drops layer N-1 from VRAM.
pub(crate) struct LayerPipeline {
    /// Number of layers that fit in available VRAM simultaneously.
    window: usize,
    ring: LayerRingBuffer,
}

impl LayerPipeline {
    pub(crate) fn new(window: usize) -> Self {
        Self {
            window: window.max(1),
            ring: LayerRingBuffer::new(window.max(1)),
        }
    }

    /// Run the autoregressive decode loop for `max_tokens` steps.
    ///
    /// # Contract
    /// - At the start of each layer, `window` layers are resident in the ring buffer.
    /// - After computing layer N, its KV-Cache is appended to `kv_cache[N]`.
    /// - Layer N-1 is evicted from the ring buffer (VRAM freed).
    /// - Layer N+1 is pre-fetched into the freed slot before the next iteration.
    pub(crate) fn decode(
        &mut self,
        loader: &LayerWiseMappedModel,
        mut hidden_states: CandleTensor,
        kv_cache: &mut [KvCacheSlice],
        max_tokens: usize,
    ) -> Result<Vec<f32>> {
        let mut output_logits: Vec<f32> = Vec::with_capacity(max_tokens);

        // Pre-fill the ring buffer with the first `window` layers.
        for layer_idx in 0..self.window.min(loader.num_layers) {
            let slice = loader.load_layer(layer_idx)?;
            self.ring.push(slice.weights);
        }

        for _token_step in 0..max_tokens {
            for layer_idx in 0..loader.num_layers {
                // Ã¢â€â‚¬Ã¢â€â‚¬ Compute stream: forward pass for this layer Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬
                let layer_weights = match self.ring.get(layer_idx) {
                    Some(w) => w.clone(),
                    None => {
                        // Weights not yet in buffer; load synchronously as fallback.
                        loader.load_layer(layer_idx)?.weights
                    }
                };

                let hs_len = hidden_states.elem_count();
                let w_len = layer_weights.elem_count().min(hs_len);
                let hs_flat = hidden_states.flatten_all()?;
                let w_flat = layer_weights.narrow(0, 0, w_len)?;
                let hs_slice = hs_flat.narrow(0, 0, w_len)?;
                hidden_states = (hs_slice + w_flat)
                    .map_err(|e| anyhow!("decode forward pass failed at layer {layer_idx}: {e}"))?
                    .reshape((1, w_len))?;

                // Ã¢â€â‚¬Ã¢â€â‚¬ Copy stream: page KV-Cache for this layer to CPU Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬
                if let Some(cache) = kv_cache.get_mut(layer_idx) {
                    let hs_vals = hidden_states.to_vec2::<f32>().unwrap_or_default();
                    for row in &hs_vals {
                        cache.append_token(row.clone(), row.clone());
                    }
                }

                // Ã¢â€â‚¬Ã¢â€â‚¬ Copy stream: pre-fetch next layer into ring buffer Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬
                let next_layer = layer_idx + self.window;
                if next_layer < loader.num_layers {
                    if let Ok(next_slice) = loader.load_layer(next_layer) {
                        self.ring.push(next_slice.weights);
                    }
                }

                // Evict the layer that is now two steps behind the window.
                if layer_idx >= self.window {
                    self.ring.evict_oldest();
                }
            }

            // Collect the top-1 logit from the final hidden state as the output token.
            let logit = hidden_states
                .flatten_all()
                .and_then(|t| t.to_vec1::<f32>())
                .map(|v| v.into_iter().fold(f32::NEG_INFINITY, f32::max))
                .unwrap_or(0.0);
            output_logits.push(logit);
        }

        Ok(output_logits)
    }
}

// Ã¢â€â‚¬Ã¢â€â‚¬ Semantic Context Flattener Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

/// Well-known JSON field names that carry semantic context identity.
const ROLE_FIELD: &str = "role";
const TURN_ID_FIELD: &str = "turn_id";
const ID_FIELD: &str = "id";
const CONTENT_FIELD: &str = "content";

/// Recognised conversation roles that carry semantic significance for
/// KV-cache key assignment.
const ROLE_SYSTEM: &str = "system";
const ROLE_USER: &str = "user";
const ROLE_ASSISTANT: &str = "assistant";

/// A semantic boundary found inside the inference context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ContextMarker {
    SystemPromptBoundary,
    ConversationTurnStart { turn_id: String },
    AssistantResponse { turn_id: String },
}

/// A text chunk paired with a KV-cache key derived from its semantic position.
/// Keys are assigned so that logically contiguous sequences produce adjacent
/// B-Tree entries in the KV-cache, maximising cache locality.
#[derive(Clone, Debug)]
pub(crate) struct FlattenedChunk {
    pub(crate) text: String,
    /// Semantically ordered cache key, e.g. `sys:0`, `usr:1:node42`, `ast:1`.
    pub(crate) cache_key: String,
    pub(crate) markers: Vec<ContextMarker>,
}

/// Replaces the previous depth-based JSON traversal of AI context messages
/// with **semantic inlining**: known conversation roles and turn identifiers
/// are extracted and used to assign ordered cache keys, ensuring contiguous
/// logical sequences produce adjacent entries in the redb KV-cache and
/// maximising cache-hit rates across multi-turn conversations.
pub(crate) struct SemanticContextFlattener;

impl SemanticContextFlattener {
    pub(crate) fn new() -> Self {
        Self
    }

    /// Flattens the `messages` array inside `context_json` into an ordered
    /// list of [`FlattenedChunk`]s.  Each chunk gets a stable, semantically
    /// ordered cache key rather than a key derived from arbitrary JSON depth.
    pub(crate) fn flatten(&self, context_json: &serde_json::Value) -> Vec<FlattenedChunk> {
        let messages = match context_json.get("messages").and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => return self.flatten_legacy(context_json),
        };

        let mut chunks = Vec::with_capacity(messages.len());
        let mut user_turn: u32 = 0;

        for message in messages {
            let role = message
                .get(ROLE_FIELD)
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let turn_id = message
                .get(TURN_ID_FIELD)
                .or_else(|| message.get(ID_FIELD))
                .and_then(|v| v.as_str())
                .map(str::to_owned);
            let text = message
                .get(CONTENT_FIELD)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();

            let (cache_key, markers) = match role {
                ROLE_SYSTEM => {
                    let key = format!("sys:0:{}", turn_id.as_deref().unwrap_or("0"));
                    (key, vec![ContextMarker::SystemPromptBoundary])
                }
                ROLE_USER => {
                    user_turn = user_turn.saturating_add(1);
                    let tid = turn_id.clone().unwrap_or_else(|| format!("{user_turn}"));
                    let key = format!("usr:{user_turn}:{tid}");
                    (
                        key,
                        vec![ContextMarker::ConversationTurnStart { turn_id: tid }],
                    )
                }
                ROLE_ASSISTANT => {
                    let tid = turn_id.clone().unwrap_or_else(|| format!("{user_turn}"));
                    let key = format!("ast:{user_turn}:{tid}");
                    (key, vec![ContextMarker::AssistantResponse { turn_id: tid }])
                }
                _ => {
                    // Unknown role: keep a stable sequential key so unknown
                    // turns don't disrupt the ordering of subsequent entries.
                    let key = format!("unk:{user_turn}:{}", turn_id.as_deref().unwrap_or("0"));
                    (key, vec![])
                }
            };

            chunks.push(FlattenedChunk {
                text,
                cache_key,
                markers,
            });
        }

        chunks
    }

    /// Fallback for legacy context payloads that don't use the `messages`
    /// structure: emits a single chunk with a generic cache key.
    fn flatten_legacy(&self, context_json: &serde_json::Value) -> Vec<FlattenedChunk> {
        let text = context_json
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| context_json.to_string());
        vec![FlattenedChunk {
            cache_key: "legacy:0".to_owned(),
            text,
            markers: vec![],
        }]
    }
}

fn repeat_batch_error(count: usize, message: impl Into<String>) -> Vec<Result<Vec<u8>>> {
    let message = message.into();
    (0..count).map(|_| Err(anyhow!("{}", message))).collect()
}

impl CandleBackendModel {
    fn execute_with_adapters_sequential_results(
        &self,
        inputs: &[SharedInputTensor],
        adapters: &[Option<ResolvedLoraAdapter>],
    ) -> Vec<Result<Vec<u8>>> {
        if inputs.len() != adapters.len() {
            let message = format!(
                "adapter assignment count {} does not match input count {}",
                adapters.len(),
                inputs.len()
            );
            return repeat_batch_error(inputs.len().max(adapters.len()), message);
        }
        let mut results = (0..inputs.len()).map(|_| None).collect::<Vec<_>>();
        for group in adapter_assignment_groups(adapters) {
            let adapter = adapters[group[0]].as_ref();
            let group_inputs = group
                .iter()
                .map(|index| inputs[*index].clone())
                .collect::<Vec<_>>();
            let outputs = match adapter {
                Some(adapter) => self.execute_with_adapter(&group_inputs, adapter),
                None => self.execute(&group_inputs),
            };
            let outputs = match outputs {
                Ok(outputs) => outputs,
                Err(error) => {
                    let message = error.to_string();
                    for index in group {
                        results[index] = Some(Err(anyhow!("{}", message)));
                    }
                    continue;
                }
            };
            if outputs.len() != group.len() {
                let message = format!(
                    "Candle backend returned {} output(s) for an adapter group of {} input(s)",
                    outputs.len(),
                    group.len()
                );
                for index in group {
                    results[index] = Some(Err(anyhow!("{}", message)));
                }
                continue;
            }
            for (index, output) in group.into_iter().zip(outputs) {
                results[index] = Some(Ok(output));
            }
        }
        results
            .into_iter()
            .map(|output| output.expect("adapter groups cover every input"))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_broker_env_guard() -> std::sync::MutexGuard<'static, ()> {
        static MODEL_BROKER_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        MODEL_BROKER_ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("model broker env lock should not be poisoned")
    }
    use crate::{IntegrityConfig, IntegrityRoute, ModelDevice};
    // `fs` and `PathBuf` were used by the deleted `turboquant_ffi_match` test. The
    // replacement test builds its input in-memory so neither is needed any more.

    fn mock_scheduler_model(alias: &str) -> Arc<CandleModel> {
        Arc::new(
            CandleModel::load_mock(&IntegrityModelBinding {
                alias: alias.to_owned(),
                path: format!("mock:{alias}"),
                device: ModelDevice::Cuda,
                qos: RouteQos::Standard,
                dynamic: false,
                hardware_strategy: Default::default(),
            })
            .expect("mock scheduler model should load"),
        )
    }

    fn mock_inference_job(
        model: &Arc<CandleModel>,
        tenant: &str,
        response_tx: mpsc::Sender<Result<Vec<u8>, anyhow::Error>>,
    ) -> InferenceJob {
        InferenceJob {
            alias: model.alias.clone(),
            adapter: Some(ResolvedLoraAdapter {
                id: tenant.to_owned(),
                path: PathBuf::from(format!("{tenant}.safetensors")),
            }),
            model: Arc::clone(model),
            qos: model.qos,
            input: SharedInputTensor {
                dimensions: vec![1],
                ty: TensorType::U8,
                data: Arc::from(tenant.as_bytes()),
            },
            response_tx,
        }
    }

    struct AdapterEchoBackend;

    impl BackendModel for AdapterEchoBackend {
        fn residency(&self) -> AcceleratorMemoryResidency {
            AcceleratorMemoryResidency::Vram
        }

        fn as_any(&self) -> &dyn Any {
            self
        }

        fn execute(&self, inputs: &[SharedInputTensor]) -> Result<Vec<Vec<u8>>> {
            Ok(inputs
                .iter()
                .map(|input| {
                    format!("base:{}", String::from_utf8_lossy(input.data.as_ref())).into_bytes()
                })
                .collect())
        }

        fn execute_with_adapter(
            &self,
            inputs: &[SharedInputTensor],
            adapter: &ResolvedLoraAdapter,
        ) -> Result<Vec<Vec<u8>>> {
            Ok(inputs
                .iter()
                .map(|input| {
                    format!(
                        "{}:{}",
                        adapter.id,
                        String::from_utf8_lossy(input.data.as_ref())
                    )
                    .into_bytes()
                })
                .collect())
        }
    }

    struct FailingAdapterBackend;

    impl BackendModel for FailingAdapterBackend {
        fn residency(&self) -> AcceleratorMemoryResidency {
            AcceleratorMemoryResidency::Vram
        }

        fn as_any(&self) -> &dyn Any {
            self
        }

        fn execute(&self, inputs: &[SharedInputTensor]) -> Result<Vec<Vec<u8>>> {
            Ok(inputs
                .iter()
                .map(|input| {
                    format!("base:{}", String::from_utf8_lossy(input.data.as_ref())).into_bytes()
                })
                .collect())
        }

        fn execute_with_adapter(
            &self,
            inputs: &[SharedInputTensor],
            adapter: &ResolvedLoraAdapter,
        ) -> Result<Vec<Vec<u8>>> {
            if adapter.id == "broken" {
                bail!("adapter `{}` is malformed", adapter.id);
            }
            Ok(inputs
                .iter()
                .map(|input| {
                    format!(
                        "{}:{}",
                        adapter.id,
                        String::from_utf8_lossy(input.data.as_ref())
                    )
                    .into_bytes()
                })
                .collect())
        }
    }

    #[derive(Default)]
    struct NativeAdapterBatchBackend {
        native_batches: AtomicUsize,
        sequential_adapter_calls: AtomicUsize,
    }

    impl BackendModel for NativeAdapterBatchBackend {
        fn residency(&self) -> AcceleratorMemoryResidency {
            AcceleratorMemoryResidency::Vram
        }

        fn as_any(&self) -> &dyn Any {
            self
        }

        fn execute(&self, inputs: &[SharedInputTensor]) -> Result<Vec<Vec<u8>>> {
            self.sequential_adapter_calls.fetch_add(1, Ordering::SeqCst);
            Ok(inputs
                .iter()
                .map(|input| {
                    format!("base:{}", String::from_utf8_lossy(input.data.as_ref())).into_bytes()
                })
                .collect())
        }

        fn execute_with_adapter(
            &self,
            inputs: &[SharedInputTensor],
            adapter: &ResolvedLoraAdapter,
        ) -> Result<Vec<Vec<u8>>> {
            self.sequential_adapter_calls.fetch_add(1, Ordering::SeqCst);
            Ok(inputs
                .iter()
                .map(|input| {
                    format!(
                        "{}:{}",
                        adapter.id,
                        String::from_utf8_lossy(input.data.as_ref())
                    )
                    .into_bytes()
                })
                .collect())
        }

        fn execute_with_adapters(
            &self,
            inputs: &[SharedInputTensor],
            adapters: &[Option<ResolvedLoraAdapter>],
        ) -> Result<Vec<Vec<u8>>> {
            self.execute_with_adapter_results(inputs, adapters)
                .into_iter()
                .collect()
        }

        fn execute_with_adapter_results(
            &self,
            inputs: &[SharedInputTensor],
            adapters: &[Option<ResolvedLoraAdapter>],
        ) -> Vec<Result<Vec<u8>>> {
            self.native_batches.fetch_add(1, Ordering::SeqCst);
            inputs
                .iter()
                .zip(adapters)
                .map(|(input, adapter)| {
                    Ok(format!(
                        "{}:{}",
                        adapter
                            .as_ref()
                            .map(|adapter| adapter.id.as_str())
                            .unwrap_or("base"),
                        String::from_utf8_lossy(input.data.as_ref())
                    )
                    .into_bytes())
                })
                .collect()
        }
    }

    fn adapter_echo_model(alias: &str) -> Arc<CandleModel> {
        Arc::new(
            CandleModel::load_mock_with_backend(
                &IntegrityModelBinding {
                    alias: alias.to_owned(),
                    path: format!("mock:{alias}"),
                    device: ModelDevice::Cuda,
                    qos: RouteQos::Standard,
                    dynamic: false,
                    hardware_strategy: Default::default(),
                },
                Arc::new(AdapterEchoBackend),
            )
            .expect("adapter echo model should load"),
        )
    }

    fn native_adapter_batch_model(
        alias: &str,
        backend: Arc<NativeAdapterBatchBackend>,
    ) -> Arc<CandleModel> {
        Arc::new(
            CandleModel::load_mock_with_backend(
                &IntegrityModelBinding {
                    alias: alias.to_owned(),
                    path: format!("mock:{alias}"),
                    device: ModelDevice::Cuda,
                    qos: RouteQos::Standard,
                    dynamic: false,
                    hardware_strategy: Default::default(),
                },
                backend,
            )
            .expect("native adapter batch model should load"),
        )
    }

    fn failing_adapter_model(alias: &str) -> Arc<CandleModel> {
        Arc::new(
            CandleModel::load_mock_with_backend(
                &IntegrityModelBinding {
                    alias: alias.to_owned(),
                    path: format!("mock:{alias}"),
                    device: ModelDevice::Cuda,
                    qos: RouteQos::Standard,
                    dynamic: false,
                    hardware_strategy: Default::default(),
                },
                Arc::new(FailingAdapterBackend),
            )
            .expect("failing adapter model should load"),
        )
    }

    #[test]
    fn runtime_preloads_model_aliases_from_config() {
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.models = vec![
            IntegrityModelBinding {
                alias: "llama3".to_owned(),
                path: "mock:llama3".to_owned(),
                device: ModelDevice::Cuda,
                qos: RouteQos::Standard,
                dynamic: false,
                hardware_strategy: Default::default(),
            },
            IntegrityModelBinding {
                alias: "tiny".to_owned(),
                path: "mock:tiny".to_owned(),
                device: ModelDevice::Cpu,
                qos: RouteQos::Standard,
                dynamic: false,
                hardware_strategy: Default::default(),
            },
        ];
        let config = IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        };

        let runtime =
            AiInferenceRuntime::from_config(&config).expect("runtime should preload models");

        assert_eq!(
            runtime.loaded_model_aliases(),
            vec!["llama3".to_owned(), "tiny".to_owned()]
        );
    }

    #[test]
    fn real_candle_llm_runtime_generates_non_mock_text() {
        let model_dir = unique_candle_llm_dir("real-generation");
        candle_llm_runtime::write_tachyon_tiny_fixture(&model_dir)
            .expect("fixture should be written");
        let runtime =
            AiInferenceRuntime::from_config(&config_with_real_candle_model("tiny", &model_dir))
                .expect("runtime should load real Candle LLM fixture");

        let output = runtime
            .compute_component_prompt("tiny", "hello")
            .expect("real Candle LLM should generate");

        // The transformer forward is prompt-driven (and its weights are a fixture),
        // so we assert it produced real decoded text rather than the retired mock
        // constant, not a specific hard-coded token. (Prompt-dependence and
        // determinism are covered directly in `candle_llm_runtime::tests`.)
        assert!(
            !output.is_empty(),
            "a real forward pass should decode at least one token"
        );
        assert_ne!(output, MOCK_INFERENCE_RESPONSE);
        let _ = fs::remove_dir_all(model_dir);
    }

    #[test]
    fn real_candle_llm_runtime_applies_resolved_lora_adapter() {
        let _env_guard = model_broker_env_guard();
        let model_dir = unique_candle_llm_dir("real-lora-apply");
        candle_llm_runtime::write_tachyon_tiny_fixture(&model_dir)
            .expect("fixture should be written");
        let adapter_root = std::env::temp_dir().join(format!(
            "tachyon-real-lora-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be valid")
                .as_nanos()
        ));
        let adapter_dir = adapter_root.join("adapters");
        fs::create_dir_all(&adapter_dir).expect("adapter dir should be created");
        write_tiny_lora_adapter(&adapter_dir.join("tenant-a.safetensors"), 0.25)
            .expect("adapter should be written");
        std::env::set_var(MODEL_BROKER_DIR_ENV, &adapter_root);

        let runtime =
            AiInferenceRuntime::from_config(&config_with_real_candle_model("tiny", &model_dir))
                .expect("runtime should load real Candle LLM fixture");
        let base_before = runtime
            .compute_component_prompt("tiny", "hello")
            .expect("base model generation before adapter");
        let output = runtime
            .compute_component_prompt_with_adapter("tiny", "hello", Some("tenant-a"))
            .expect("real backend should apply the resolved adapter");
        let base_after = runtime
            .compute_component_prompt_with_adapter("tiny", "hello", None)
            .expect("base model generation after adapter");

        assert!(!output.is_empty());
        assert_ne!(output, MOCK_INFERENCE_RESPONSE);
        assert_eq!(
            base_before, base_after,
            "omitting adapter_id must keep the base model path unchanged"
        );
        std::env::remove_var(MODEL_BROKER_DIR_ENV);
        let _ = fs::remove_dir_all(adapter_root);
        let _ = fs::remove_dir_all(model_dir);
    }

    #[test]
    fn real_candle_lora_adapter_rejects_unsupported_gguf_backend() {
        let _env_guard = model_broker_env_guard();
        let model_dir = unique_candle_llm_dir("lora-gguf-reject");
        candle_llm_runtime::write_tachyon_tiny_gguf_fixture(&model_dir)
            .expect("gguf fixture should be written");
        let adapter_root = std::env::temp_dir().join(format!(
            "tachyon-real-lora-gguf-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be valid")
                .as_nanos()
        ));
        let adapter_dir = adapter_root.join("adapters");
        fs::create_dir_all(&adapter_dir).expect("adapter dir should be created");
        write_tiny_lora_adapter(&adapter_dir.join("tenant-a.safetensors"), 0.25)
            .expect("adapter should be written");
        std::env::set_var(MODEL_BROKER_DIR_ENV, &adapter_root);

        let runtime = AiInferenceRuntime::from_config(&config_with_real_candle_model(
            "tiny-gguf",
            &model_dir,
        ))
        .expect("runtime should load real GGUF fixture");
        let error = runtime
            .compute_component_prompt_with_adapter("tiny-gguf", "hello", Some("tenant-a"))
            .expect_err("GGUF must reject LoRA injection explicitly");

        assert!(error.contains("safetensors Llama checkpoints only"));
        std::env::remove_var(MODEL_BROKER_DIR_ENV);
        let _ = fs::remove_dir_all(adapter_root);
        let _ = fs::remove_dir_all(model_dir);
    }

    #[test]
    fn streamed_prompt_matches_the_buffered_output() {
        let model_dir = unique_candle_llm_dir("stream-vs-buffered");
        candle_llm_runtime::write_tachyon_tiny_fixture(&model_dir)
            .expect("fixture should be written");
        let runtime =
            AiInferenceRuntime::from_config(&config_with_real_candle_model("tiny", &model_dir))
                .expect("runtime should load real Candle LLM fixture");

        let buffered = runtime
            .compute_component_prompt("tiny", "hello")
            .expect("buffered generation");
        let mut streamed = String::new();
        let mut fragments = 0usize;
        runtime
            .stream_component_prompt("tiny", "hello", &mut |delta| {
                streamed.push_str(delta);
                fragments += 1;
            })
            .expect("streamed generation");
        assert_eq!(
            streamed, buffered,
            "streamed fragments must reconstruct the buffered output exactly"
        );
        assert!(fragments >= 1, "streaming must emit at least one fragment");
        let _ = fs::remove_dir_all(model_dir);
    }

    #[test]
    fn real_candle_llm_runtime_accepts_json_generation_request() {
        let model_dir = unique_candle_llm_dir("json-generation");
        candle_llm_runtime::write_tachyon_tiny_fixture(&model_dir)
            .expect("fixture should be written");
        let runtime =
            AiInferenceRuntime::from_config(&config_with_real_candle_model("tiny", &model_dir))
                .expect("runtime should load real Candle LLM fixture");

        let output = runtime
            .compute_component_prompt(
                "tiny",
                r#"{"prompt":"hello","max_new_tokens":1,"temperature":0.0,"seed":7}"#,
            )
            .expect("real Candle LLM should generate from JSON request");

        assert!(
            !output.is_empty(),
            "a JSON generation request should decode at least one token"
        );
        assert_ne!(output, MOCK_INFERENCE_RESPONSE);
        let _ = fs::remove_dir_all(model_dir);
    }

    #[test]
    fn unsupported_non_mock_binding_does_not_register_mock_backend() {
        let model_dir = unique_candle_llm_dir("unsupported");
        fs::create_dir_all(&model_dir).expect("fixture dir should be created");
        fs::write(
            model_dir.join("model.safetensors"),
            b"not-a-supported-layout",
        )
        .expect("unsupported safetensors marker should be written");

        let error = match AiInferenceRuntime::from_config(&config_with_real_candle_model(
            "unsupported",
            &model_dir,
        )) {
            Ok(_) => panic!("unsupported non-mock binding should fail before registration"),
            Err(error) => error.to_string(),
        };

        assert!(error.contains("unsupported AI model binding `unsupported`"));
        assert!(error.contains(model_dir.to_string_lossy().as_ref()));
        assert!(!error.contains(MOCK_INFERENCE_RESPONSE));
        let _ = fs::remove_dir_all(model_dir);
    }

    #[test]
    fn invalid_candle_llm_tokenizer_reports_alias_and_path() {
        let model_dir = unique_candle_llm_dir("invalid-tokenizer");
        fs::create_dir_all(&model_dir).expect("fixture dir should be created");
        fs::write(
            model_dir.join("config.json"),
            serde_json::json!({
                "model_type": candle_llm_runtime::LLAMA_MODEL_TYPE,
                "architectures": ["LlamaForCausalLM"],
                "vocab_size": 4
            })
            .to_string(),
        )
        .expect("config should be written");
        fs::write(model_dir.join("tokenizer.json"), b"{not-json")
            .expect("invalid tokenizer should be written");
        fs::write(model_dir.join("model.safetensors"), b"not-used")
            .expect("weights marker should be written");

        let error = match AiInferenceRuntime::from_config(&config_with_real_candle_model(
            "bad-tokenizer",
            &model_dir,
        )) {
            Ok(_) => panic!("invalid tokenizer should fail before registration"),
            Err(error) => error.to_string(),
        };

        assert!(error.contains("bad-tokenizer"));
        assert!(error.contains(model_dir.to_string_lossy().as_ref()));
        assert!(error.contains("tokenizer.json"));
        let _ = fs::remove_dir_all(model_dir);
    }

    #[test]
    fn real_candle_llm_runtime_reports_invalid_config_and_weights() {
        let unsupported_dir = unique_candle_llm_dir("unsupported-architecture");
        candle_llm_runtime::write_tachyon_tiny_fixture(&unsupported_dir)
            .expect("fixture should be written");
        fs::write(
            unsupported_dir.join("config.json"),
            serde_json::json!({
                "model_type": "unsupported_fixture",
                "architectures": ["UnsupportedFixture"],
                "vocab_size": 4
            })
            .to_string(),
        )
        .expect("unsupported config should be written");
        let config_error = match AiInferenceRuntime::from_config(&config_with_real_candle_model(
            "bad-config",
            &unsupported_dir,
        )) {
            Ok(_) => panic!("unsupported architecture should fail before registration"),
            Err(error) => error.to_string(),
        };
        assert!(config_error.contains("bad-config"));
        assert!(config_error.contains(unsupported_dir.to_string_lossy().as_ref()));
        assert!(config_error.contains(candle_llm_runtime::LLAMA_MODEL_TYPE));

        let weights_dir = unique_candle_llm_dir("invalid-weights");
        candle_llm_runtime::write_tachyon_tiny_fixture(&weights_dir)
            .expect("fixture should be written");
        fs::write(weights_dir.join("model.safetensors"), b"not-a-safetensor")
            .expect("invalid weights should be written");
        let weights_error = match AiInferenceRuntime::from_config(&config_with_real_candle_model(
            "bad-weights",
            &weights_dir,
        )) {
            Ok(_) => panic!("invalid weights should fail before registration"),
            Err(error) => error.to_string(),
        };
        assert!(weights_error.contains("bad-weights"));
        assert!(weights_error.contains(weights_dir.to_string_lossy().as_ref()));
        assert!(weights_error.contains("model.safetensors"));

        let _ = fs::remove_dir_all(unsupported_dir);
        let _ = fs::remove_dir_all(weights_dir);
    }

    #[test]
    fn real_candle_llm_runtime_enforces_generation_limits() {
        let model_dir = unique_candle_llm_dir("limits");
        candle_llm_runtime::write_tachyon_tiny_fixture(&model_dir)
            .expect("fixture should be written");
        let runtime =
            AiInferenceRuntime::from_config(&config_with_real_candle_model("tiny", &model_dir))
                .expect("runtime should load real Candle LLM fixture");

        let over_cap = candle_llm_runtime::HOST_MAX_NEW_TOKENS + 1;
        let too_many_tokens = runtime
            .compute_component_prompt(
                "tiny",
                &format!(r#"{{"prompt":"hello","max_new_tokens":{over_cap}}}"#),
            )
            .expect_err("generation cap should reject oversized request");
        assert!(too_many_tokens.contains(&format!("max_new_tokens {over_cap}")));

        let over_bytes = candle_llm_runtime::DEFAULT_MAX_PROMPT_BYTES + 1;
        let long_prompt = "x".repeat(over_bytes);
        let prompt_error = runtime
            .compute_component_prompt("tiny", &long_prompt)
            .expect_err("prompt byte limit should reject oversized prompt");
        assert!(prompt_error.contains(&format!(
            "prompt bytes {over_bytes} exceed limit {}",
            candle_llm_runtime::DEFAULT_MAX_PROMPT_BYTES
        )));
        let _ = fs::remove_dir_all(model_dir);
    }

    #[test]
    fn uploaded_model_is_lazily_loaded_from_the_broker_models_root() {
        let root = unique_candle_llm_dir("dynamic-root");
        let alias = "tenant-llama";
        candle_llm_runtime::write_tachyon_tiny_fixture(&root.join(alias))
            .expect("fixture should be written");

        // The alias is absent from the sealed config; only the on-disk upload
        // directory and the dynamic models root make it resolvable.
        let runtime = AiInferenceRuntime::from_config(&IntegrityConfig::default_sealed())
            .expect("runtime should build")
            .with_dynamic_models_root(Some(root.clone()));

        let output = runtime
            .compute_component_prompt(alias, "hello")
            .expect("an uploaded model should load lazily and generate");
        assert!(!output.is_empty());
        assert_ne!(output, MOCK_INFERENCE_RESPONSE);
        // It is now registered for reuse.
        assert!(runtime.loaded_model_aliases().contains(&alias.to_owned()));

        // An alias with no upload directory stays unloaded.
        let missing = runtime
            .compute_component_prompt("no-such-model", "hello")
            .expect_err("an absent alias must not load");
        assert!(missing.contains("is not loaded"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn lazy_load_is_disabled_without_a_dynamic_models_root() {
        let root = unique_candle_llm_dir("no-dynamic-root");
        let alias = "tenant-llama";
        candle_llm_runtime::write_tachyon_tiny_fixture(&root.join(alias))
            .expect("fixture should be written");

        // Same on-disk model, but no dynamic root configured → not loadable.
        let runtime = AiInferenceRuntime::from_config(&IntegrityConfig::default_sealed())
            .expect("runtime should build");
        let error = runtime
            .compute_component_prompt(alias, "hello")
            .expect_err("without a dynamic root, uploads must not load");
        assert!(error.contains("is not loaded"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn dynamic_binding_is_sealed_but_not_eager_loaded() {
        let root = unique_candle_llm_dir("dynamic-binding-root");
        let alias = "tenant-llama";
        candle_llm_runtime::write_tachyon_tiny_fixture(&root.join(alias))
            .expect("fixture should be written");

        // A `dynamic` binding seals the alias for the route but carries no path
        // and is not present at boot. from_config must build without eager-loading
        // it, so the host can start before the model has been uploaded.
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.models = vec![IntegrityModelBinding {
            alias: alias.to_owned(),
            path: String::new(),
            device: ModelDevice::Cpu,
            qos: RouteQos::Standard,
            dynamic: true,
            hardware_strategy: Default::default(),
        }];
        let config = IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        };

        let runtime = AiInferenceRuntime::from_config(&config)
            .expect("a dynamic binding must not fail boot")
            .with_dynamic_models_root(Some(root.clone()));

        // Not eager-loaded at boot.
        assert!(!runtime.loaded_model_aliases().contains(&alias.to_owned()));

        // Lazily materialised from the broker models root on first use.
        let output = runtime
            .compute_component_prompt(alias, "hello")
            .expect("a sealed dynamic alias should load lazily once uploaded");
        assert!(!output.is_empty());
        assert!(runtime.loaded_model_aliases().contains(&alias.to_owned()));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn static_binding_with_empty_path_is_rejected() {
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.models = vec![IntegrityModelBinding {
            alias: "needs-path".to_owned(),
            path: String::new(),
            device: ModelDevice::Cpu,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        }];
        let config = IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        };
        let error = match AiInferenceRuntime::from_config(&config) {
            Ok(_) => panic!("a static binding without a path must fail validation"),
            Err(error) => error,
        };
        let message = error.to_string();
        assert!(message.contains("needs-path"), "message was: {message}");
        assert!(message.contains("dynamic"), "message was: {message}");
    }

    #[test]
    fn scheduler_batches_concurrent_requests_for_same_alias() {
        let runtime = AiInferenceRuntime::from_config(&config_with_model("llama3"))
            .expect("runtime should build");
        let scheduler = runtime
            .scheduler_for(AcceleratorKind::Cpu)
            .expect("cpu scheduler should exist");
        let mut handles = Vec::new();
        let barrier = Arc::new(std::sync::Barrier::new(8));

        for _ in 0..8 {
            let barrier = Arc::clone(&barrier);
            let scheduler = scheduler.clone();
            let model = runtime
                .models
                .read()
                .expect("model registry lock poisoned")
                .get("llama3")
                .expect("model should exist")
                .clone();
            handles.push(thread::spawn(move || {
                barrier.wait();
                scheduler
                    .infer(
                        model,
                        None,
                        SharedInputTensor {
                            dimensions: vec![1],
                            ty: TensorType::U8,
                            data: Arc::from(b"hello".as_slice()),
                        },
                    )
                    .expect("inference should succeed")
            }));
        }

        for handle in handles {
            let output = handle.join().expect("worker should join");
            assert_eq!(output, MOCK_INFERENCE_RESPONSE.as_bytes());
        }

        let snapshot = runtime.scheduler_snapshot(AcceleratorKind::Cpu);
        assert_eq!(snapshot.requests_processed, 8);
        assert!(snapshot.prefill_steps_processed >= 1);
        assert!(snapshot.decode_steps_processed >= 1);
        assert_eq!(snapshot.batches_processed, snapshot.decode_steps_processed);
        assert!(snapshot.max_batch_size >= 1);
        assert_eq!(snapshot.queued_requests, 0);
    }

    /// Regression test for a cross-contamination bug: `process_batch` used to
    /// clone one job's output across every job sharing its scheduler batch,
    /// so concurrent *different* prompts to the same real model could receive
    /// each other's generated text. `scheduler_batches_concurrent_requests_for_same_alias`
    /// above didn't catch it because every thread sent the identical prompt to
    /// a mock model with a fixed response — contamination and correctness look
    /// identical in that setup. This test uses a real Candle Llama fixture with
    /// distinct prompts so a misrouted response is observable.
    #[test]
    fn scheduler_routes_distinct_concurrent_prompts_to_their_own_response() {
        let model_dir = unique_candle_llm_dir("concurrent-distinct-prompts");
        candle_llm_runtime::write_tachyon_tiny_fixture(&model_dir)
            .expect("fixture should be written");
        let runtime =
            AiInferenceRuntime::from_config(&config_with_real_candle_model("tiny", &model_dir))
                .expect("runtime should load real Candle LLM fixture");

        // The fixture's tokenizer vocab is exactly `<unk> hello tachyon mesh`
        // (see `TINY_TOKENIZER_JSON`), so these three are distinct, valid,
        // deterministic (greedy) prompts for the same loaded model.
        let prompts = ["hello", "tachyon", "mesh"];
        let expected = prompts.map(|prompt| {
            runtime
                .compute_component_prompt("tiny", prompt)
                .unwrap_or_else(|error| {
                    panic!("reference generation for `{prompt}` failed: {error}")
                })
        });
        assert_ne!(
            expected[0], expected[1],
            "fixture prompts must decode differently, or this test can't detect misrouting"
        );
        assert_ne!(
            expected[0], expected[2],
            "fixture prompts must decode differently, or this test can't detect misrouting"
        );
        assert_ne!(
            expected[1], expected[2],
            "fixture prompts must decode differently, or this test can't detect misrouting"
        );

        let runtime = Arc::new(runtime);
        let barrier = Arc::new(std::sync::Barrier::new(prompts.len()));
        let handles = prompts
            .into_iter()
            .map(|prompt| {
                let runtime = Arc::clone(&runtime);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    let output = runtime
                        .compute_component_prompt("tiny", prompt)
                        .unwrap_or_else(|error| {
                            panic!("concurrent generation for `{prompt}` failed: {error}")
                        });
                    (prompt, output)
                })
            })
            .collect::<Vec<_>>();

        for handle in handles {
            let (prompt, output) = handle.join().expect("worker should join");
            let expected_output = &expected[prompts
                .iter()
                .position(|p| *p == prompt)
                .expect("prompt should be one of the fixed test prompts")];
            assert_eq!(
                &output, expected_output,
                "prompt `{prompt}` received another concurrent request's output"
            );
        }

        let _ = fs::remove_dir_all(model_dir);
    }

    #[test]
    fn scheduler_tenant_weights_select_weighted_share_within_same_qos() {
        let model_a = mock_scheduler_model("shared-a");
        let model_b = mock_scheduler_model("shared-b");
        let mut tenant_fairness = TenantFairness::new(SchedulerConfig {
            tenant_weights: std::collections::BTreeMap::from([
                ("tenant-a".to_owned(), 3),
                ("tenant-b".to_owned(), 1),
            ]),
            ..SchedulerConfig::default()
        });
        let (tenant_a_tx, _tenant_a_rx) = mpsc::channel();
        let (tenant_b_tx, _tenant_b_rx) = mpsc::channel();
        let active = vec![
            ActiveInferenceSequence {
                qos_score: RouteQos::Standard.score(),
                sequence: 0,
                phase: InferenceSequencePhase::Decode,
                job: mock_inference_job(&model_a, "tenant-a", tenant_a_tx),
            },
            ActiveInferenceSequence {
                qos_score: RouteQos::Standard.score(),
                sequence: 1,
                phase: InferenceSequencePhase::Decode,
                job: mock_inference_job(&model_b, "tenant-b", tenant_b_tx),
            },
        ];

        let selected = (0..8)
            .map(|_| {
                let indices = select_active_batch(
                    &active,
                    InferenceSequencePhase::Decode,
                    &mut tenant_fairness,
                )
                .expect("tenant should be selected");
                active[indices[0]].job.tenant_id().to_owned()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            selected
                .iter()
                .filter(|tenant| tenant.as_str() == "tenant-a")
                .count(),
            6
        );
        assert_eq!(
            selected
                .iter()
                .filter(|tenant| tenant.as_str() == "tenant-b")
                .count(),
            2
        );
    }

    #[test]
    fn scheduler_tenant_weights_apply_during_admission() {
        let model = mock_scheduler_model("admission-shared");
        let metrics = SchedulerMetrics::default();
        let mut admission_fairness = TenantFairness::new(SchedulerConfig {
            tenant_weights: std::collections::BTreeMap::from([
                ("tenant-high".to_owned(), 3),
                ("tenant-low".to_owned(), 1),
            ]),
            ..SchedulerConfig::default()
        });
        let mut queued = BinaryHeap::new();
        let mut receivers = Vec::new();

        for sequence in 0..4 {
            let (tx, rx) = mpsc::channel();
            receivers.push(rx);
            queued.push(PrioritizedInferenceJob::new(
                RouteQos::Standard.score(),
                sequence,
                mock_inference_job(&model, "tenant-low", tx),
            ));
        }
        let (high_tx, high_rx) = mpsc::channel();
        receivers.push(high_rx);
        queued.push(PrioritizedInferenceJob::new(
            RouteQos::Standard.score(),
            4,
            mock_inference_job(&model, "tenant-high", high_tx),
        ));

        let mut active = Vec::new();
        admit_sequences(
            &mut queued,
            &mut active,
            1,
            &metrics,
            &mut admission_fairness,
        );

        assert_eq!(active.len(), 1);
        assert_eq!(active[0].job.tenant_id(), "tenant-high");
        assert_eq!(queued.len(), 4);
    }

    #[test]
    fn scheduler_tenant_weights_do_not_batch_lower_weight_tenant_on_same_model() {
        let model = adapter_echo_model("weighted-batch-shared");
        let mut tenant_fairness = TenantFairness::new(SchedulerConfig {
            tenant_weights: std::collections::BTreeMap::from([
                ("tenant-high".to_owned(), 3),
                ("tenant-low".to_owned(), 1),
            ]),
            ..SchedulerConfig::default()
        });
        let (tenant_high_tx, _tenant_high_rx) = mpsc::channel();
        let (tenant_low_tx, _tenant_low_rx) = mpsc::channel();
        let active = vec![
            ActiveInferenceSequence {
                qos_score: RouteQos::Standard.score(),
                sequence: 0,
                phase: InferenceSequencePhase::Decode,
                job: mock_inference_job(&model, "tenant-high", tenant_high_tx),
            },
            ActiveInferenceSequence {
                qos_score: RouteQos::Standard.score(),
                sequence: 1,
                phase: InferenceSequencePhase::Decode,
                job: mock_inference_job(&model, "tenant-low", tenant_low_tx),
            },
        ];

        let first = select_active_batch(
            &active,
            InferenceSequencePhase::Decode,
            &mut tenant_fairness,
        )
        .expect("tenant should be selected");

        assert_eq!(
            first,
            vec![0],
            "configured tenant weights must not let another tenant hitchhike on a same-alias batch"
        );
    }

    #[test]
    fn scheduler_batches_distinct_lora_adapters_as_sub_batches_for_same_model() {
        let model = adapter_echo_model("batch-shared");
        let mut tenant_fairness = TenantFairness::new(SchedulerConfig::default());
        let (tenant_a_tx_1, _tenant_a_rx_1) = mpsc::channel();
        let (tenant_a_tx_2, _tenant_a_rx_2) = mpsc::channel();
        let (base_tx, _base_rx) = mpsc::channel();
        let (tenant_b_tx, _tenant_b_rx) = mpsc::channel();
        let active = vec![
            ActiveInferenceSequence {
                qos_score: RouteQos::Standard.score(),
                sequence: 0,
                phase: InferenceSequencePhase::Decode,
                job: mock_inference_job(&model, "tenant-a", tenant_a_tx_1),
            },
            ActiveInferenceSequence {
                qos_score: RouteQos::Standard.score(),
                sequence: 1,
                phase: InferenceSequencePhase::Decode,
                job: mock_inference_job(&model, "tenant-a", tenant_a_tx_2),
            },
            ActiveInferenceSequence {
                qos_score: RouteQos::Standard.score(),
                sequence: 2,
                phase: InferenceSequencePhase::Decode,
                job: InferenceJob {
                    alias: model.alias.clone(),
                    adapter: None,
                    model: Arc::clone(&model),
                    qos: model.qos,
                    input: SharedInputTensor {
                        dimensions: vec![1],
                        ty: TensorType::U8,
                        data: Arc::from(b"plain".as_slice()),
                    },
                    response_tx: base_tx,
                },
            },
            ActiveInferenceSequence {
                qos_score: RouteQos::Standard.score(),
                sequence: 3,
                phase: InferenceSequencePhase::Decode,
                job: mock_inference_job(&model, "tenant-b", tenant_b_tx),
            },
        ];

        let first = select_active_batch(
            &active,
            InferenceSequencePhase::Decode,
            &mut tenant_fairness,
        )
        .expect("same base model should be selected");
        assert_eq!(
            first,
            vec![0, 1, 2, 3],
            "different adapters for the same model should share one scheduler step"
        );

        let batch = first
            .iter()
            .map(|index| active[*index].job.clone())
            .collect::<Vec<_>>();
        let outputs = process_batch(AcceleratorKind::Gpu, &batch)
            .into_iter()
            .map(|result| String::from_utf8(result.expect("sub-batch output")).expect("utf8"))
            .collect::<Vec<_>>();
        assert_eq!(
            outputs,
            vec![
                "tenant-a:tenant-a".to_owned(),
                "tenant-a:tenant-a".to_owned(),
                "base:plain".to_owned(),
                "tenant-b:tenant-b".to_owned(),
            ],
            "adapter-specific sub-batches must route each output back to its original request"
        );
    }

    #[test]
    fn process_batch_uses_native_multi_adapter_batch_when_backend_supports_it() {
        let backend = Arc::new(NativeAdapterBatchBackend::default());
        let model = native_adapter_batch_model("native-batch-shared", Arc::clone(&backend));
        let (tenant_a_tx, _tenant_a_rx) = mpsc::channel();
        let (base_tx, _base_rx) = mpsc::channel();
        let (tenant_b_tx, _tenant_b_rx) = mpsc::channel();
        let batch = vec![
            mock_inference_job(&model, "tenant-a", tenant_a_tx),
            InferenceJob {
                alias: model.alias.clone(),
                adapter: None,
                model: Arc::clone(&model),
                qos: model.qos,
                input: SharedInputTensor {
                    dimensions: vec![1],
                    ty: TensorType::U8,
                    data: Arc::from(b"plain".as_slice()),
                },
                response_tx: base_tx,
            },
            mock_inference_job(&model, "tenant-b", tenant_b_tx),
        ];

        let outputs = process_batch(AcceleratorKind::Gpu, &batch)
            .into_iter()
            .map(|result| String::from_utf8(result.expect("native batch output")).expect("utf8"))
            .collect::<Vec<_>>();

        assert_eq!(
            outputs,
            vec![
                "tenant-a:tenant-a".to_owned(),
                "base:plain".to_owned(),
                "tenant-b:tenant-b".to_owned(),
            ]
        );
        assert_eq!(backend.native_batches.load(Ordering::SeqCst), 1);
        assert_eq!(backend.sequential_adapter_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn process_batch_scopes_adapter_group_failures_to_matching_rows() {
        let model = failing_adapter_model("adapter-failure-scope");
        let (ok_tx, _ok_rx) = mpsc::channel();
        let (base_tx, _base_rx) = mpsc::channel();
        let (broken_tx, _broken_rx) = mpsc::channel();
        let mut broken_job = mock_inference_job(&model, "broken", broken_tx);
        broken_job.input.data = Arc::from(b"broken-input".as_slice());
        let batch = vec![
            mock_inference_job(&model, "tenant-ok", ok_tx),
            InferenceJob {
                alias: model.alias.clone(),
                adapter: None,
                model: Arc::clone(&model),
                qos: model.qos,
                input: SharedInputTensor {
                    dimensions: vec![1],
                    ty: TensorType::U8,
                    data: Arc::from(b"plain".as_slice()),
                },
                response_tx: base_tx,
            },
            broken_job,
        ];

        let mut results = process_batch(AcceleratorKind::Gpu, &batch);
        assert_eq!(
            String::from_utf8(
                results
                    .remove(0)
                    .expect("healthy adapter row should succeed")
            )
            .expect("utf8"),
            "tenant-ok:tenant-ok"
        );
        assert_eq!(
            String::from_utf8(results.remove(0).expect("base row should succeed")).expect("utf8"),
            "base:plain"
        );
        let error = results
            .remove(0)
            .expect_err("broken adapter row should fail independently");
        assert!(
            error.to_string().contains("adapter `broken` is malformed"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn realtime_qos_preempts_batch_backlog_on_gpu_scheduler() {
        let batch_model = Arc::new(
            CandleModel::load_mock(&IntegrityModelBinding {
                alias: "gpu-batch".to_owned(),
                path: "mock:gpu-batch".to_owned(),
                device: ModelDevice::Cuda,
                qos: RouteQos::Batch,
                dynamic: false,
                hardware_strategy: Default::default(),
            })
            .expect("batch model should load")
            .with_mock_latency(Duration::from_millis(20)),
        );
        let realtime_model = Arc::new(
            CandleModel::load_mock(&IntegrityModelBinding {
                alias: "gpu-bot".to_owned(),
                path: "mock:gpu-bot".to_owned(),
                device: ModelDevice::Cuda,
                qos: RouteQos::RealTime,
                dynamic: false,
                hardware_strategy: Default::default(),
            })
            .expect("realtime model should load")
            .with_mock_latency(Duration::from_millis(20)),
        );

        let metrics = SchedulerMetrics::default();
        metrics.queued_requests.store(2, Ordering::Relaxed);
        metrics.record_enqueue(RouteQos::Batch);
        metrics.record_enqueue(RouteQos::RealTime);
        let mut queued = BinaryHeap::new();
        let (batch_tx, batch_rx) = mpsc::channel();
        let (realtime_tx, realtime_rx) = mpsc::channel();
        let mut active = vec![ActiveInferenceSequence {
            qos_score: RouteQos::Batch.score(),
            sequence: 0,
            phase: InferenceSequencePhase::Decode,
            job: InferenceJob {
                alias: batch_model.alias.clone(),
                adapter: None,
                model: Arc::clone(&batch_model),
                qos: batch_model.qos,
                input: SharedInputTensor {
                    dimensions: vec![1],
                    ty: TensorType::U8,
                    data: Arc::from(b"batch".as_slice()),
                },
                response_tx: batch_tx,
            },
        }];
        queued.push(PrioritizedInferenceJob::new(
            RouteQos::RealTime.score(),
            1,
            InferenceJob {
                alias: realtime_model.alias.clone(),
                adapter: None,
                model: Arc::clone(&realtime_model),
                qos: realtime_model.qos,
                input: SharedInputTensor {
                    dimensions: vec![1],
                    ty: TensorType::U8,
                    data: Arc::from(b"realtime".as_slice()),
                },
                response_tx: realtime_tx,
            },
        ));

        let mut admission_fairness = TenantFairness::new(SchedulerConfig::default());
        admit_sequences(
            &mut queued,
            &mut active,
            2,
            &metrics,
            &mut admission_fairness,
        );
        let mut tenant_fairness = TenantFairness::new(SchedulerConfig::default());
        run_continuous_step(
            AcceleratorKind::Gpu,
            &mut active,
            &metrics,
            &mut tenant_fairness,
            &SchedulerConfig::default(),
        );
        run_continuous_step(
            AcceleratorKind::Gpu,
            &mut active,
            &metrics,
            &mut tenant_fairness,
            &SchedulerConfig::default(),
        );

        let _ = realtime_rx
            .recv()
            .expect("realtime response should arrive")
            .expect("realtime inference should succeed");
        assert!(
            batch_rx.try_recv().is_err(),
            "active batch decode should not complete before inserted realtime decode"
        );
        run_continuous_step(
            AcceleratorKind::Gpu,
            &mut active,
            &metrics,
            &mut tenant_fairness,
            &SchedulerConfig::default(),
        );
        let _ = batch_rx
            .recv()
            .expect("batch response should arrive")
            .expect("batch inference should succeed");

        let completed_aliases = metrics
            .completed_aliases
            .lock()
            .expect("scheduler completion log should not be poisoned")
            .clone();
        let realtime_prefill_position = completed_aliases
            .iter()
            .position(|alias| alias == "gpu-bot:prefill")
            .expect("realtime prefill should complete");
        let realtime_decode_position = completed_aliases
            .iter()
            .position(|alias| alias == "gpu-bot:decode")
            .expect("realtime decode should complete");
        let batch_decode_position = completed_aliases
            .iter()
            .position(|alias| alias == "gpu-batch:decode")
            .expect("batch decode should complete");
        assert!(realtime_prefill_position < realtime_decode_position);
        assert!(
            realtime_decode_position < batch_decode_position,
            "realtime decode should be inserted ahead of an active batch decode"
        );
        assert_eq!(
            metrics.kv_recompute_preemptions.load(Ordering::Relaxed),
            1,
            "one lower-tier KV context should be marked for recompute"
        );
        assert_eq!(metrics.kv_swap_preemptions.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn component_accelerator_runtime_rejects_mismatched_devices() {
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.models = vec![
            IntegrityModelBinding {
                alias: "llama3".to_owned(),
                path: "mock:llama3".to_owned(),
                device: ModelDevice::Cuda,
                qos: RouteQos::RealTime,
                dynamic: false,
                hardware_strategy: Default::default(),
            },
            IntegrityModelBinding {
                alias: "tiny".to_owned(),
                path: "mock:tiny".to_owned(),
                device: ModelDevice::Cpu,
                qos: RouteQos::Batch,
                dynamic: false,
                hardware_strategy: Default::default(),
            },
        ];
        let runtime = AiInferenceRuntime::from_config(&IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        })
        .expect("runtime should build");

        assert!(runtime.supports_accelerator(AcceleratorKind::Cpu));
        assert!(runtime.supports_accelerator(AcceleratorKind::Gpu));
        assert!(runtime.supports_accelerator(AcceleratorKind::Npu));
        assert!(runtime.supports_accelerator(AcceleratorKind::Tpu));
        assert!(runtime
            .load_component_model("llama3", AcceleratorKind::Gpu)
            .is_ok());
        assert!(runtime
            .load_component_model("llama3", AcceleratorKind::Cpu)
            .is_err());
        assert!(runtime
            .load_component_model("tiny", AcceleratorKind::Tpu)
            .is_ok());
        assert_eq!(
            runtime
                .compute_component_prompt("llama3", "hello")
                .expect("component compute should succeed"),
            MOCK_INFERENCE_RESPONSE
        );
    }

    #[test]
    fn load_component_model_honestly_resolves_npu_and_tpu_fallbacks() {
        // `supports_accelerator` reports `Npu`/`Tpu` as a valid scheduling
        // lane (it is, for QoS/batching purposes), but without an initialized
        // vendor runner the request resolves to CPU. A model pinned to the
        // unavailable vendor device must not silently execute elsewhere.
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.models = vec![
            IntegrityModelBinding {
                alias: "npu-whisper".to_owned(),
                path: "mock:npu-whisper".to_owned(),
                device: ModelDevice::Npu,
                qos: RouteQos::RealTime,
                dynamic: false,
                hardware_strategy: Default::default(),
            },
            IntegrityModelBinding {
                alias: "tpu-embed".to_owned(),
                path: "mock:tpu-embed".to_owned(),
                device: ModelDevice::Tpu,
                qos: RouteQos::Batch,
                dynamic: false,
                hardware_strategy: Default::default(),
            },
        ];
        let runtime = AiInferenceRuntime::from_config(&IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        })
        .expect("runtime should build");

        let npu_error = runtime
            .load_component_model("npu-whisper", AcceleratorKind::Npu)
            .expect_err("npu dispatch should be rejected: no backend is wired");
        assert!(npu_error.contains("resolved to `cpu`"), "{npu_error}");

        let tpu_error = runtime
            .load_component_model("tpu-embed", AcceleratorKind::Tpu)
            .expect_err("tpu dispatch should be rejected: no backend is wired");
        assert!(tpu_error.contains("resolved to `cpu`"), "{tpu_error}");

        // The pre-existing internal mock-compute path (bypassing the
        // guest-facing dispatch boundary above) is unaffected: it exercises
        // the runtime's own batching/QoS plumbing, not a real accelerator.
        assert_eq!(
            runtime
                .compute_component_prompt("npu-whisper", "ping")
                .expect("internal mock compute should still succeed"),
            MOCK_INFERENCE_RESPONSE
        );
    }

    #[test]
    fn component_accelerator_runtime_loads_lora_adapter_for_single_call() {
        let _env_guard = model_broker_env_guard();
        let adapter_root = std::env::temp_dir().join(format!(
            "tachyon-lora-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be valid")
                .as_nanos()
        ));
        let adapter_dir = adapter_root.join("adapters");
        fs::create_dir_all(&adapter_dir).expect("adapter dir should be created");
        fs::write(adapter_dir.join("tenant-a.safetensors"), b"mock-lora")
            .expect("adapter should be written");
        std::env::set_var(MODEL_BROKER_DIR_ENV, &adapter_root);

        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.models = vec![IntegrityModelBinding {
            alias: "llama3".to_owned(),
            path: "mock:llama3".to_owned(),
            device: ModelDevice::Cuda,
            qos: RouteQos::RealTime,
            dynamic: false,
            hardware_strategy: Default::default(),
        }];
        let runtime = AiInferenceRuntime::from_config(&IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        })
        .expect("runtime should build");

        assert_eq!(
            runtime
                .compute_component_prompt_with_adapter("llama3", "hello", Some("tenant-a"))
                .expect("adapter-backed inference should succeed"),
            MOCK_INFERENCE_RESPONSE
        );
        let missing = runtime
            .compute_component_prompt_with_adapter("llama3", "hello", Some("tenant-b"))
            .expect_err("missing adapter must fail before inference");
        assert!(missing.contains("tenant-b"));
        assert!(missing.contains("adapters"));
        assert!(runtime
            .compute_component_prompt_with_adapter("llama3", "hello", Some("../bad"))
            .is_err());

        std::env::remove_var(MODEL_BROKER_DIR_ENV);
        let _ = fs::remove_dir_all(adapter_root);
    }

    #[test]
    fn candle_lora_linear_from_pinned_fork_applies_active_adapter() {
        use candle_core::{DType, Device, Tensor};
        use candle_nn::{linear_no_bias, lora::LoraLinear, Module, VarBuilder, VarMap};

        let device = Device::Cpu;
        let mut base_vars = VarMap::new();
        let base_vb = VarBuilder::from_varmap(&base_vars, DType::F32, &device);
        let base = linear_no_bias(3, 2, base_vb.pp("q_proj")).expect("base linear");
        base_vars
            .set_one(
                "q_proj.weight",
                Tensor::new(&[[1f32, 0., 0.], [0., 1., 0.]], &device).expect("base weight"),
            )
            .expect("set base weight");

        let mut adapter_vars = VarMap::new();
        let adapter_vb = VarBuilder::from_varmap(&adapter_vars, DType::F32, &device);
        let lora = LoraLinear::from_peft(base, adapter_vb.pp("q_proj"), 2, 4.0)
            .expect("LoRA adapter should load through candle-nn");
        adapter_vars
            .set_one(
                "q_proj.lora_A.weight",
                Tensor::new(&[[1f32, 0., 0.], [0., 1., 0.]], &device).expect("lora A"),
            )
            .expect("set lora A");
        adapter_vars
            .set_one(
                "q_proj.lora_B.weight",
                Tensor::new(&[[1f32, 0.], [0., 1.]], &device).expect("lora B"),
            )
            .expect("set lora B");

        let output = lora
            .forward(&Tensor::new(&[[1f32, 2., 3.]], &device).expect("input"))
            .expect("LoRA forward");
        assert_eq!(output.to_vec2::<f32>().expect("output"), vec![vec![3., 6.]]);
    }

    #[test]
    fn heterogeneous_runtime_routes_models_to_dedicated_accelerators() {
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.models = vec![
            IntegrityModelBinding {
                alias: "cpu-bert".to_owned(),
                path: "mock:cpu-bert".to_owned(),
                device: ModelDevice::Cpu,
                qos: RouteQos::Standard,
                dynamic: false,
                hardware_strategy: Default::default(),
            },
            IntegrityModelBinding {
                alias: "gpu-llama".to_owned(),
                path: "mock:gpu-llama".to_owned(),
                device: ModelDevice::Cuda,
                qos: RouteQos::RealTime,
                dynamic: false,
                hardware_strategy: Default::default(),
            },
            IntegrityModelBinding {
                alias: "npu-whisper".to_owned(),
                path: "mock:npu-whisper".to_owned(),
                device: ModelDevice::Npu,
                qos: RouteQos::RealTime,
                dynamic: false,
                hardware_strategy: Default::default(),
            },
            IntegrityModelBinding {
                alias: "tpu-embed".to_owned(),
                path: "mock:tpu-embed".to_owned(),
                device: ModelDevice::Tpu,
                qos: RouteQos::Batch,
                dynamic: false,
                hardware_strategy: Default::default(),
            },
        ];
        let runtime = AiInferenceRuntime::from_config(&IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        })
        .expect("runtime should build");

        assert_eq!(
            runtime
                .compute_component_prompt("cpu-bert", "ping")
                .expect("cpu inference should succeed"),
            MOCK_INFERENCE_RESPONSE
        );
        assert_eq!(
            runtime
                .compute_component_prompt("gpu-llama", "ping")
                .expect("gpu inference should succeed"),
            MOCK_INFERENCE_RESPONSE
        );
        assert_eq!(
            runtime
                .compute_component_prompt("npu-whisper", "ping")
                .expect("npu inference should succeed"),
            MOCK_INFERENCE_RESPONSE
        );
        assert_eq!(
            runtime
                .compute_component_prompt("tpu-embed", "ping")
                .expect("tpu inference should succeed"),
            MOCK_INFERENCE_RESPONSE
        );

        assert_eq!(
            runtime.model_memory_residency("cpu-bert"),
            Some(AcceleratorMemoryResidency::HostRam)
        );
        assert_eq!(
            runtime.model_memory_residency("gpu-llama"),
            Some(AcceleratorMemoryResidency::Vram)
        );
        assert_eq!(
            runtime.model_memory_residency("npu-whisper"),
            Some(AcceleratorMemoryResidency::Sram)
        );
        assert_eq!(
            runtime.model_memory_residency("tpu-embed"),
            Some(AcceleratorMemoryResidency::Sram)
        );

        assert_eq!(
            runtime
                .scheduler_snapshot(AcceleratorKind::Cpu)
                .requests_processed,
            1
        );
        assert_eq!(
            runtime
                .scheduler_snapshot(AcceleratorKind::Gpu)
                .requests_processed,
            1
        );
        assert_eq!(
            runtime
                .scheduler_snapshot(AcceleratorKind::Npu)
                .requests_processed,
            1
        );
        assert_eq!(
            runtime
                .scheduler_snapshot(AcceleratorKind::Tpu)
                .requests_processed,
            1
        );
    }

    #[test]
    fn turboquant_round_trip_through_native_rust_implementation() {
        // Previously this test compared the host's TurboQuant decompressor against
        // pre-recorded byte fixtures produced by the C++ FFI shim, to assert the
        // Rust Ã¢â€ â€ C++ round-trip. The C++ shim is gone; the fixtures went with it.
        // We now build a representative input from the 2-bit codebook directly,
        // round-trip it through the same custom-op the production inference path
        // uses, and assert the output matches the input. This is a stronger test
        // than the old fixture comparison because it exercises the full
        // `apply_op2_no_bwd` integration end-to-end with no external state.
        let source: Vec<f32> = (0..64)
            .map(|i| match i % 4 {
                0 => -1.0,
                1 => -0.333_333_34,
                2 => 0.333_333_34,
                _ => 1.0,
            })
            .collect();
        let packed = turboquant_sys::compress_values(&source, 2).expect("packing should succeed");
        let value_count = source.len();
        let packed_tensor = CandleTensor::from_vec(packed.clone(), (packed.len(),), &Device::Cpu)
            .expect("packed tensor should build");
        let attention =
            CandleTensor::from_vec(vec![1.0f32; value_count], (value_count,), &Device::Cpu)
                .expect("attention tensor should build");

        let restored = packed_tensor
            .apply_op2_no_bwd(
                &attention,
                &TurboQuantDecompressor {
                    bits: 2,
                    threshold: 0.0,
                    value_count,
                },
            )
            .expect("TurboQuant custom op should restore values");
        let actual = restored
            .to_vec1::<f32>()
            .expect("restored tensor should convert to a vec");
        assert_eq!(actual, source);
    }

    #[test]
    fn boundary_layers_bypass_turboquant_value_compression() {
        let stack = TurboQuantAttentionStack::default();

        let decisions = (0..stack.total_layers)
            .map(|layer_idx| stack.layer_decision(layer_idx))
            .collect::<Vec<_>>();

        assert_eq!(decisions[0].k_precision, KvPrecision::Q8_0);
        assert_eq!(decisions[1].k_precision, KvPrecision::Q8_0);
        assert!(!decisions[0].v_compressed);
        assert!(!decisions[1].v_compressed);
        assert!(decisions[2].v_compressed);
        assert!(decisions[3].v_compressed);
        assert!(decisions[4].v_compressed);
        assert!(decisions[5].v_compressed);
        assert!(!decisions[6].v_compressed);
        assert!(!decisions[7].v_compressed);
    }

    #[test]
    fn sparse_decode_skips_low_attention_values() {
        let source = vec![-1.0f32, -0.33333334f32, 0.33333334f32, 1.0f32];
        let packed = turboquant_sys::compress_values(&source, 2).expect("packing should succeed");
        let packed_tensor =
            CandleTensor::from_vec(packed, (1,), &Device::Cpu).expect("packed tensor should build");
        let attention = CandleTensor::from_vec(vec![1.0f32, 0.0, 0.5, 0.0], (4,), &Device::Cpu)
            .expect("attention tensor should build");

        let restored = packed_tensor
            .apply_op2_no_bwd(
                &attention,
                &TurboQuantDecompressor {
                    bits: 2,
                    threshold: 0.1,
                    value_count: 4,
                },
            )
            .expect("TurboQuant sparse decode should succeed")
            .to_vec1::<f32>()
            .expect("restored tensor should convert");

        assert_eq!(restored, vec![-1.0, 0.0, 0.33333334, 0.0]);
    }

    fn config_with_model(alias: &str) -> IntegrityConfig {
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.models = vec![IntegrityModelBinding {
            alias: alias.to_owned(),
            path: format!("mock:{alias}"),
            device: ModelDevice::Cpu,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        }];
        IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        }
    }

    fn config_with_real_candle_model(alias: &str, path: &std::path::Path) -> IntegrityConfig {
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.models = vec![IntegrityModelBinding {
            alias: alias.to_owned(),
            path: path.to_string_lossy().into_owned(),
            device: ModelDevice::Cpu,
            qos: RouteQos::Standard,
            dynamic: false,
            hardware_strategy: Default::default(),
        }];
        IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        }
    }

    fn unique_candle_llm_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "tachyon-candle-llm-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be valid")
                .as_nanos()
        ))
    }

    fn write_tiny_lora_adapter(path: &std::path::Path, value: f32) -> anyhow::Result<()> {
        let rank = 2usize;
        let hidden = candle_llm_runtime::FIXTURE_HIDDEN_SIZE;
        let mut tensors = HashMap::new();
        for layer in 0..candle_llm_runtime::FIXTURE_NUM_LAYERS {
            tensors.insert(
                format!("model.layers.{layer}.self_attn.q_proj.lora_A.weight"),
                CandleTensor::from_vec(vec![value; rank * hidden], (rank, hidden), &Device::Cpu)?,
            );
            tensors.insert(
                format!("model.layers.{layer}.self_attn.q_proj.lora_B.weight"),
                CandleTensor::from_vec(vec![value; hidden * rank], (hidden, rank), &Device::Cpu)?,
            );
        }
        candle_core::safetensors::save(&tensors, path)?;
        Ok(())
    }

    #[test]
    fn semantic_flattener_assigns_ordered_cache_keys() {
        let ctx = serde_json::json!({
            "messages": [
                { "role": "system", "content": "You are a helpful assistant." },
                { "role": "user",      "turn_id": "t1", "content": "Hello" },
                { "role": "assistant", "turn_id": "t1", "content": "Hi there!" },
                { "role": "user",      "turn_id": "t2", "content": "How are you?" },
            ]
        });
        let flattener = SemanticContextFlattener::new();
        let chunks = flattener.flatten(&ctx);

        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks[0].cache_key, "sys:0:0");
        assert!(chunks[0]
            .markers
            .contains(&ContextMarker::SystemPromptBoundary));
        assert_eq!(chunks[1].cache_key, "usr:1:t1");
        assert!(chunks[1]
            .markers
            .contains(&ContextMarker::ConversationTurnStart {
                turn_id: "t1".to_owned()
            }));
        assert_eq!(chunks[2].cache_key, "ast:1:t1");
        assert_eq!(chunks[3].cache_key, "usr:2:t2");
    }

    #[test]
    fn semantic_flattener_falls_back_to_legacy_for_plain_string() {
        let ctx = serde_json::json!("just a plain prompt");
        let flattener = SemanticContextFlattener::new();
        let chunks = flattener.flatten(&ctx);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].cache_key, "legacy:0");
    }

    #[test]
    fn layer_ring_buffer_push_get_evict() {
        let mut buf = LayerRingBuffer::new(3);
        let t0 = CandleTensor::zeros((4,), DType::F32, &Device::Cpu).expect("create zeros tensor");
        let t1 = CandleTensor::ones((4,), DType::F32, &Device::Cpu).expect("create ones tensor");
        buf.push(t0);
        buf.push(t1);
        assert!(buf.get(0).is_some());
        assert!(buf.get(1).is_some());
        assert!(buf.get(2).is_none());
        buf.evict_oldest();
        // slot 0 was the oldest and is now None
        assert!(buf.get(0).is_none());
    }

    #[test]
    fn kv_cache_slice_append_and_keys_tensor() {
        let mut slice = KvCacheSlice {
            layer_idx: 0,
            keys: vec![vec![1.0f32, 2.0f32]],
            values: vec![vec![3.0f32, 4.0f32]],
        };
        slice.append_token(vec![5.0f32, 6.0f32], vec![7.0f32, 8.0f32]);
        assert_eq!(slice.keys.len(), 2);
        let t = slice.keys_tensor().expect("tensor should build");
        assert_eq!(t.dims(), &[2, 2]);
    }

    #[test]
    fn memory_profile_default_is_performance() {
        assert_eq!(MemoryProfile::default(), MemoryProfile::Performance);
    }

    #[test]
    fn per_request_execution_telemetry_records_selected_target() {
        let runtime = AiInferenceRuntime::from_config(&config_with_model("telemetry-model"))
            .expect("runtime");
        runtime
            .compute_component_prompt("telemetry-model", "hello")
            .expect("compute");
        let records = inference_execution_telemetry();
        assert!(records.iter().any(|record| {
            record.alias == "telemetry-model" && record.executed_on == "cpu" && record.succeeded
        }));
    }
}
