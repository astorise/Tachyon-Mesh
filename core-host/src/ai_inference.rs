use anyhow::{anyhow, Result};
use candle_core::{
    bail as candle_bail, CpuStorage, CustomOp2, DType, Device, Layout, Shape,
    Tensor as CandleTensor,
};
use candle_nn::VarMap;
use std::{
    cmp::Ordering as CmpOrdering,
    collections::{BinaryHeap, HashMap},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Condvar, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use wasmtime::format_err;
use wasmtime_wasi_nn::{
    backend::{self, BackendError, BackendExecutionContext, BackendGraph, Id, NamedTensor},
    wit::{Tensor as WasiTensor, TensorType},
    witx::WasiNnCtx,
    Backend as WasiNnBackend, Graph as WasiGraph, GraphRegistry, Registry as WasiRegistry,
};

use crate::{IntegrityConfig, IntegrityModelBinding, RouteQos};

const MOCK_INFERENCE_RESPONSE: &str = "MOCK_LLM_RESPONSE";
const DEFAULT_BATCH_SIZE: usize = 32;
const DEFAULT_BATCH_WINDOW: Duration = Duration::from_millis(25);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum AcceleratorKind {
    #[default]
    Cpu,
    Gpu,
    Npu,
    Tpu,
}

#[derive(Clone)]
pub(crate) struct AiInferenceRuntime {
    schedulers: HashMap<AcceleratorKind, CandleBatchScheduler>,
    models: HashMap<String, Arc<CandleModel>>,
}

#[cfg(test)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SchedulerSnapshot {
    pub(crate) batches_processed: usize,
    pub(crate) requests_processed: usize,
    pub(crate) max_batch_size: usize,
    pub(crate) completed_aliases: Vec<String>,
}

impl AiInferenceRuntime {
    pub(crate) fn from_config(config: &IntegrityConfig) -> Result<Self> {
        let mut schedulers = HashMap::from([(
            AcceleratorKind::Cpu,
            CandleBatchScheduler::new(DEFAULT_BATCH_SIZE, DEFAULT_BATCH_WINDOW),
        )]);
        let mut models = HashMap::new();

        for route in &config.routes {
            for binding in &route.models {
                if models.contains_key(&binding.alias) {
                    return Err(anyhow!(
                        "Integrity Validation Failed: model alias `{}` must be globally unique",
                        binding.alias
                    ));
                }
                schedulers
                    .entry(AcceleratorKind::from_model_device(&binding.device))
                    .or_insert_with(|| {
                        CandleBatchScheduler::new(DEFAULT_BATCH_SIZE, DEFAULT_BATCH_WINDOW)
                    });
                models.insert(
                    binding.alias.clone(),
                    Arc::new(CandleModel::load_mock(binding)?),
                );
            }
        }

        Ok(Self { schedulers, models })
    }

    pub(crate) fn build_wasi_nn_ctx(&self) -> WasiNnCtx {
        let registry = AliasGraphRegistry {
            graphs: self
                .models
                .iter()
                .map(|(alias, model)| {
                    (
                        alias.clone(),
                        WasiGraph::from(Box::new(CandleModelGraph {
                            model: Arc::clone(model),
                            scheduler: self
                                .scheduler_for(model.accelerator)
                                .expect("accelerator scheduler should exist for model"),
                        }) as Box<dyn BackendGraph>),
                    )
                })
                .collect(),
        };
        let backends = [WasiNnBackend::from(backend::onnx::OnnxBackend::default())];
        WasiNnCtx::new(backends, WasiRegistry::from(registry))
    }

    #[cfg(test)]
    pub(crate) fn loaded_model_aliases(&self) -> Vec<String> {
        let mut aliases = self.models.keys().cloned().collect::<Vec<_>>();
        aliases.sort();
        aliases
    }

    #[cfg(test)]
    pub(crate) fn scheduler_snapshot(&self, accelerator: AcceleratorKind) -> SchedulerSnapshot {
        self.scheduler_for(accelerator)
            .map(|scheduler| scheduler.snapshot())
            .unwrap_or_default()
    }

    pub(crate) fn supports_accelerator(&self, accelerator: AcceleratorKind) -> bool {
        self.schedulers.contains_key(&accelerator)
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

        let model = self
            .models
            .get(alias)
            .ok_or_else(|| format!("model alias `{alias}` is not loaded"))?;
        if model.accelerator != accelerator {
            return Err(format!(
                "model alias `{alias}` requires `{}` but `{}` was requested",
                model.accelerator.as_str(),
                accelerator.as_str()
            ));
        }
        Ok(())
    }

    pub(crate) fn compute_component_prompt(
        &self,
        alias: &str,
        prompt: &str,
    ) -> Result<String, String> {
        let model = self
            .models
            .get(alias)
            .ok_or_else(|| format!("model alias `{alias}` is not loaded"))?;
        let output = self
            .scheduler_for(model.accelerator)
            .ok_or_else(|| {
                format!(
                    "{} accelerator is unavailable on this host",
                    model.accelerator.as_str()
                )
            })?
            .infer(
                Arc::clone(model),
                WasiTensor::new(vec![prompt.len() as u32], TensorType::U8, prompt.as_bytes().to_vec()),
            )
            .map_err(|error| error.to_string())?;
        String::from_utf8(output.data).map_err(|error| error.to_string())
    }

    fn scheduler_for(&self, accelerator: AcceleratorKind) -> Option<CandleBatchScheduler> {
        self.schedulers.get(&accelerator).cloned()
    }
}

impl AcceleratorKind {
    fn from_model_device(device: &crate::ModelDevice) -> Self {
        match device {
            crate::ModelDevice::Cpu => Self::Cpu,
            crate::ModelDevice::Cuda | crate::ModelDevice::Metal => Self::Gpu,
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

#[derive(Clone)]
struct CandleBatchScheduler {
    shared: Arc<SchedulerShared>,
    #[cfg_attr(not(test), allow(dead_code))]
    metrics: Arc<SchedulerMetrics>,
}

impl CandleBatchScheduler {
    fn new(batch_size: usize, batch_window: Duration) -> Self {
        let shared = Arc::new(SchedulerShared::default());
        let metrics = Arc::new(SchedulerMetrics::default());
        let worker_metrics = Arc::clone(&metrics);
        let worker_shared = Arc::clone(&shared);

        thread::Builder::new()
            .name("tachyon-candle-batcher".to_owned())
            .spawn(move || run_scheduler(worker_shared, worker_metrics, batch_size, batch_window))
            .expect("AI inference batch scheduler thread should start");

        Self { shared, metrics }
    }

    fn infer(&self, model: Arc<CandleModel>, input: WasiTensor) -> Result<WasiTensor, BackendError> {
        let response_rx = self.enqueue(model, input);
        response_rx.recv().map_err(|_| {
            backend_access_error("AI inference response channel closed unexpectedly")
        })?
    }

    fn enqueue(
        &self,
        model: Arc<CandleModel>,
        input: WasiTensor,
    ) -> mpsc::Receiver<Result<WasiTensor, BackendError>> {
        let (response_tx, response_rx) = mpsc::channel();
        let mut state = self
            .shared
            .state
            .lock()
            .expect("AI inference scheduler queue should not be poisoned");
        let sequence = state.next_sequence;
        state.next_sequence = state.next_sequence.saturating_add(1);
        state.queue.push(PrioritizedInferenceJob::new(
            model.qos.score(),
            sequence,
            InferenceJob {
                alias: model.alias.clone(),
                model,
                input,
                response_tx,
            },
        ));
        drop(state);
        self.shared.notify.notify_one();
        response_rx
    }

    #[cfg(test)]
    fn snapshot(&self) -> SchedulerSnapshot {
        SchedulerSnapshot {
            batches_processed: self.metrics.batches_processed.load(Ordering::Relaxed),
            requests_processed: self.metrics.requests_processed.load(Ordering::Relaxed),
            max_batch_size: self.metrics.max_batch_size.load(Ordering::Relaxed),
            completed_aliases: self
                .metrics
                .completed_aliases
                .lock()
                .expect("scheduler completion log should not be poisoned")
                .clone(),
        }
    }
}

#[derive(Default)]
struct SchedulerMetrics {
    batches_processed: AtomicUsize,
    requests_processed: AtomicUsize,
    max_batch_size: AtomicUsize,
    #[cfg(test)]
    completed_aliases: Mutex<Vec<String>>,
}

impl SchedulerMetrics {
    fn record_batch(&self, batch: &[InferenceJob]) {
        self.batches_processed.fetch_add(1, Ordering::Relaxed);
        self.requests_processed
            .fetch_add(batch.len(), Ordering::Relaxed);
        self.max_batch_size.fetch_max(batch.len(), Ordering::Relaxed);
        #[cfg(test)]
        if let Some(first) = batch.first() {
            self.completed_aliases
                .lock()
                .expect("scheduler completion log should not be poisoned")
                .push(first.alias.clone());
        }
    }
}

#[derive(Default)]
struct SchedulerShared {
    state: Mutex<SchedulerState>,
    notify: Condvar,
}

#[derive(Default)]
struct SchedulerState {
    queue: BinaryHeap<PrioritizedInferenceJob>,
    next_sequence: u64,
}

struct InferenceJob {
    alias: String,
    model: Arc<CandleModel>,
    input: WasiTensor,
    response_tx: mpsc::Sender<Result<WasiTensor, BackendError>>,
}

struct PrioritizedInferenceJob {
    qos_score: u16,
    sequence: u64,
    job: InferenceJob,
}

impl PrioritizedInferenceJob {
    fn new(qos_score: u16, sequence: u64, job: InferenceJob) -> Self {
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
    shared: Arc<SchedulerShared>,
    metrics: Arc<SchedulerMetrics>,
    batch_size: usize,
    batch_window: Duration,
) {
    loop {
        let first = wait_for_next_job(&shared);
        let batch_alias = first.alias.clone();
        let mut batch = vec![first];
        let deadline = Instant::now() + batch_window;

        while batch.len() < batch_size {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let Some(job) = try_take_compatible_job(&shared, &batch_alias, remaining) else {
                break;
            };
            batch.push(job);
        }

        let results = process_batch(&batch);
        metrics.record_batch(&batch);
        for (job, result) in batch.into_iter().zip(results.into_iter()) {
            let _ = job.response_tx.send(result);
        }
        age_waiting_jobs(&shared);
    }
}

fn wait_for_next_job(shared: &SchedulerShared) -> InferenceJob {
    let mut state = shared
        .state
        .lock()
        .expect("AI inference scheduler queue should not be poisoned");
    loop {
        if let Some(job) = state.queue.pop() {
            return job.job;
        }
        state = shared
            .notify
            .wait(state)
            .expect("AI inference scheduler condvar should not be poisoned");
    }
}

fn try_take_compatible_job(
    shared: &SchedulerShared,
    alias: &str,
    wait_for: Duration,
) -> Option<InferenceJob> {
    let mut state = shared
        .state
        .lock()
        .expect("AI inference scheduler queue should not be poisoned");
    let deadline = Instant::now() + wait_for;

    loop {
        let mut deferred = Vec::new();
        let mut selected = None;
        while let Some(job) = state.queue.pop() {
            if job.job.alias == alias {
                selected = Some(job.job);
                break;
            }
            deferred.push(job);
        }
        for job in deferred {
            state.queue.push(job);
        }
        if selected.is_some() {
            return selected;
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let (next_state, _) = shared
            .notify
            .wait_timeout(state, remaining)
            .expect("AI inference scheduler condvar should not be poisoned");
        state = next_state;
    }
}

fn age_waiting_jobs(shared: &SchedulerShared) {
    let mut state = shared
        .state
        .lock()
        .expect("AI inference scheduler queue should not be poisoned");
    if state.queue.is_empty() {
        return;
    }
    let aged = state
        .queue
        .drain()
        .map(PrioritizedInferenceJob::age)
        .collect::<Vec<_>>();
    for job in aged {
        state.queue.push(job);
    }
}

fn process_batch(batch: &[InferenceJob]) -> Vec<Result<WasiTensor, BackendError>> {
    let longest_prompt = batch
        .iter()
        .map(|job| job.input.data.len().max(1))
        .max()
        .unwrap_or(1);
    let model = Arc::clone(&batch[0].model);
    #[cfg(test)]
    if model.mock_latency > Duration::ZERO {
        thread::sleep(model.mock_latency);
    }
    match model.run_mock_batch(batch.len(), longest_prompt) {
        Ok(output) => batch.iter().map(|_| Ok(output.clone())).collect(),
        Err(error) => {
            let message = error.to_string();
            batch
                .iter()
                .map(|_| Err(backend_access_error(message.clone())))
                .collect()
        }
    }
}

#[derive(Clone)]
struct CandleModelGraph {
    model: Arc<CandleModel>,
    scheduler: CandleBatchScheduler,
}

impl BackendGraph for CandleModelGraph {
    fn init_execution_context(&self) -> Result<wasmtime_wasi_nn::ExecutionContext, BackendError> {
        Ok((Box::new(CandleExecutionContext {
            model: Arc::clone(&self.model),
            scheduler: self.scheduler.clone(),
            input: None,
            output: None,
        }) as Box<dyn BackendExecutionContext>)
            .into())
    }
}

struct CandleExecutionContext {
    model: Arc<CandleModel>,
    scheduler: CandleBatchScheduler,
    input: Option<WasiTensor>,
    output: Option<WasiTensor>,
}

impl BackendExecutionContext for CandleExecutionContext {
    fn set_input(&mut self, id: Id, tensor: &WasiTensor) -> Result<(), BackendError> {
        match id.index() {
            Some(0) => {
                self.input = Some(tensor.clone());
                Ok(())
            }
            _ => Err(backend_access_error(
                "mock Candle backend only supports input tensor 0",
            )),
        }
    }

    fn get_output(&mut self, id: Id) -> Result<WasiTensor, BackendError> {
        match id.index() {
            Some(0) => self
                .output
                .clone()
                .ok_or_else(|| backend_access_error("no AI inference output is available yet")),
            _ => Err(backend_access_error(
                "mock Candle backend only supports output tensor 0",
            )),
        }
    }

    fn compute(
        &mut self,
        inputs: Option<Vec<NamedTensor>>,
    ) -> Result<Option<Vec<NamedTensor>>, BackendError> {
        let use_named_io = inputs.is_some();
        let input = match inputs {
            Some(mut inputs) => inputs
                .drain(..)
                .next()
                .map(|named| named.tensor)
                .ok_or_else(|| {
                    backend_access_error("wasi-nn compute requires at least one input tensor")
                })?,
            None => self.input.clone().ok_or_else(|| {
                backend_access_error("wasi-nn input tensor 0 must be set before compute")
            })?,
        };

        let output = self.scheduler.infer(Arc::clone(&self.model), input)?;
        self.output = Some(output.clone());

        if use_named_io {
            Ok(Some(vec![NamedTensor {
                name: "output".to_owned(),
                tensor: output,
            }]))
        } else {
            Ok(None)
        }
    }
}

struct CandleModel {
    alias: String,
    path: String,
    requested_target: String,
    accelerator: AcceleratorKind,
    qos: RouteQos,
    attention_stack: TurboQuantAttentionStack,
    _variables: VarMap,
    #[cfg(test)]
    mock_latency: Duration,
}

impl CandleModel {
    fn load_mock(binding: &IntegrityModelBinding) -> Result<Self> {
        if binding.path.trim().is_empty() {
            return Err(anyhow!(
                "Integrity Validation Failed: model alias `{}` must declare a non-empty `path`",
                binding.alias
            ));
        }

        Ok(Self {
            alias: binding.alias.clone(),
            path: binding.path.clone(),
            requested_target: binding.device.as_str().to_owned(),
            accelerator: AcceleratorKind::from_model_device(&binding.device),
            qos: binding.qos,
            attention_stack: TurboQuantAttentionStack::default(),
            _variables: VarMap::new(),
            #[cfg(test)]
            mock_latency: Duration::ZERO,
        })
    }

    #[cfg(test)]
    fn with_mock_latency(mut self, mock_latency: Duration) -> Self {
        self.mock_latency = mock_latency;
        self
    }

    fn run_mock_batch(
        &self,
        batch_size: usize,
        longest_prompt: usize,
    ) -> Result<WasiTensor, BackendError> {
        let _prompt_batch = CandleTensor::zeros(
            (batch_size.max(1), longest_prompt.max(1)),
            DType::F32,
            &Device::Cpu,
        )
        .map_err(|error| {
            backend_access_error(format!(
                "failed to prepare mock Candle batch for `{}` on requested `{}` from `{}`: {error}",
                self.alias, self.requested_target, self.path
            ))
        })?;
        self.attention_stack
            .run_mock_prompt(batch_size.max(1), longest_prompt.max(1))
            .map_err(|error| {
                backend_access_error(format!(
                    "failed to execute TurboQuant attention mock for `{}` on requested `{}` from `{}`: {error}",
                    self.alias, self.requested_target, self.path
                ))
            })?;

        Ok(WasiTensor::new(
            vec![MOCK_INFERENCE_RESPONSE.len() as u32],
            TensorType::U8,
            MOCK_INFERENCE_RESPONSE.as_bytes().to_vec(),
        ))
    }
}

#[derive(Default)]
struct AliasGraphRegistry {
    graphs: HashMap<String, WasiGraph>,
}

impl GraphRegistry for AliasGraphRegistry {
    fn get(&self, name: &str) -> Option<&WasiGraph> {
        self.graphs.get(name)
    }

    fn get_mut(&mut self, name: &str) -> Option<&mut WasiGraph> {
        self.graphs.get_mut(name)
    }
}

fn backend_access_error(message: impl Into<String>) -> BackendError {
    BackendError::BackendAccess(format_err!("{}", message.into()))
}

#[allow(dead_code)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IntegrityConfig, IntegrityRoute, ModelDevice};
    use std::{fs, path::PathBuf};

    #[test]
    fn runtime_preloads_model_aliases_from_config() {
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.models = vec![
            IntegrityModelBinding {
                alias: "llama3".to_owned(),
                path: "/models/llama3.gguf".to_owned(),
                device: ModelDevice::Cuda,
                qos: RouteQos::Standard,
            },
            IntegrityModelBinding {
                alias: "tiny".to_owned(),
                path: "/models/tiny.gguf".to_owned(),
                device: ModelDevice::Cpu,
                qos: RouteQos::Standard,
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
                .get("llama3")
                .expect("model should exist")
                .clone();
            handles.push(thread::spawn(move || {
                barrier.wait();
                scheduler
                    .infer(model, WasiTensor::new(vec![1], TensorType::U8, b"hello".to_vec()))
                    .expect("inference should succeed")
            }));
        }

        for handle in handles {
            let output = handle.join().expect("worker should join");
            assert_eq!(output.data, MOCK_INFERENCE_RESPONSE.as_bytes());
        }

        let snapshot = runtime.scheduler_snapshot(AcceleratorKind::Cpu);
        assert_eq!(snapshot.requests_processed, 8);
        assert_eq!(snapshot.batches_processed, 1);
        assert_eq!(snapshot.max_batch_size, 8);
    }

    #[test]
    fn realtime_qos_preempts_batch_backlog_on_gpu_scheduler() {
        let scheduler = CandleBatchScheduler::new(1, Duration::from_millis(0));
        let batch_model = Arc::new(
            CandleModel::load_mock(&IntegrityModelBinding {
                alias: "gpu-batch".to_owned(),
                path: "/models/gpu-batch.gguf".to_owned(),
                device: ModelDevice::Cuda,
                qos: RouteQos::Batch,
            })
            .expect("batch model should load")
            .with_mock_latency(Duration::from_millis(20)),
        );
        let realtime_model = Arc::new(
            CandleModel::load_mock(&IntegrityModelBinding {
                alias: "gpu-bot".to_owned(),
                path: "/models/gpu-bot.gguf".to_owned(),
                device: ModelDevice::Cuda,
                qos: RouteQos::RealTime,
            })
            .expect("realtime model should load")
            .with_mock_latency(Duration::from_millis(20)),
        );

        let mut receivers = Vec::new();
        for _ in 0..6 {
            receivers.push(scheduler.enqueue(
                Arc::clone(&batch_model),
                WasiTensor::new(vec![1], TensorType::U8, b"batch".to_vec()),
            ));
        }
        thread::sleep(Duration::from_millis(5));
        let realtime_rx = scheduler.enqueue(
            Arc::clone(&realtime_model),
            WasiTensor::new(vec![1], TensorType::U8, b"realtime".to_vec()),
        );

        for receiver in receivers {
            let _ = receiver
                .recv()
                .expect("batch response should arrive")
                .expect("batch inference should succeed");
        }
        let _ = realtime_rx
            .recv()
            .expect("realtime response should arrive")
            .expect("realtime inference should succeed");

        let snapshot = scheduler.snapshot();
        assert!(
            snapshot.completed_aliases.len() >= 2,
            "scheduler should have processed at least two batches"
        );
        assert_eq!(snapshot.completed_aliases[0], "gpu-batch");
        assert_eq!(snapshot.completed_aliases[1], "gpu-bot");
    }

    #[test]
    fn component_accelerator_runtime_rejects_unavailable_or_mismatched_devices() {
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.models = vec![
            IntegrityModelBinding {
                alias: "llama3".to_owned(),
                path: "/models/llama3.gguf".to_owned(),
                device: ModelDevice::Cuda,
                qos: RouteQos::RealTime,
            },
            IntegrityModelBinding {
                alias: "tiny".to_owned(),
                path: "/models/tiny.gguf".to_owned(),
                device: ModelDevice::Cpu,
                qos: RouteQos::Batch,
            },
        ];
        let runtime = AiInferenceRuntime::from_config(&IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        })
        .expect("runtime should build");

        assert!(runtime.supports_accelerator(AcceleratorKind::Cpu));
        assert!(runtime.supports_accelerator(AcceleratorKind::Gpu));
        assert!(!runtime.supports_accelerator(AcceleratorKind::Tpu));
        assert!(runtime.load_component_model("llama3", AcceleratorKind::Gpu).is_ok());
        assert!(
            runtime
                .load_component_model("llama3", AcceleratorKind::Cpu)
                .is_err()
        );
        assert!(
            runtime
                .load_component_model("tiny", AcceleratorKind::Tpu)
                .is_err()
        );
        assert_eq!(
            runtime
                .compute_component_prompt("llama3", "hello")
                .expect("component compute should succeed"),
            MOCK_INFERENCE_RESPONSE
        );
    }

    #[test]
    fn turboquant_ffi_match() {
        let fixture_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../turboquant-sys/fixtures");
        let source_bytes =
            fs::read(fixture_dir.join("v_tensor_f32.bin")).expect("source fixture should exist");
        let packed_bytes =
            fs::read(fixture_dir.join("v_tensor_tq.bin")).expect("packed fixture should exist");
        let value_count = source_bytes.len() / std::mem::size_of::<f32>();
        let packed_tensor =
            CandleTensor::from_vec(packed_bytes.clone(), (packed_bytes.len(),), &Device::Cpu)
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
            .expect("TurboQuant custom op should restore fixture values");
        let actual = restored
            .to_vec1::<f32>()
            .expect("restored tensor should convert to a vec");
        let actual_bytes = actual
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();

        assert_eq!(actual_bytes, source_bytes);
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
            path: format!("/models/{alias}.gguf"),
            device: ModelDevice::Cpu,
            qos: RouteQos::Standard,
        }];
        IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        }
    }
}
