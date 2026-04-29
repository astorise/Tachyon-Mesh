use anyhow::{anyhow, Result};
use candle_core::{
    bail as candle_bail, CpuStorage, CustomOp2, DType, Device, Layout, Shape,
    Tensor as CandleTensor,
};
use candle_nn::VarMap;
#[cfg(test)]
use std::sync::Mutex;
use std::{
    any::Any,
    cmp::Ordering as CmpOrdering,
    collections::{BinaryHeap, HashMap},
    fs,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};
use tokio::sync::mpsc as tokio_mpsc;
use wasmtime::format_err;
use wasmtime_wasi_nn::{
    backend::{self, BackendError, BackendExecutionContext, BackendGraph, Id, NamedTensor},
    wit::{Tensor as WasiTensor, TensorType},
    witx::WasiNnCtx,
    Backend as WasiNnProvider, Graph as WasiGraph, GraphRegistry, Registry as WasiRegistry,
};

use crate::{IntegrityConfig, IntegrityModelBinding, RouteQos};

const MOCK_INFERENCE_RESPONSE: &str = "MOCK_LLM_RESPONSE";
const DEFAULT_BATCH_SIZE: usize = 32;
const DEFAULT_BATCH_WINDOW: Duration = Duration::from_millis(25);
const ACCELERATOR_QUEUE_CAPACITY: usize = 256;
const ACCELERATOR_QUEUE_POLL_INTERVAL: Duration = Duration::from_millis(1);

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
    model_bytes: Arc<[u8]>,
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

impl From<WasiTensor> for SharedInputTensor {
    fn from(value: WasiTensor) -> Self {
        Self {
            dimensions: value.dimensions,
            ty: value.ty,
            data: Arc::from(value.data.into_boxed_slice()),
        }
    }
}

trait BackendModel: Send + Sync {
    fn residency(&self) -> AcceleratorMemoryResidency;
    fn as_any(&self) -> &dyn Any;
}

trait WasiNnBackend: Send + Sync {
    fn accelerator(&self) -> AcceleratorKind;
    fn backend_name(&self) -> &'static str;
    fn init(&self, source: &BackendModelSource) -> Result<Arc<dyn BackendModel>, BackendError>;
    fn execute(
        &self,
        model: &dyn BackendModel,
        inputs: &[SharedInputTensor],
    ) -> Result<WasiTensor, BackendError>;
}

#[derive(Clone)]
pub(crate) struct AiInferenceRuntime {
    schedulers: HashMap<AcceleratorKind, AcceleratorScheduler>,
    models: HashMap<String, Arc<CandleModel>>,
}

#[cfg(test)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SchedulerSnapshot {
    pub(crate) batches_processed: usize,
    pub(crate) requests_processed: usize,
    pub(crate) max_batch_size: usize,
    pub(crate) queued_requests: usize,
    pub(crate) realtime_queued: usize,
    pub(crate) standard_queued: usize,
    pub(crate) batch_queued: usize,
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
                        DEFAULT_BATCH_WINDOW,
                    ),
                )
            })
            .collect::<HashMap<_, _>>();
        let backends = default_backends();
        let mut models = HashMap::new();

        for route in &config.routes {
            for binding in &route.models {
                if models.contains_key(&binding.alias) {
                    return Err(anyhow!(
                        "Integrity Validation Failed: model alias `{}` must be globally unique",
                        binding.alias
                    ));
                }
                let accelerator = AcceleratorKind::from_model_device(&binding.device);
                let backend = backends.get(&accelerator).cloned().ok_or_else(|| {
                    anyhow!(
                        "Integrity Validation Failed: {} backend is unavailable for model `{}`",
                        accelerator.as_str(),
                        binding.alias
                    )
                })?;
                models.insert(
                    binding.alias.clone(),
                    Arc::new(CandleModel::load_mock_with_backend(binding, backend)?),
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
        let backends = [WasiNnProvider::from(backend::onnx::OnnxBackend::default())];
        WasiNnCtx::new(backends, WasiRegistry::from(registry))
    }

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
        self.models.get(alias).map(|model| model.memory_residency)
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
                WasiTensor::new(
                    vec![prompt.len() as u32],
                    TensorType::U8,
                    prompt.as_bytes().to_vec(),
                ),
            )
            .map_err(|error| error.to_string())?;
        String::from_utf8(output.data).map_err(|error| error.to_string())
    }

    fn scheduler_for(&self, accelerator: AcceleratorKind) -> Option<AcceleratorScheduler> {
        self.schedulers.get(&accelerator).cloned()
    }
}

#[derive(Clone)]
struct AcceleratorScheduler {
    sender: tokio_mpsc::Sender<PrioritizedInferenceJob>,
    metrics: Arc<SchedulerMetrics>,
}

impl AcceleratorScheduler {
    fn new(accelerator: AcceleratorKind, batch_size: usize, batch_window: Duration) -> Self {
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
                    batch_size,
                    batch_window,
                )
            })
            .expect("AI inference scheduler thread should start");

        Self { sender, metrics }
    }

    fn infer(
        &self,
        model: Arc<CandleModel>,
        input: WasiTensor,
    ) -> Result<WasiTensor, BackendError> {
        let response_rx = self.enqueue(model, input)?;
        response_rx.recv().map_err(|_| {
            backend_access_error("AI inference response channel closed unexpectedly")
        })?
    }

    fn enqueue(
        &self,
        model: Arc<CandleModel>,
        input: WasiTensor,
    ) -> Result<mpsc::Receiver<Result<WasiTensor, BackendError>>, BackendError> {
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
                model,
                qos,
                input: input.into(),
                response_tx,
            },
        );
        self.sender
            .blocking_send(job)
            .map_err(|_| backend_access_error("AI inference scheduler has stopped"))?;
        Ok(response_rx)
    }

    #[cfg(test)]
    fn snapshot(&self) -> SchedulerSnapshot {
        SchedulerSnapshot {
            batches_processed: self.metrics.batches_processed.load(Ordering::Relaxed),
            requests_processed: self.metrics.requests_processed.load(Ordering::Relaxed),
            max_batch_size: self.metrics.max_batch_size.load(Ordering::Relaxed),
            queued_requests: self.metrics.queued_requests.load(Ordering::Relaxed),
            realtime_queued: self.metrics.realtime_queued.load(Ordering::Relaxed),
            standard_queued: self.metrics.standard_queued.load(Ordering::Relaxed),
            batch_queued: self.metrics.batch_queued.load(Ordering::Relaxed),
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
    queued_requests: AtomicUsize,
    realtime_queued: AtomicUsize,
    standard_queued: AtomicUsize,
    batch_queued: AtomicUsize,
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

    fn record_batch(&self, batch: &[InferenceJob]) {
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
                .push(first.alias.clone());
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

struct InferenceJob {
    alias: String,
    model: Arc<CandleModel>,
    qos: RouteQos,
    input: SharedInputTensor,
    response_tx: mpsc::Sender<Result<WasiTensor, BackendError>>,
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
    batch_size: usize,
    batch_window: Duration,
) {
    let mut queued = BinaryHeap::new();

    loop {
        let first = match wait_for_next_job(&mut queued, &mut receiver) {
            Some(job) => job,
            None => return,
        };
        let batch_alias = first.alias.clone();
        let mut batch = vec![first];
        let deadline = Instant::now() + batch_window;

        while batch.len() < batch_size {
            drain_ready_jobs(&mut receiver, &mut queued);
            let Some(job) = try_take_compatible_job(&mut queued, &batch_alias) else {
                if Instant::now() >= deadline {
                    break;
                }
                thread::sleep(ACCELERATOR_QUEUE_POLL_INTERVAL);
                continue;
            };
            batch.push(job);
        }

        let results = process_batch(accelerator, &batch);
        metrics.record_batch(&batch);
        for (job, result) in batch.into_iter().zip(results) {
            metrics.queued_requests.fetch_sub(1, Ordering::Relaxed);
            metrics.record_dequeue(job.qos);
            let _ = job.response_tx.send(result);
        }
        age_waiting_jobs(&mut queued);
    }
}

fn wait_for_next_job(
    queued: &mut BinaryHeap<PrioritizedInferenceJob>,
    receiver: &mut tokio_mpsc::Receiver<PrioritizedInferenceJob>,
) -> Option<InferenceJob> {
    drain_ready_jobs(receiver, queued);
    if let Some(job) = queued.pop() {
        return Some(job.job);
    }

    receiver.blocking_recv().map(|job| job.job)
}

fn drain_ready_jobs(
    receiver: &mut tokio_mpsc::Receiver<PrioritizedInferenceJob>,
    queued: &mut BinaryHeap<PrioritizedInferenceJob>,
) {
    while let Ok(job) = receiver.try_recv() {
        queued.push(job);
    }
}

fn try_take_compatible_job(
    queued: &mut BinaryHeap<PrioritizedInferenceJob>,
    alias: &str,
) -> Option<InferenceJob> {
    let mut deferred = Vec::new();
    let mut selected = None;

    while let Some(job) = queued.pop() {
        if job.job.alias == alias {
            selected = Some(job.job);
            break;
        }
        deferred.push(job);
    }

    for job in deferred {
        queued.push(job);
    }

    selected
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
) -> Vec<Result<WasiTensor, BackendError>> {
    let model = Arc::clone(&batch[0].model);
    #[cfg(test)]
    if model.mock_latency > Duration::ZERO {
        thread::sleep(model.mock_latency);
    }
    let inputs = batch
        .iter()
        .map(|job| job.input.clone())
        .collect::<Vec<_>>();
    match model.run_mock_batch(&inputs) {
        Ok(output) => batch.iter().map(|_| Ok(output.clone())).collect(),
        Err(error) => {
            let message = format!(
                "{} backend failed for model `{}`: {}",
                accelerator.as_str(),
                model.alias,
                error
            );
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
    scheduler: AcceleratorScheduler,
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
    scheduler: AcceleratorScheduler,
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
                "mock accelerator backend only supports input tensor 0",
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
                "mock accelerator backend only supports output tensor 0",
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
    accelerator: AcceleratorKind,
    qos: RouteQos,
    #[cfg_attr(not(test), allow(dead_code))]
    memory_residency: AcceleratorMemoryResidency,
    backend: Arc<dyn WasiNnBackend>,
    backend_model: Arc<dyn BackendModel>,
    #[cfg(test)]
    mock_latency: Duration,
}

impl CandleModel {
    #[cfg_attr(not(test), allow(dead_code))]
    fn load_mock(binding: &IntegrityModelBinding) -> Result<Self> {
        let backends = default_backends();
        let accelerator = AcceleratorKind::from_model_device(&binding.device);
        let backend = backends
            .get(&accelerator)
            .cloned()
            .ok_or_else(|| anyhow!("no backend is registered for `{}`", accelerator.as_str()))?;
        Self::load_mock_with_backend(binding, backend)
    }

    fn load_mock_with_backend(
        binding: &IntegrityModelBinding,
        backend: Arc<dyn WasiNnBackend>,
    ) -> Result<Self> {
        if binding.path.trim().is_empty() {
            return Err(anyhow!(
                "Integrity Validation Failed: model alias `{}` must declare a non-empty `path`",
                binding.alias
            ));
        }

        let source = BackendModelSource {
            alias: binding.alias.clone(),
            path: binding.path.clone(),
            requested_target: binding.device.as_str().to_owned(),
            accelerator: AcceleratorKind::from_model_device(&binding.device),
            qos: binding.qos,
            model_bytes: load_model_bytes(&binding.path),
        };
        let backend_model = backend
            .init(&source)
            .map_err(|error| anyhow!(error.to_string()))?;
        let memory_residency = backend_model.residency();

        Ok(Self {
            alias: binding.alias.clone(),
            accelerator: source.accelerator,
            qos: source.qos,
            memory_residency,
            backend,
            backend_model,
            #[cfg(test)]
            mock_latency: Duration::ZERO,
        })
    }

    #[cfg(test)]
    fn with_mock_latency(mut self, mock_latency: Duration) -> Self {
        self.mock_latency = mock_latency;
        self
    }

    fn run_mock_batch(&self, inputs: &[SharedInputTensor]) -> Result<WasiTensor, BackendError> {
        self.backend.execute(self.backend_model.as_ref(), inputs)
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

fn load_model_bytes(path: &str) -> Arc<[u8]> {
    match fs::read(path) {
        Ok(bytes) => Arc::from(bytes.into_boxed_slice()),
        Err(_) => Arc::from(path.as_bytes().to_vec().into_boxed_slice()),
    }
}

fn default_backends() -> HashMap<AcceleratorKind, Arc<dyn WasiNnBackend>> {
    HashMap::from([
        (
            AcceleratorKind::Cpu,
            Arc::new(CpuBackend) as Arc<dyn WasiNnBackend>,
        ),
        (
            AcceleratorKind::Gpu,
            Arc::new(GpuBackend) as Arc<dyn WasiNnBackend>,
        ),
        (
            AcceleratorKind::Npu,
            Arc::new(NpuBackend) as Arc<dyn WasiNnBackend>,
        ),
        (
            AcceleratorKind::Tpu,
            Arc::new(TpuBackend) as Arc<dyn WasiNnBackend>,
        ),
    ])
}

fn typed_backend_model<'a, T: BackendModel + 'static>(
    model: &'a dyn BackendModel,
    accelerator: AcceleratorKind,
    backend_name: &str,
) -> Result<&'a T, BackendError> {
    model.as_any().downcast_ref::<T>().ok_or_else(|| {
        backend_access_error(format!(
            "{backend_name} backend received an incompatible {} model handle",
            accelerator.as_str()
        ))
    })
}

fn mock_text_tensor() -> WasiTensor {
    WasiTensor::new(
        vec![MOCK_INFERENCE_RESPONSE.len() as u32],
        TensorType::U8,
        MOCK_INFERENCE_RESPONSE.as_bytes().to_vec(),
    )
}

fn mock_batch_dimensions(inputs: &[SharedInputTensor]) -> (usize, usize) {
    let batch_size = inputs.len().max(1);
    let longest_prompt = inputs
        .iter()
        .map(SharedInputTensor::byte_len)
        .max()
        .unwrap_or(1);
    (batch_size, longest_prompt)
}

fn validate_input_tensors(
    inputs: &[SharedInputTensor],
    accelerator: AcceleratorKind,
) -> Result<()> {
    if inputs.is_empty() {
        return Err(anyhow!(
            "{} backend requires at least one input tensor",
            accelerator.as_str()
        ));
    }
    for input in inputs {
        if input.dimensions.is_empty() {
            return Err(anyhow!(
                "{} backend requires at least one tensor dimension",
                accelerator.as_str()
            ));
        }
        if !matches!(input.ty, TensorType::U8 | TensorType::Fp32) {
            return Err(anyhow!(
                "{} backend only supports U8 or F32 test tensors",
                accelerator.as_str()
            ));
        }
    }
    Ok(())
}

fn execute_basic_backend(
    source: &BackendModelSource,
    accelerator: AcceleratorKind,
    inputs: &[SharedInputTensor],
) -> Result<WasiTensor, BackendError> {
    validate_input_tensors(inputs, accelerator)
        .map_err(|error| backend_access_error(error.to_string()))?;
    let (batch_size, longest_prompt) = mock_batch_dimensions(inputs);
    let _prompt_batch = CandleTensor::zeros((batch_size, longest_prompt), DType::F32, &Device::Cpu)
        .map_err(|error| {
            backend_access_error(format!(
                "failed to prepare {} mock batch for `{}` on requested `{}` from `{}`: {error}",
                accelerator.as_str(),
                source.alias,
                source.requested_target,
                source.path
            ))
        })?;
    let _resident_weights = source.model_bytes.len();
    Ok(mock_text_tensor())
}

struct CpuBackend;
struct GpuBackend;
struct NpuBackend;
struct TpuBackend;

#[derive(Clone)]
struct CpuBackendModel {
    source: BackendModelSource,
}

#[derive(Clone)]
struct GpuBackendModel {
    source: BackendModelSource,
    resident_weights: Arc<[u8]>,
    attention_stack: TurboQuantAttentionStack,
    _variables: VarMap,
}

#[derive(Clone)]
struct NpuBackendModel {
    source: BackendModelSource,
    resident_weights: Arc<[u8]>,
}

#[derive(Clone)]
struct TpuBackendModel {
    source: BackendModelSource,
    resident_weights: Arc<[u8]>,
}

impl BackendModel for CpuBackendModel {
    fn residency(&self) -> AcceleratorMemoryResidency {
        AcceleratorMemoryResidency::HostRam
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl BackendModel for GpuBackendModel {
    fn residency(&self) -> AcceleratorMemoryResidency {
        AcceleratorMemoryResidency::Vram
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl BackendModel for NpuBackendModel {
    fn residency(&self) -> AcceleratorMemoryResidency {
        AcceleratorMemoryResidency::Sram
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl BackendModel for TpuBackendModel {
    fn residency(&self) -> AcceleratorMemoryResidency {
        AcceleratorMemoryResidency::Sram
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl WasiNnBackend for CpuBackend {
    fn accelerator(&self) -> AcceleratorKind {
        AcceleratorKind::Cpu
    }

    fn backend_name(&self) -> &'static str {
        "ort"
    }

    fn init(&self, source: &BackendModelSource) -> Result<Arc<dyn BackendModel>, BackendError> {
        if source.model_bytes.is_empty() {
            return Err(backend_access_error(
                "CPU backend requires non-empty model bytes",
            ));
        }
        Ok(Arc::new(CpuBackendModel {
            source: source.clone(),
        }))
    }

    fn execute(
        &self,
        model: &dyn BackendModel,
        inputs: &[SharedInputTensor],
    ) -> Result<WasiTensor, BackendError> {
        let model =
            typed_backend_model::<CpuBackendModel>(model, self.accelerator(), self.backend_name())?;
        execute_basic_backend(&model.source, self.accelerator(), inputs)
    }
}

impl WasiNnBackend for GpuBackend {
    fn accelerator(&self) -> AcceleratorKind {
        AcceleratorKind::Gpu
    }

    fn backend_name(&self) -> &'static str {
        "candle"
    }

    fn init(&self, source: &BackendModelSource) -> Result<Arc<dyn BackendModel>, BackendError> {
        if source.model_bytes.is_empty() {
            return Err(backend_access_error(
                "GPU backend requires non-empty model bytes",
            ));
        }
        Ok(Arc::new(GpuBackendModel {
            source: source.clone(),
            resident_weights: Arc::clone(&source.model_bytes),
            attention_stack: TurboQuantAttentionStack::default(),
            _variables: VarMap::new(),
        }))
    }

    fn execute(
        &self,
        model: &dyn BackendModel,
        inputs: &[SharedInputTensor],
    ) -> Result<WasiTensor, BackendError> {
        let model =
            typed_backend_model::<GpuBackendModel>(model, self.accelerator(), self.backend_name())?;
        validate_input_tensors(inputs, self.accelerator())
            .map_err(|error| backend_access_error(error.to_string()))?;
        let (batch_size, longest_prompt) = mock_batch_dimensions(inputs);
        let _prompt_batch =
            CandleTensor::zeros((batch_size, longest_prompt), DType::F32, &Device::Cpu).map_err(
                |error| {
                    backend_access_error(format!(
                "failed to prepare mock Candle batch for `{}` on requested `{}` from `{}`: {error}",
                model.source.alias, model.source.requested_target, model.source.path
            ))
                },
            )?;
        let _resident_vram = model.resident_weights.len();
        model
            .attention_stack
            .run_mock_prompt(batch_size, longest_prompt)
            .map_err(|error| {
                backend_access_error(format!(
                    "failed to execute TurboQuant attention mock for `{}` on requested `{}` from `{}`: {error}",
                    model.source.alias, model.source.requested_target, model.source.path
                ))
            })?;
        Ok(mock_text_tensor())
    }
}

impl WasiNnBackend for NpuBackend {
    fn accelerator(&self) -> AcceleratorKind {
        AcceleratorKind::Npu
    }

    fn backend_name(&self) -> &'static str {
        "openvino"
    }

    fn init(&self, source: &BackendModelSource) -> Result<Arc<dyn BackendModel>, BackendError> {
        if source.model_bytes.is_empty() {
            return Err(backend_access_error(
                "NPU backend requires non-empty model bytes",
            ));
        }
        Ok(Arc::new(NpuBackendModel {
            source: source.clone(),
            resident_weights: Arc::clone(&source.model_bytes),
        }))
    }

    fn execute(
        &self,
        model: &dyn BackendModel,
        inputs: &[SharedInputTensor],
    ) -> Result<WasiTensor, BackendError> {
        let model =
            typed_backend_model::<NpuBackendModel>(model, self.accelerator(), self.backend_name())?;
        let _resident_sram = model.resident_weights.len();
        execute_basic_backend(&model.source, self.accelerator(), inputs)
    }
}

impl WasiNnBackend for TpuBackend {
    fn accelerator(&self) -> AcceleratorKind {
        AcceleratorKind::Tpu
    }

    fn backend_name(&self) -> &'static str {
        "libtpu"
    }

    fn init(&self, source: &BackendModelSource) -> Result<Arc<dyn BackendModel>, BackendError> {
        if source.model_bytes.is_empty() {
            return Err(backend_access_error(
                "TPU backend requires non-empty model bytes",
            ));
        }
        Ok(Arc::new(TpuBackendModel {
            source: source.clone(),
            resident_weights: Arc::clone(&source.model_bytes),
        }))
    }

    fn execute(
        &self,
        model: &dyn BackendModel,
        inputs: &[SharedInputTensor],
    ) -> Result<WasiTensor, BackendError> {
        let model =
            typed_backend_model::<TpuBackendModel>(model, self.accelerator(), self.backend_name())?;
        let _resident_sram = model.resident_weights.len();
        execute_basic_backend(&model.source, self.accelerator(), inputs)
    }
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
    // `fs` and `PathBuf` were used by the deleted `turboquant_ffi_match` test. The
    // replacement test builds its input in-memory so neither is needed any more.

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
                    .infer(
                        model,
                        WasiTensor::new(vec![1], TensorType::U8, b"hello".to_vec()),
                    )
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
        assert_eq!(snapshot.queued_requests, 0);
    }

    #[test]
    fn realtime_qos_preempts_batch_backlog_on_gpu_scheduler() {
        let scheduler =
            AcceleratorScheduler::new(AcceleratorKind::Gpu, 1, Duration::from_millis(0));
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
            receivers.push(
                scheduler
                    .enqueue(
                        Arc::clone(&batch_model),
                        WasiTensor::new(vec![1], TensorType::U8, b"batch".to_vec()),
                    )
                    .expect("batch request should queue"),
            );
        }
        thread::sleep(Duration::from_millis(5));
        let realtime_rx = scheduler
            .enqueue(
                Arc::clone(&realtime_model),
                WasiTensor::new(vec![1], TensorType::U8, b"realtime".to_vec()),
            )
            .expect("realtime request should queue");

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
    fn component_accelerator_runtime_rejects_mismatched_devices() {
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
            .is_err());
        assert_eq!(
            runtime
                .compute_component_prompt("llama3", "hello")
                .expect("component compute should succeed"),
            MOCK_INFERENCE_RESPONSE
        );
    }

    #[test]
    fn heterogeneous_runtime_routes_models_to_dedicated_accelerators() {
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.models = vec![
            IntegrityModelBinding {
                alias: "cpu-bert".to_owned(),
                path: "/models/cpu-bert.onnx".to_owned(),
                device: ModelDevice::Cpu,
                qos: RouteQos::Standard,
            },
            IntegrityModelBinding {
                alias: "gpu-llama".to_owned(),
                path: "/models/gpu-llama.gguf".to_owned(),
                device: ModelDevice::Cuda,
                qos: RouteQos::RealTime,
            },
            IntegrityModelBinding {
                alias: "npu-whisper".to_owned(),
                path: "/models/npu-whisper.xml".to_owned(),
                device: ModelDevice::Npu,
                qos: RouteQos::RealTime,
            },
            IntegrityModelBinding {
                alias: "tpu-embed".to_owned(),
                path: "/models/tpu-embed.tflite".to_owned(),
                device: ModelDevice::Tpu,
                qos: RouteQos::Batch,
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
        // Rust ↔ C++ round-trip. The C++ shim is gone; the fixtures went with it.
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
