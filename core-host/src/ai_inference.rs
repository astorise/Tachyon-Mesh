use anyhow::{anyhow, Result};
use candle_core::{DType, Device, Tensor as CandleTensor};
use candle_nn::VarMap;
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, RecvTimeoutError},
        Arc,
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

use crate::{IntegrityConfig, IntegrityModelBinding};

const MOCK_INFERENCE_RESPONSE: &str = "MOCK_LLM_RESPONSE";
const DEFAULT_BATCH_SIZE: usize = 32;
const DEFAULT_BATCH_WINDOW: Duration = Duration::from_millis(25);

#[derive(Clone)]
pub(crate) struct AiInferenceRuntime {
    scheduler: CandleBatchScheduler,
    models: HashMap<String, Arc<CandleModel>>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SchedulerSnapshot {
    pub(crate) batches_processed: usize,
    pub(crate) requests_processed: usize,
    pub(crate) max_batch_size: usize,
}

impl AiInferenceRuntime {
    pub(crate) fn from_config(config: &IntegrityConfig) -> Result<Self> {
        let scheduler = CandleBatchScheduler::new(DEFAULT_BATCH_SIZE, DEFAULT_BATCH_WINDOW);
        let mut models = HashMap::new();

        for route in &config.routes {
            for binding in &route.models {
                if models.contains_key(&binding.alias) {
                    return Err(anyhow!(
                        "Integrity Validation Failed: model alias `{}` must be globally unique",
                        binding.alias
                    ));
                }
                models.insert(
                    binding.alias.clone(),
                    Arc::new(CandleModel::load_mock(binding)?),
                );
            }
        }

        Ok(Self { scheduler, models })
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
                            alias: alias.clone(),
                            model: Arc::clone(model),
                            scheduler: self.scheduler.clone(),
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
    pub(crate) fn scheduler_snapshot(&self) -> SchedulerSnapshot {
        self.scheduler.snapshot()
    }
}

#[derive(Clone)]
struct CandleBatchScheduler {
    sender: mpsc::Sender<InferenceJob>,
    #[cfg_attr(not(test), allow(dead_code))]
    metrics: Arc<SchedulerMetrics>,
}

impl CandleBatchScheduler {
    fn new(batch_size: usize, batch_window: Duration) -> Self {
        let (sender, receiver) = mpsc::channel();
        let metrics = Arc::new(SchedulerMetrics::default());
        let worker_metrics = Arc::clone(&metrics);

        thread::Builder::new()
            .name("tachyon-candle-batcher".to_owned())
            .spawn(move || run_scheduler(receiver, worker_metrics, batch_size, batch_window))
            .expect("AI inference batch scheduler thread should start");

        Self { sender, metrics }
    }

    fn infer(
        &self,
        alias: &str,
        model: Arc<CandleModel>,
        input: WasiTensor,
    ) -> Result<WasiTensor, BackendError> {
        let (response_tx, response_rx) = mpsc::channel();
        self.sender
            .send(InferenceJob {
                alias: alias.to_owned(),
                model,
                input,
                response_tx,
            })
            .map_err(|_| backend_access_error("AI inference scheduler is unavailable"))?;
        response_rx.recv().map_err(|_| {
            backend_access_error("AI inference response channel closed unexpectedly")
        })?
    }

    #[cfg(test)]
    fn snapshot(&self) -> SchedulerSnapshot {
        SchedulerSnapshot {
            batches_processed: self.metrics.batches_processed.load(Ordering::Relaxed),
            requests_processed: self.metrics.requests_processed.load(Ordering::Relaxed),
            max_batch_size: self.metrics.max_batch_size.load(Ordering::Relaxed),
        }
    }
}

#[derive(Default)]
struct SchedulerMetrics {
    batches_processed: AtomicUsize,
    requests_processed: AtomicUsize,
    max_batch_size: AtomicUsize,
}

impl SchedulerMetrics {
    fn record_batch(&self, batch_size: usize) {
        self.batches_processed.fetch_add(1, Ordering::Relaxed);
        self.requests_processed
            .fetch_add(batch_size, Ordering::Relaxed);
        self.max_batch_size.fetch_max(batch_size, Ordering::Relaxed);
    }
}

struct InferenceJob {
    alias: String,
    model: Arc<CandleModel>,
    input: WasiTensor,
    response_tx: mpsc::Sender<Result<WasiTensor, BackendError>>,
}

fn run_scheduler(
    receiver: mpsc::Receiver<InferenceJob>,
    metrics: Arc<SchedulerMetrics>,
    batch_size: usize,
    batch_window: Duration,
) {
    let mut backlog = VecDeque::new();
    let mut disconnected = false;

    loop {
        let first = match backlog.pop_front() {
            Some(job) => job,
            None => match receiver.recv() {
                Ok(job) => job,
                Err(_) => break,
            },
        };

        let batch_alias = first.alias.clone();
        let mut batch = vec![first];
        let deadline = Instant::now() + batch_window;

        while batch.len() < batch_size {
            if let Some(index) = backlog.iter().position(|job| job.alias == batch_alias) {
                let job = backlog
                    .remove(index)
                    .expect("backlog index selected from iterator should remain valid");
                batch.push(job);
                continue;
            }

            if disconnected {
                break;
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }

            match receiver.recv_timeout(remaining) {
                Ok(job) if job.alias == batch_alias => batch.push(job),
                Ok(job) => backlog.push_back(job),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }

        let results = process_batch(&batch);
        metrics.record_batch(batch.len());
        for (job, result) in batch.into_iter().zip(results.into_iter()) {
            let _ = job.response_tx.send(result);
        }

        if disconnected && backlog.is_empty() {
            break;
        }
    }
}

fn process_batch(batch: &[InferenceJob]) -> Vec<Result<WasiTensor, BackendError>> {
    let longest_prompt = batch
        .iter()
        .map(|job| job.input.data.len().max(1))
        .max()
        .unwrap_or(1);
    let model = Arc::clone(&batch[0].model);
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
    alias: String,
    model: Arc<CandleModel>,
    scheduler: CandleBatchScheduler,
}

impl BackendGraph for CandleModelGraph {
    fn init_execution_context(&self) -> Result<wasmtime_wasi_nn::ExecutionContext, BackendError> {
        Ok((Box::new(CandleExecutionContext {
            alias: self.alias.clone(),
            model: Arc::clone(&self.model),
            scheduler: self.scheduler.clone(),
            input: None,
            output: None,
        }) as Box<dyn BackendExecutionContext>)
            .into())
    }
}

struct CandleExecutionContext {
    alias: String,
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

        let output = self
            .scheduler
            .infer(&self.alias, Arc::clone(&self.model), input)?;
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
    _variables: VarMap,
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
            _variables: VarMap::new(),
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IntegrityConfig, IntegrityRoute, ModelDevice};

    #[test]
    fn runtime_preloads_model_aliases_from_config() {
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.models = vec![
            IntegrityModelBinding {
                alias: "llama3".to_owned(),
                path: "/models/llama3.gguf".to_owned(),
                device: ModelDevice::Cuda,
            },
            IntegrityModelBinding {
                alias: "tiny".to_owned(),
                path: "/models/tiny.gguf".to_owned(),
                device: ModelDevice::Cpu,
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
        let mut handles = Vec::new();
        let barrier = Arc::new(std::sync::Barrier::new(8));

        for _ in 0..8 {
            let barrier = Arc::clone(&barrier);
            let scheduler = runtime.scheduler.clone();
            let model = runtime
                .models
                .get("llama3")
                .expect("model should exist")
                .clone();
            handles.push(thread::spawn(move || {
                barrier.wait();
                scheduler
                    .infer(
                        "llama3",
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

        let snapshot = runtime.scheduler_snapshot();
        assert_eq!(snapshot.requests_processed, 8);
        assert_eq!(snapshot.batches_processed, 1);
        assert_eq!(snapshot.max_batch_size, 8);
    }

    fn config_with_model(alias: &str) -> IntegrityConfig {
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.models = vec![IntegrityModelBinding {
            alias: alias.to_owned(),
            path: format!("/models/{alias}.gguf"),
            device: ModelDevice::Cpu,
        }];
        IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        }
    }
}
