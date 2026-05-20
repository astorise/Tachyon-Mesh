/// WASI-NN backend backed by `candle-onnx` Ã¢â‚¬â€ pure Rust, musl-compatible.
///
/// WASM guests load ONNX models via the standard WASI-NN graph API:
///   graph_load(bytes, onnx, cpu) Ã¢â€ â€™ graph
///   init_execution_context(graph) Ã¢â€ â€™ ctx
///   set_input(ctx, 0, tensor)
///   compute(ctx)
///   get_output(ctx, 0) Ã¢â€ â€™ tensor
use std::collections::HashMap;

use candle_core::{DType, Device, Tensor as CandleTensor};
use candle_onnx::onnx::ModelProto;
use prost::Message as _;
use wasmtime_wasi_nn::backend::{
    BackendError, BackendExecutionContext, BackendGraph, BackendInner, Id, NamedTensor,
};
use wasmtime_wasi_nn::wit::{ExecutionTarget, GraphEncoding, Tensor as WasiTensor, TensorType};
use wasmtime_wasi_nn::{Backend, ExecutionContext, Graph};

pub(super) fn candle_onnx_backend() -> Backend {
    Backend::from(CandleOnnxBackend)
}

// Ã¢â€â‚¬Ã¢â€â‚¬ BackendInner Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

struct CandleOnnxBackend;

impl BackendInner for CandleOnnxBackend {
    fn encoding(&self) -> GraphEncoding {
        GraphEncoding::Onnx
    }

    fn load(
        &mut self,
        builders: &[&[u8]],
        _target: ExecutionTarget,
    ) -> Result<Graph, BackendError> {
        let bytes = builders
            .first()
            .ok_or(BackendError::InvalidNumberOfBuilders(1, 0))?;
        let model = ModelProto::decode(*bytes).map_err(|e| {
            BackendError::BackendAccess(wasmtime::Error::msg(format!(
                "failed to parse ONNX model: {e}"
            )))
        })?;
        Ok(Graph::from(
            Box::new(CandleOnnxGraph { model }) as Box<dyn BackendGraph>
        ))
    }

    fn as_dir_loadable(&mut self) -> Option<&mut dyn wasmtime_wasi_nn::backend::BackendFromDir> {
        None
    }
}

// Ã¢â€â‚¬Ã¢â€â‚¬ BackendGraph Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

struct CandleOnnxGraph {
    model: ModelProto,
}

impl BackendGraph for CandleOnnxGraph {
    fn init_execution_context(&self) -> Result<ExecutionContext, BackendError> {
        let input_names = graph_input_names(&self.model);
        let output_names = graph_output_names(&self.model);
        Ok(ExecutionContext::from(Box::new(CandleOnnxContext {
            model: self.model.clone(),
            inputs: HashMap::new(),
            outputs: HashMap::new(),
            input_names,
            output_names,
        })
            as Box<dyn BackendExecutionContext>))
    }
}

// Ã¢â€â‚¬Ã¢â€â‚¬ BackendExecutionContext Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

struct CandleOnnxContext {
    model: ModelProto,
    inputs: HashMap<String, CandleTensor>,
    outputs: HashMap<String, CandleTensor>,
    input_names: Vec<String>,
    output_names: Vec<String>,
}

impl BackendExecutionContext for CandleOnnxContext {
    fn set_input(&mut self, id: Id, tensor: &WasiTensor) -> Result<(), BackendError> {
        let name = resolve_name(&id, &self.input_names)?;
        self.inputs.insert(name, wasi_to_candle(tensor)?);
        Ok(())
    }

    fn get_output(&mut self, id: Id) -> Result<WasiTensor, BackendError> {
        let name = resolve_name(&id, &self.output_names)?;
        let tensor = self.outputs.get(&name).ok_or_else(|| {
            BackendError::BackendAccess(wasmtime::Error::msg(format!(
                "output `{name}` not ready Ã¢â‚¬â€ call compute() first"
            )))
        })?;
        candle_to_wasi(tensor)
    }

    fn compute(
        &mut self,
        named: Option<Vec<NamedTensor>>,
    ) -> Result<Option<Vec<NamedTensor>>, BackendError> {
        if let Some(named_inputs) = named {
            for nt in named_inputs {
                self.inputs.insert(nt.name, wasi_to_candle(&nt.tensor)?);
            }
        }
        self.outputs = candle_onnx::simple_eval(&self.model, self.inputs.clone())
            .map_err(|e| BackendError::BackendAccess(wasmtime::Error::msg(e.to_string())))?;
        Ok(None)
    }
}

// Ã¢â€â‚¬Ã¢â€â‚¬ Helpers Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

fn graph_input_names(model: &ModelProto) -> Vec<String> {
    model
        .graph
        .as_ref()
        .map(|g| g.input.iter().map(|i| i.name.clone()).collect())
        .unwrap_or_default()
}

fn graph_output_names(model: &ModelProto) -> Vec<String> {
    model
        .graph
        .as_ref()
        .map(|g| g.output.iter().map(|o| o.name.clone()).collect())
        .unwrap_or_default()
}

fn resolve_name(id: &Id, names: &[String]) -> Result<String, BackendError> {
    match id {
        Id::Name(n) => Ok(n.clone()),
        Id::Index(i) => names.get(*i as usize).cloned().ok_or_else(|| {
            BackendError::BackendAccess(wasmtime::Error::msg(format!(
                "tensor index {i} out of range ({} tensors available)",
                names.len()
            )))
        }),
    }
}

fn wasi_to_candle(tensor: &WasiTensor) -> Result<CandleTensor, BackendError> {
    let shape: Vec<usize> = tensor.dimensions.iter().map(|&d| d as usize).collect();
    let dtype = match tensor.ty {
        TensorType::Fp32 => DType::F32,
        TensorType::Fp16 => DType::F16,
        TensorType::U8 => DType::U8,
        TensorType::I32 => DType::I64,
        other => return Err(BackendError::UnsupportedTensorType(format!("{other:?}"))),
    };
    CandleTensor::from_raw_buffer(&tensor.data, dtype, &shape, &Device::Cpu)
        .map_err(|e| BackendError::BackendAccess(wasmtime::Error::msg(e.to_string())))
}

fn candle_to_wasi(tensor: &CandleTensor) -> Result<WasiTensor, BackendError> {
    let dimensions = tensor.dims().iter().map(|&d| d as u32).collect();
    let (ty, data) = match tensor.dtype() {
        DType::F32 => {
            let v = tensor
                .flatten_all()
                .and_then(|t| t.to_vec1::<f32>())
                .map_err(|e| BackendError::BackendAccess(wasmtime::Error::msg(e.to_string())))?;
            (
                TensorType::Fp32,
                v.iter().flat_map(|f| f.to_le_bytes()).collect(),
            )
        }
        DType::U8 => {
            let v = tensor
                .flatten_all()
                .and_then(|t| t.to_vec1::<u8>())
                .map_err(|e| BackendError::BackendAccess(wasmtime::Error::msg(e.to_string())))?;
            (TensorType::U8, v)
        }
        DType::I64 => {
            let v = tensor
                .flatten_all()
                .and_then(|t| t.to_vec1::<i64>())
                .map_err(|e| BackendError::BackendAccess(wasmtime::Error::msg(e.to_string())))?;
            let bytes = v.iter().flat_map(|i| i.to_le_bytes()).collect();
            (TensorType::I32, bytes)
        }
        other => return Err(BackendError::UnsupportedTensorType(format!("{other:?}"))),
    };
    Ok(WasiTensor {
        dimensions,
        ty,
        data,
    })
}
