use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{bail, Context, Result};
use serde_json::Value;

#[cfg(feature = "nvfp4-cuda")]
#[path = "modelopt_nvfp4_cuda.rs"]
pub(crate) mod cuda;

const CONFIG_JSON: &str = "config.json";
const SAFETENSORS_INDEX_JSON: &str = "model.safetensors.index.json";
const HF_QUANT_CONFIG_JSON: &str = "hf_quant_config.json";
const NVFP4_ALGO: &str = "W4A16_NVFP4";
pub(crate) const NVFP4_BLOCK_SIZE: usize = 16;

const E2M1_LUT: [f32; 16] = [
    0.0, 0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0, -0.0, -0.5, -1.0, -1.5, -2.0, -3.0, -4.0, -6.0,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Nvfp4OutputDType {
    F32,
    BF16,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Nvfp4DenseValues {
    F32(Vec<f32>),
    BF16(Vec<u16>),
}

impl Nvfp4DenseValues {
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::F32(values) => values.len(),
            Self::BF16(values) => values.len(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Nvfp4FallbackScope {
    Eager,
    LayerWindow(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Nvfp4FallbackMemoryEstimate {
    pub(crate) scope: Nvfp4FallbackScope,
    pub(crate) active_layers: usize,
    pub(crate) packed_model_bytes: u64,
    pub(crate) dense_active_bytes: u64,
    pub(crate) host_ram_bytes: u64,
    pub(crate) accelerator_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Nvfp4FallbackMemoryLimits {
    pub(crate) max_host_ram_bytes: Option<u64>,
    pub(crate) max_accelerator_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Nvfp4KernelKind {
    Dequantize,
    Matmul,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Nvfp4AcceleratorCapabilities {
    pub(crate) native_fp4: bool,
    pub(crate) runtime_available: bool,
    pub(crate) compiled_kernels: BTreeSet<Nvfp4KernelKind>,
}

impl Nvfp4AcceleratorCapabilities {
    pub(crate) fn fallback_only() -> Self {
        Self::default()
    }

    pub(crate) fn native_fp4(kernels: impl IntoIterator<Item = Nvfp4KernelKind>) -> Self {
        Self {
            native_fp4: true,
            runtime_available: true,
            compiled_kernels: kernels.into_iter().collect(),
        }
    }

    pub(crate) fn supports_native_nvfp4_matmul(&self) -> bool {
        self.native_fp4
            && self.runtime_available
            && self.compiled_kernels.contains(&Nvfp4KernelKind::Dequantize)
            && self.compiled_kernels.contains(&Nvfp4KernelKind::Matmul)
    }

    pub(crate) fn cuda_cutlass() -> Self {
        #[cfg(feature = "nvfp4-cuda")]
        {
            cuda::Nvfp4CudaBackend::capabilities()
        }
        #[cfg(not(feature = "nvfp4-cuda"))]
        {
            Self::fallback_only()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Nvfp4KernelSelectionMode {
    NativePreferred,
    NativeRequired,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Nvfp4ExecutionPlan {
    Native {
        required_kernels: Vec<Nvfp4KernelKind>,
    },
    Fallback(Nvfp4FallbackMemoryEstimate),
}

#[derive(Clone, Debug)]
pub(crate) struct ModelOptNvfp4Directory {
    alias: String,
    root: PathBuf,
    config_json: Option<Value>,
    hf_quant_json: Option<Value>,
    shard_index: SafetensorsShardIndex,
    header_cache: Arc<Mutex<BTreeMap<PathBuf, SafetensorsHeader>>>,
    quantization: ModelOptNvfp4QuantizationLayout,
}

impl ModelOptNvfp4Directory {
    pub(crate) fn try_load(alias: &str, path: impl AsRef<Path>) -> Result<Option<Self>> {
        let root = path.as_ref();
        if !root.is_dir() {
            return Ok(None);
        }

        let config_path = root.join(CONFIG_JSON);
        let index_path = root.join(SAFETENSORS_INDEX_JSON);
        let hf_quant_path = root.join(HF_QUANT_CONFIG_JSON);
        if !config_path.exists() && !index_path.exists() && !hf_quant_path.exists() {
            return Ok(None);
        }

        let config_json = if config_path.exists() {
            Some(read_json(&config_path, alias)?)
        } else {
            None
        };
        let hf_quant_json = if hf_quant_path.exists() {
            Some(read_json(&hf_quant_path, alias)?)
        } else {
            None
        };
        let metadata_mentions_nvfp4 = json_option_contains_text(config_json.as_ref(), NVFP4_ALGO)
            || json_option_contains_text(config_json.as_ref(), "NVFP4")
            || json_option_contains_text(hf_quant_json.as_ref(), NVFP4_ALGO)
            || json_option_contains_text(hf_quant_json.as_ref(), "NVFP4");

        if !index_path.exists() {
            if metadata_mentions_nvfp4 {
                return Err(ModelOptNvfp4LoadError::MissingFile {
                    alias: alias.to_owned(),
                    root: root.to_path_buf(),
                    file: SAFETENSORS_INDEX_JSON.to_owned(),
                }
                .into());
            }
            return Ok(None);
        }

        let shard_index = SafetensorsShardIndex::load(alias, root, &index_path)?;
        let looks_like_nvfp4 =
            metadata_mentions_nvfp4 || shard_index.has_tensor_suffix(".weight_scale_2");
        if !looks_like_nvfp4 {
            return Ok(None);
        }

        let quantization = ModelOptNvfp4QuantizationLayout::detect(
            alias,
            config_json.as_ref(),
            hf_quant_json.as_ref(),
            &shard_index,
        )?;

        Ok(Some(Self {
            alias: alias.to_owned(),
            root: root.to_path_buf(),
            config_json,
            hf_quant_json,
            shard_index,
            header_cache: Arc::new(Mutex::new(BTreeMap::new())),
            quantization,
        }))
    }

    pub(crate) fn alias(&self) -> &str {
        &self.alias
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn quantization(&self) -> &ModelOptNvfp4QuantizationLayout {
        &self.quantization
    }

    pub(crate) fn config_json(&self) -> Option<&Value> {
        self.config_json.as_ref()
    }

    pub(crate) fn hf_quant_json(&self) -> Option<&Value> {
        self.hf_quant_json.as_ref()
    }

    pub(crate) fn contains_tensor(&self, tensor_name: &str) -> bool {
        self.shard_index.contains_tensor(tensor_name)
    }

    /// Every tensor name the checkpoint declares, across all shards. Callers
    /// building a runtime walk this to classify and materialize parameters.
    pub(crate) fn tensor_names(&self) -> impl Iterator<Item = &str> {
        self.shard_index.tensor_names()
    }

    pub(crate) fn tensor_location(&self, tensor_name: &str) -> Option<TensorShardLocation> {
        self.shard_index.tensor_location(&self.root, tensor_name)
    }

    pub(crate) fn tensor_info(&self, tensor_name: &str) -> Result<SafetensorsTensorRef> {
        let location = self.tensor_location(tensor_name).ok_or_else(|| {
            ModelOptNvfp4LoadError::TensorNotFound {
                alias: self.alias.clone(),
                tensor: tensor_name.to_owned(),
            }
        })?;
        let info = {
            let mut cache = self
                .header_cache
                .lock()
                .map_err(|_| anyhow::anyhow!("safetensors header cache lock is poisoned"))?;
            if !cache.contains_key(&location.shard_path) {
                let header = SafetensorsHeader::read(&location.shard_path)?;
                cache.insert(location.shard_path.clone(), header);
            }
            cache
                .get(&location.shard_path)
                .and_then(|header| header.tensor(tensor_name))
        };
        let info = info.ok_or_else(|| ModelOptNvfp4LoadError::TensorNotFoundInShard {
            alias: self.alias.clone(),
            tensor: tensor_name.to_owned(),
            shard: location.shard_path.clone(),
        })?;
        Ok(SafetensorsTensorRef {
            name: tensor_name.to_owned(),
            shard_path: location.shard_path,
            info,
        })
    }

    pub(crate) fn layer_tensor_maps(&self) -> Vec<Nvfp4LayerTensorMap> {
        self.shard_index.layer_tensor_maps()
    }

    pub(crate) fn layer_tensor_refs(
        &self,
        layer_index: usize,
    ) -> Result<Vec<SafetensorsTensorRef>> {
        self.shard_index
            .tensor_names_for_layer(layer_index)
            .into_iter()
            .map(|name| self.tensor_info(&name))
            .collect()
    }

    pub(crate) fn modelopt_linear(&self, base_name: &str) -> Result<ModelOptLinearTensors> {
        let weight = self.tensor_info(&format!("{base_name}.weight"))?;
        let input_scale = self
            .tensor_info(&format!("{base_name}.input_scale"))
            .map(InputScaleTensorRef::from)
            .ok();
        let weight_scale = self
            .tensor_info(&format!("{base_name}.weight_scale"))
            .map(Fp8BlockScaleTensorRef::from)
            .ok();
        let weight_scale_2 = self
            .tensor_info(&format!("{base_name}.weight_scale_2"))
            .map(TensorScaleRef::from)
            .ok();

        if let Some(tensor_scale) = weight_scale_2 {
            let block_scales = weight_scale.ok_or_else(|| {
                ModelOptNvfp4LoadError::MissingQuantizationComponent {
                    alias: self.alias.clone(),
                    base: base_name.to_owned(),
                    component: "weight_scale".to_owned(),
                }
            })?;
            let linear = Nvfp4LinearTensors {
                base_name: base_name.to_owned(),
                packed_weight: PackedFp4TensorRef::from(weight),
                block_scales,
                tensor_scale,
                input_scale,
            };
            linear.validate_layout(&self.alias)?;
            return Ok(ModelOptLinearTensors::Nvfp4(linear));
        }

        if let Some(weight_scale) = weight_scale {
            return Ok(ModelOptLinearTensors::Fp8(Fp8LinearTensors {
                base_name: base_name.to_owned(),
                weight,
                weight_scale,
                input_scale,
            }));
        }

        Ok(ModelOptLinearTensors::Passthrough(
            PassthroughTensorRef::from(weight),
        ))
    }

    pub(crate) fn estimate_fallback_memory(
        &self,
        output_dtype: Nvfp4OutputDType,
        scope: Nvfp4FallbackScope,
    ) -> Nvfp4FallbackMemoryEstimate {
        let total_layers = self.quantization.layer_count.max(1);
        let active_layers = match scope {
            Nvfp4FallbackScope::Eager => total_layers,
            Nvfp4FallbackScope::LayerWindow(window) => window.clamp(1, total_layers),
        };
        let bytes_per_value = match output_dtype {
            Nvfp4OutputDType::F32 => 4u64,
            Nvfp4OutputDType::BF16 => 2u64,
        };
        let dense_total_bytes = self
            .shard_index
            .total_parameters
            .unwrap_or(0)
            .saturating_mul(bytes_per_value);
        let dense_per_layer = div_ceil_u64(dense_total_bytes, total_layers as u64);
        let dense_active_bytes = dense_per_layer.saturating_mul(active_layers as u64);
        let packed_model_bytes = self.shard_index.total_size.unwrap_or(0);
        Nvfp4FallbackMemoryEstimate {
            scope,
            active_layers,
            packed_model_bytes,
            dense_active_bytes,
            host_ram_bytes: packed_model_bytes.saturating_add(dense_active_bytes),
            accelerator_bytes: dense_active_bytes,
        }
    }

    pub(crate) fn ensure_fallback_memory_within_limits(
        &self,
        output_dtype: Nvfp4OutputDType,
        scope: Nvfp4FallbackScope,
        limits: Nvfp4FallbackMemoryLimits,
    ) -> Result<Nvfp4FallbackMemoryEstimate> {
        let estimate = self.estimate_fallback_memory(output_dtype, scope);
        if let Some(limit) = limits.max_host_ram_bytes {
            if estimate.host_ram_bytes > limit {
                return Err(ModelOptNvfp4LoadError::UnsupportedQuantization {
                    alias: self.alias.clone(),
                    detail: format!(
                        "fallback dequantization requires {} host RAM bytes, limit is {limit}",
                        estimate.host_ram_bytes
                    ),
                }
                .into());
            }
        }
        if let Some(limit) = limits.max_accelerator_bytes {
            if estimate.accelerator_bytes > limit {
                return Err(ModelOptNvfp4LoadError::UnsupportedQuantization {
                    alias: self.alias.clone(),
                    detail: format!(
                        "fallback dequantization requires {} accelerator bytes, limit is {limit}",
                        estimate.accelerator_bytes
                    ),
                }
                .into());
            }
        }
        Ok(estimate)
    }

    pub(crate) fn select_execution_plan(
        &self,
        capabilities: &Nvfp4AcceleratorCapabilities,
        output_dtype: Nvfp4OutputDType,
        fallback_scope: Nvfp4FallbackScope,
        fallback_limits: Nvfp4FallbackMemoryLimits,
        mode: Nvfp4KernelSelectionMode,
    ) -> Result<Nvfp4ExecutionPlan> {
        if capabilities.supports_native_nvfp4_matmul() {
            return Ok(Nvfp4ExecutionPlan::Native {
                required_kernels: vec![Nvfp4KernelKind::Dequantize, Nvfp4KernelKind::Matmul],
            });
        }
        if mode == Nvfp4KernelSelectionMode::NativeRequired {
            return Err(ModelOptNvfp4LoadError::UnsupportedQuantization {
                alias: self.alias.clone(),
                detail: "native NVFP4 execution requires FP4-capable hardware, runtime availability, and compiled dequant+matmul kernels".to_owned(),
            }
            .into());
        }

        self.ensure_fallback_memory_within_limits(output_dtype, fallback_scope, fallback_limits)
            .map(Nvfp4ExecutionPlan::Fallback)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModelOptNvfp4QuantizationLayout {
    pub(crate) producer: Option<String>,
    pub(crate) has_nvfp4_layers: bool,
    pub(crate) has_fp8_layers: bool,
    pub(crate) group_size: usize,
    pub(crate) layer_count: usize,
}

impl ModelOptNvfp4QuantizationLayout {
    fn detect(
        alias: &str,
        config_json: Option<&Value>,
        hf_quant_json: Option<&Value>,
        shard_index: &SafetensorsShardIndex,
    ) -> Result<Self> {
        let has_nvfp4_layers = json_option_contains_text(config_json, NVFP4_ALGO)
            || json_option_contains_text(config_json, "NVFP4")
            || json_option_contains_text(hf_quant_json, NVFP4_ALGO)
            || json_option_contains_text(hf_quant_json, "NVFP4")
            || shard_index.has_tensor_suffix(".weight_scale_2");
        if !has_nvfp4_layers {
            return Err(ModelOptNvfp4LoadError::UnsupportedQuantization {
                alias: alias.to_owned(),
                detail: "expected ModelOpt W4A16_NVFP4 metadata or .weight_scale_2 tensors"
                    .to_owned(),
            }
            .into());
        }

        let producer = hf_quant_json
            .and_then(|json| json.get("producer"))
            .and_then(|producer| producer.get("name"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let has_fp8_layers = json_option_contains_text(config_json, "FP8")
            || json_option_contains_text(hf_quant_json, "FP8");
        let group_size = config_json
            .and_then(find_group_size)
            .or_else(|| hf_quant_json.and_then(find_group_size))
            .unwrap_or(NVFP4_BLOCK_SIZE);
        if group_size != NVFP4_BLOCK_SIZE {
            return Err(ModelOptNvfp4LoadError::UnsupportedQuantization {
                alias: alias.to_owned(),
                detail: format!("expected NVFP4 group size {NVFP4_BLOCK_SIZE}, got {group_size}"),
            }
            .into());
        }
        let layer_count = config_json
            .and_then(|json| find_usize_key(json, "num_hidden_layers"))
            .or_else(|| shard_index.inferred_layer_count())
            .unwrap_or(1)
            .max(1);

        Ok(Self {
            producer,
            has_nvfp4_layers,
            has_fp8_layers,
            group_size,
            layer_count,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SafetensorsShardIndex {
    pub(crate) total_parameters: Option<u64>,
    pub(crate) total_size: Option<u64>,
    weight_map: BTreeMap<String, String>,
    shards: BTreeSet<String>,
}

impl SafetensorsShardIndex {
    fn load(alias: &str, root: &Path, index_path: &Path) -> Result<Self> {
        let value = read_json(index_path, alias)?;
        let metadata = value.get("metadata");
        let total_parameters = metadata
            .and_then(|metadata| metadata.get("total_parameters"))
            .and_then(Value::as_u64);
        let total_size = metadata
            .and_then(|metadata| metadata.get("total_size"))
            .and_then(Value::as_u64);
        let raw_weight_map = value
            .get("weight_map")
            .and_then(Value::as_object)
            .ok_or_else(|| ModelOptNvfp4LoadError::InvalidJson {
                alias: alias.to_owned(),
                path: index_path.to_path_buf(),
                detail: "missing object field `weight_map`".to_owned(),
            })?;

        let mut weight_map = BTreeMap::new();
        let mut shards = BTreeSet::new();
        for (tensor, shard) in raw_weight_map {
            let shard = shard
                .as_str()
                .ok_or_else(|| ModelOptNvfp4LoadError::InvalidJson {
                    alias: alias.to_owned(),
                    path: index_path.to_path_buf(),
                    detail: format!("weight_map entry `{tensor}` is not a string"),
                })?;
            let shard_path = root.join(shard);
            if !shard_path.exists() {
                return Err(ModelOptNvfp4LoadError::MissingShard {
                    alias: alias.to_owned(),
                    root: root.to_path_buf(),
                    shard: shard.to_owned(),
                }
                .into());
            }
            weight_map.insert(tensor.clone(), shard.to_owned());
            shards.insert(shard.to_owned());
        }

        Ok(Self {
            total_parameters,
            total_size,
            weight_map,
            shards,
        })
    }

    pub(crate) fn tensor_location(
        &self,
        root: &Path,
        tensor_name: &str,
    ) -> Option<TensorShardLocation> {
        self.weight_map
            .get(tensor_name)
            .map(|shard| TensorShardLocation {
                tensor_name: tensor_name.to_owned(),
                shard_name: shard.clone(),
                shard_path: root.join(shard),
            })
    }

    pub(crate) fn tensor_count(&self) -> usize {
        self.weight_map.len()
    }

    pub(crate) fn shard_count(&self) -> usize {
        self.shards.len()
    }

    pub(crate) fn has_tensor_suffix(&self, suffix: &str) -> bool {
        self.weight_map
            .keys()
            .any(|tensor| tensor.ends_with(suffix))
    }

    pub(crate) fn contains_tensor(&self, tensor_name: &str) -> bool {
        self.weight_map.contains_key(tensor_name)
    }

    pub(crate) fn tensor_names(&self) -> impl Iterator<Item = &str> {
        self.weight_map.keys().map(String::as_str)
    }

    fn inferred_layer_count(&self) -> Option<usize> {
        self.weight_map
            .keys()
            .filter_map(|tensor| layer_index_from_tensor_name(tensor))
            .max()
            .map(|index| index + 1)
    }

    fn layer_tensor_maps(&self) -> Vec<Nvfp4LayerTensorMap> {
        let mut by_layer = BTreeMap::<usize, Vec<String>>::new();
        for tensor in self.weight_map.keys() {
            if let Some(layer_index) = layer_index_from_tensor_name(tensor) {
                by_layer
                    .entry(layer_index)
                    .or_default()
                    .push(tensor.clone());
            }
        }
        by_layer
            .into_iter()
            .map(|(layer_index, tensor_names)| Nvfp4LayerTensorMap {
                layer_index,
                tensor_names,
            })
            .collect()
    }

    fn tensor_names_for_layer(&self, layer_index: usize) -> Vec<String> {
        self.weight_map
            .keys()
            .filter(|tensor| layer_index_from_tensor_name(tensor) == Some(layer_index))
            .cloned()
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TensorShardLocation {
    pub(crate) tensor_name: String,
    pub(crate) shard_name: String,
    pub(crate) shard_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Nvfp4LayerTensorMap {
    pub(crate) layer_index: usize,
    pub(crate) tensor_names: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SafetensorsTensorRef {
    pub(crate) name: String,
    pub(crate) shard_path: PathBuf,
    pub(crate) info: SafetensorsTensorInfo,
}

impl SafetensorsTensorRef {
    pub(crate) fn read_bytes(&self) -> Result<Vec<u8>> {
        let mut file = File::open(&self.shard_path).with_context(|| {
            format!(
                "failed to open safetensors shard `{}`",
                self.shard_path.display()
            )
        })?;
        let mut prefix = [0u8; 8];
        file.read_exact(&mut prefix)?;
        let header_len = u64::from_le_bytes(prefix);
        let data_start = 8u64
            .checked_add(header_len)
            .context("safetensors data offset overflow")?;
        let start = data_start
            .checked_add(u64::try_from(self.info.data_offsets.0)?)
            .context("safetensors tensor start offset overflow")?;
        let len = self
            .info
            .data_offsets
            .1
            .checked_sub(self.info.data_offsets.0)
            .context("invalid safetensors tensor offsets")?;
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = vec![0u8; len];
        file.read_exact(&mut bytes).with_context(|| {
            format!(
                "failed to read tensor `{}` from `{}`",
                self.name,
                self.shard_path.display()
            )
        })?;
        Ok(bytes)
    }

    pub(crate) fn read_f32(&self) -> Result<Vec<f32>> {
        let bytes = self.read_bytes()?;
        let expected = elem_count(&self.info.shape);
        let values: Vec<f32> = match self.info.dtype {
            SafetensorsDType::F8E4M3 => bytes.into_iter().map(e4m3_to_f32).collect(),
            SafetensorsDType::BF16 => bytes
                .chunks_exact(2)
                .map(|chunk| {
                    let bits = u16::from_le_bytes([chunk[0], chunk[1]]);
                    f32::from_bits(u32::from(bits) << 16)
                })
                .collect(),
            SafetensorsDType::F16 => bytes
                .chunks_exact(2)
                .map(|chunk| f16_bits_to_f32(u16::from_le_bytes([chunk[0], chunk[1]])))
                .collect(),
            SafetensorsDType::F32 => bytes
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect(),
            dtype => bail!(
                "tensor `{}` cannot be decoded as f32 from dtype {dtype:?}",
                self.name
            ),
        };
        if values.len() != expected {
            bail!(
                "tensor `{}` decoded {} values, expected {expected}",
                self.name,
                values.len()
            );
        }
        Ok(values)
    }

    pub(crate) fn read_f32_slice(
        &self,
        element_start: usize,
        element_len: usize,
    ) -> Result<Vec<f32>> {
        let element_size = match self.info.dtype {
            SafetensorsDType::F8E4M3 => 1usize,
            SafetensorsDType::BF16 | SafetensorsDType::F16 => 2,
            SafetensorsDType::F32 => 4,
            dtype => bail!(
                "tensor `{}` cannot be sliced as f32 from dtype {dtype:?}",
                self.name
            ),
        };
        let total = elem_count(&self.info.shape);
        let element_end = element_start
            .checked_add(element_len)
            .context("tensor slice range overflow")?;
        if element_end > total {
            bail!(
                "tensor `{}` slice [{element_start}..{element_end}] exceeds {total} elements",
                self.name
            );
        }
        let byte_start = element_start
            .checked_mul(element_size)
            .context("tensor slice byte offset overflow")?;
        let byte_len = element_len
            .checked_mul(element_size)
            .context("tensor slice byte length overflow")?;
        let mut file = File::open(&self.shard_path)?;
        let mut prefix = [0u8; 8];
        file.read_exact(&mut prefix)?;
        let data_start = 8u64
            .checked_add(u64::from_le_bytes(prefix))
            .context("safetensors data offset overflow")?;
        let tensor_start = data_start
            .checked_add(u64::try_from(self.info.data_offsets.0)?)
            .and_then(|offset| offset.checked_add(u64::try_from(byte_start).ok()?))
            .context("tensor slice start offset overflow")?;
        file.seek(SeekFrom::Start(tensor_start))?;
        let mut bytes = vec![0u8; byte_len];
        file.read_exact(&mut bytes)?;
        let values = match self.info.dtype {
            SafetensorsDType::F8E4M3 => bytes.into_iter().map(e4m3_to_f32).collect(),
            SafetensorsDType::BF16 => bytes
                .chunks_exact(2)
                .map(|chunk| {
                    f32::from_bits(u32::from(u16::from_le_bytes([chunk[0], chunk[1]])) << 16)
                })
                .collect(),
            SafetensorsDType::F16 => bytes
                .chunks_exact(2)
                .map(|chunk| f16_bits_to_f32(u16::from_le_bytes([chunk[0], chunk[1]])))
                .collect(),
            SafetensorsDType::F32 => bytes
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect(),
            _ => unreachable!(),
        };
        Ok(values)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SafetensorsTensorInfo {
    pub(crate) dtype: SafetensorsDType,
    pub(crate) shape: Vec<usize>,
    pub(crate) data_offsets: (usize, usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SafetensorsDType {
    U8,
    U32,
    F8E4M3,
    F16,
    BF16,
    F32,
    Other,
}

impl SafetensorsDType {
    fn parse(value: &str) -> Self {
        match value {
            "U8" => Self::U8,
            "U32" => Self::U32,
            "F8_E4M3" | "F8E4M3" => Self::F8E4M3,
            "F16" => Self::F16,
            "BF16" => Self::BF16,
            "F32" => Self::F32,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Debug)]
struct SafetensorsHeader {
    tensors: BTreeMap<String, SafetensorsTensorInfo>,
}

impl SafetensorsHeader {
    fn read(path: &Path) -> Result<Self> {
        let mut file = File::open(path)
            .with_context(|| format!("failed to open safetensors shard `{}`", path.display()))?;
        let mut prefix = [0u8; 8];
        file.read_exact(&mut prefix).with_context(|| {
            format!(
                "failed to read safetensors header prefix from `{}`",
                path.display()
            )
        })?;
        let header_len = u64::from_le_bytes(prefix);
        let header_len_usize = usize::try_from(header_len).with_context(|| {
            format!(
                "safetensors header length exceeds usize for `{}`",
                path.display()
            )
        })?;
        file.seek(SeekFrom::Start(8)).with_context(|| {
            format!("failed to seek safetensors header in `{}`", path.display())
        })?;
        let mut header_bytes = vec![0u8; header_len_usize];
        file.read_exact(&mut header_bytes).with_context(|| {
            format!(
                "failed to read safetensors header from `{}`",
                path.display()
            )
        })?;
        let header_json: Value = serde_json::from_slice(&header_bytes).with_context(|| {
            format!("failed to parse safetensors header in `{}`", path.display())
        })?;
        let object = header_json.as_object().ok_or_else(|| {
            ModelOptNvfp4LoadError::InvalidSafetensorsHeader {
                path: path.to_path_buf(),
                detail: "header is not a JSON object".to_owned(),
            }
        })?;
        let mut tensors = BTreeMap::new();
        for (name, value) in object {
            if name == "__metadata__" {
                continue;
            }
            let dtype = value
                .get("dtype")
                .and_then(Value::as_str)
                .map(SafetensorsDType::parse)
                .ok_or_else(|| ModelOptNvfp4LoadError::InvalidSafetensorsHeader {
                    path: path.to_path_buf(),
                    detail: format!("tensor `{name}` is missing dtype"),
                })?;
            let shape = value
                .get("shape")
                .and_then(Value::as_array)
                .ok_or_else(|| ModelOptNvfp4LoadError::InvalidSafetensorsHeader {
                    path: path.to_path_buf(),
                    detail: format!("tensor `{name}` is missing shape"),
                })?
                .iter()
                .map(|dim| {
                    dim.as_u64()
                        .and_then(|dim| usize::try_from(dim).ok())
                        .ok_or_else(|| ModelOptNvfp4LoadError::InvalidSafetensorsHeader {
                            path: path.to_path_buf(),
                            detail: format!("tensor `{name}` has invalid shape dimension"),
                        })
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let offsets = value
                .get("data_offsets")
                .and_then(Value::as_array)
                .ok_or_else(|| ModelOptNvfp4LoadError::InvalidSafetensorsHeader {
                    path: path.to_path_buf(),
                    detail: format!("tensor `{name}` is missing data_offsets"),
                })?;
            if offsets.len() != 2 {
                return Err(ModelOptNvfp4LoadError::InvalidSafetensorsHeader {
                    path: path.to_path_buf(),
                    detail: format!("tensor `{name}` data_offsets length is not 2"),
                }
                .into());
            }
            let start = offsets[0]
                .as_u64()
                .and_then(|offset| usize::try_from(offset).ok())
                .ok_or_else(|| ModelOptNvfp4LoadError::InvalidSafetensorsHeader {
                    path: path.to_path_buf(),
                    detail: format!("tensor `{name}` has invalid start offset"),
                })?;
            let end = offsets[1]
                .as_u64()
                .and_then(|offset| usize::try_from(offset).ok())
                .ok_or_else(|| ModelOptNvfp4LoadError::InvalidSafetensorsHeader {
                    path: path.to_path_buf(),
                    detail: format!("tensor `{name}` has invalid end offset"),
                })?;
            if end < start {
                return Err(ModelOptNvfp4LoadError::InvalidSafetensorsHeader {
                    path: path.to_path_buf(),
                    detail: format!("tensor `{name}` end offset precedes start offset"),
                }
                .into());
            }
            tensors.insert(
                name.clone(),
                SafetensorsTensorInfo {
                    dtype,
                    shape,
                    data_offsets: (start, end),
                },
            );
        }

        Ok(Self { tensors })
    }

    fn tensor(&self, tensor_name: &str) -> Option<SafetensorsTensorInfo> {
        self.tensors.get(tensor_name).cloned()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PackedFp4TensorRef {
    pub(crate) tensor: SafetensorsTensorRef,
}

impl From<SafetensorsTensorRef> for PackedFp4TensorRef {
    fn from(tensor: SafetensorsTensorRef) -> Self {
        Self { tensor }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Fp8BlockScaleTensorRef {
    pub(crate) tensor: SafetensorsTensorRef,
}

impl From<SafetensorsTensorRef> for Fp8BlockScaleTensorRef {
    fn from(tensor: SafetensorsTensorRef) -> Self {
        Self { tensor }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TensorScaleRef {
    pub(crate) tensor: SafetensorsTensorRef,
}

impl From<SafetensorsTensorRef> for TensorScaleRef {
    fn from(tensor: SafetensorsTensorRef) -> Self {
        Self { tensor }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InputScaleTensorRef {
    pub(crate) tensor: SafetensorsTensorRef,
}

impl From<SafetensorsTensorRef> for InputScaleTensorRef {
    fn from(tensor: SafetensorsTensorRef) -> Self {
        Self { tensor }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PassthroughTensorRef {
    pub(crate) tensor: SafetensorsTensorRef,
}

impl From<SafetensorsTensorRef> for PassthroughTensorRef {
    fn from(tensor: SafetensorsTensorRef) -> Self {
        Self { tensor }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ModelOptLinearTensors {
    Nvfp4(Nvfp4LinearTensors),
    Fp8(Fp8LinearTensors),
    Passthrough(PassthroughTensorRef),
}

#[derive(Clone, Debug)]
pub(crate) enum PreparedLinear {
    Dense {
        rows: usize,
        cols: usize,
        weight: Vec<f32>,
    },
    Fp8 {
        rows: usize,
        cols: usize,
        weight: Vec<u8>,
        scale: f32,
    },
    Nvfp4 {
        rows: usize,
        cols: usize,
        packed: Vec<u8>,
        scales: Vec<u8>,
        tensor_scale: f32,
    },
}

impl PreparedLinear {
    pub(crate) fn resident_bytes(&self) -> u64 {
        match self {
            Self::Dense { weight, .. } => (weight.len() as u64).saturating_mul(4),
            Self::Fp8 { weight, .. } => weight.len() as u64,
            Self::Nvfp4 { packed, scales, .. } => {
                (packed.len() as u64).saturating_add(scales.len() as u64)
            }
        }
    }

    pub(crate) fn matvec(&self, input: &[f32]) -> Result<Vec<f32>> {
        let (rows, cols) = match self {
            Self::Dense { rows, cols, .. }
            | Self::Fp8 { rows, cols, .. }
            | Self::Nvfp4 { rows, cols, .. } => (*rows, *cols),
        };
        if input.len() != cols {
            bail!("linear input has {} values, expected {cols}", input.len());
        }
        let mut output = vec![0.0f32; rows];
        match self {
            Self::Dense { weight, .. } => {
                for (row, output_value) in output.iter_mut().enumerate() {
                    *output_value = weight[row * cols..(row + 1) * cols]
                        .iter()
                        .zip(input)
                        .map(|(weight, input)| weight * input)
                        .sum();
                }
            }
            Self::Fp8 { weight, scale, .. } => {
                for (row, output_value) in output.iter_mut().enumerate() {
                    *output_value = weight[row * cols..(row + 1) * cols]
                        .iter()
                        .zip(input)
                        .map(|(weight, input)| e4m3_to_f32(*weight) * scale * input)
                        .sum();
                }
            }
            Self::Nvfp4 {
                packed,
                scales,
                tensor_scale,
                ..
            } => {
                let packed_cols = cols / 2;
                let scale_cols = cols / NVFP4_BLOCK_SIZE;
                for row in 0..rows {
                    let mut sum = 0.0;
                    for block in 0..scale_cols {
                        let scale = e4m3_to_f32(scales[row * scale_cols + block]) * tensor_scale;
                        let packed_offset = row * packed_cols + block * (NVFP4_BLOCK_SIZE / 2);
                        let input_offset = block * NVFP4_BLOCK_SIZE;
                        for byte_index in 0..NVFP4_BLOCK_SIZE / 2 {
                            let byte = packed[packed_offset + byte_index];
                            let value_index = input_offset + byte_index * 2;
                            sum += E2M1_LUT[(byte >> 4) as usize] * scale * input[value_index];
                            sum +=
                                E2M1_LUT[(byte & 0x0f) as usize] * scale * input[value_index + 1];
                        }
                    }
                    output[row] = sum;
                }
            }
        }
        Ok(output)
    }

    pub(crate) fn matvec_native_nvfp4(&self, input: &[f32]) -> Result<Vec<f32>> {
        match self {
            Self::Nvfp4 {
                rows,
                cols,
                packed,
                scales,
                tensor_scale,
            } => {
                #[cfg(feature = "nvfp4-cuda")]
                {
                    return cuda::Nvfp4CudaBackend::linear_f32_host(
                        input,
                        packed,
                        scales,
                        *tensor_scale,
                        *rows,
                        *cols,
                    );
                }
                #[cfg(not(feature = "nvfp4-cuda"))]
                {
                    let _ = (rows, cols, packed, scales, tensor_scale, input);
                    bail!("native NVFP4 execution requires the `nvfp4-cuda` feature")
                }
            }
            _ => self.matvec(input),
        }
    }
}

impl ModelOptLinearTensors {
    pub(crate) fn logical_shape(&self) -> Result<(usize, usize)> {
        let shape = match self {
            Self::Nvfp4(linear) => {
                let shape = &linear.packed_weight.tensor.info.shape;
                vec![shape[0], shape[1] * 2]
            }
            Self::Fp8(linear) => linear.weight.info.shape.clone(),
            Self::Passthrough(linear) => linear.tensor.info.shape.clone(),
        };
        if shape.len() != 2 {
            bail!("linear weight must be rank 2, got {shape:?}");
        }
        Ok((shape[0], shape[1]))
    }

    pub(crate) fn dequantize_f32(&self, max_dense_bytes: Option<u64>) -> Result<Vec<f32>> {
        let (rows, cols) = self.logical_shape()?;
        let bytes = u64::try_from(rows)?
            .saturating_mul(u64::try_from(cols)?)
            .saturating_mul(4);
        if max_dense_bytes.is_some_and(|limit| bytes > limit) {
            bail!(
                "linear fallback needs {bytes} dense bytes for [{rows}, {cols}], exceeding limit {}",
                max_dense_bytes.unwrap_or_default()
            );
        }
        match self {
            Self::Passthrough(linear) => linear.tensor.read_f32(),
            Self::Fp8(linear) => {
                let scale = read_scalar_f32(&linear.weight_scale.tensor)?;
                Ok(linear
                    .weight
                    .read_f32()?
                    .into_iter()
                    .map(|value| value * scale)
                    .collect())
            }
            Self::Nvfp4(linear) => {
                let packed = linear.packed_weight.tensor.read_bytes()?;
                let block_scales = linear.block_scales.tensor.read_bytes()?;
                let tensor_scale = read_scalar_f32(&linear.tensor_scale.tensor)?;
                let Nvfp4DenseValues::F32(values) = dequantize_nvfp4_e4m3(
                    &packed,
                    &block_scales,
                    tensor_scale,
                    rows,
                    cols,
                    Nvfp4OutputDType::F32,
                )?
                else {
                    unreachable!("F32 output was requested")
                };
                Ok(values)
            }
        }
    }

    pub(crate) fn matvec_f32(
        &self,
        input: &[f32],
        max_dense_bytes: Option<u64>,
    ) -> Result<Vec<f32>> {
        let (_, cols) = self.logical_shape()?;
        if input.len() != cols {
            bail!("linear input has {} values, expected {cols}", input.len());
        }
        self.prepare(max_dense_bytes)?.matvec(input)
    }

    pub(crate) fn prepare(&self, max_dense_bytes: Option<u64>) -> Result<PreparedLinear> {
        let (rows, cols) = self.logical_shape()?;
        match self {
            Self::Nvfp4(linear) => {
                let packed = linear.packed_weight.tensor.read_bytes()?;
                let scales = linear.block_scales.tensor.read_bytes()?;
                let tensor_scale = read_scalar_f32(&linear.tensor_scale.tensor)?;
                Ok(PreparedLinear::Nvfp4 {
                    rows,
                    cols,
                    packed,
                    scales,
                    tensor_scale,
                })
            }
            Self::Fp8(linear) => {
                let weight = linear.weight.read_bytes()?;
                let scale = read_scalar_f32(&linear.weight_scale.tensor)?;
                Ok(PreparedLinear::Fp8 {
                    rows,
                    cols,
                    weight,
                    scale,
                })
            }
            Self::Passthrough(linear) => {
                let weight = linear.tensor.read_f32()?;
                let dense_bytes = (weight.len() as u64).saturating_mul(4);
                if max_dense_bytes.is_some_and(|limit| dense_bytes > limit) {
                    bail!(
                        "linear fallback needs {dense_bytes} dense bytes, exceeding limit {}",
                        max_dense_bytes.unwrap_or_default()
                    );
                }
                Ok(PreparedLinear::Dense { rows, cols, weight })
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Nvfp4LinearTensors {
    pub(crate) base_name: String,
    pub(crate) packed_weight: PackedFp4TensorRef,
    pub(crate) block_scales: Fp8BlockScaleTensorRef,
    pub(crate) tensor_scale: TensorScaleRef,
    pub(crate) input_scale: Option<InputScaleTensorRef>,
}

impl Nvfp4LinearTensors {
    pub(crate) fn validate_layout(&self, alias: &str) -> Result<()> {
        let packed_shape = &self.packed_weight.tensor.info.shape;
        if packed_shape.len() != 2 {
            return Err(ModelOptNvfp4LoadError::UnsupportedQuantization {
                alias: alias.to_owned(),
                detail: format!(
                    "{}.weight must have shape [N, K/2], got {:?}",
                    self.base_name, packed_shape
                ),
            }
            .into());
        }
        let n_rows = packed_shape[0];
        let packed_cols = packed_shape[1];
        let n_cols = packed_cols.checked_mul(2).ok_or_else(|| {
            ModelOptNvfp4LoadError::UnsupportedQuantization {
                alias: alias.to_owned(),
                detail: format!("{}.weight K dimension overflows", self.base_name),
            }
        })?;
        if n_rows == 0 || n_cols == 0 || !n_cols.is_multiple_of(NVFP4_BLOCK_SIZE) {
            return Err(ModelOptNvfp4LoadError::UnsupportedQuantization {
                alias: alias.to_owned(),
                detail: format!(
                    "{}.weight has invalid logical shape [{n_rows}, {n_cols}] for NVFP4 block size {NVFP4_BLOCK_SIZE}",
                    self.base_name
                ),
            }
            .into());
        }

        let expected_scales = [n_rows, n_cols / NVFP4_BLOCK_SIZE];
        let scale_shape = &self.block_scales.tensor.info.shape;
        if scale_shape.as_slice() != expected_scales {
            return Err(ModelOptNvfp4LoadError::UnsupportedQuantization {
                alias: alias.to_owned(),
                detail: format!(
                    "{}.weight_scale must have shape {:?}, got {:?}",
                    self.base_name, expected_scales, scale_shape
                ),
            }
            .into());
        }

        let tensor_scale_elems = elem_count(&self.tensor_scale.tensor.info.shape);
        if tensor_scale_elems != 1 {
            return Err(ModelOptNvfp4LoadError::UnsupportedQuantization {
                alias: alias.to_owned(),
                detail: format!(
                    "{}.weight_scale_2 must contain one scalar, got shape {:?}",
                    self.base_name, self.tensor_scale.tensor.info.shape
                ),
            }
            .into());
        }

        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Fp8LinearTensors {
    pub(crate) base_name: String,
    pub(crate) weight: SafetensorsTensorRef,
    pub(crate) weight_scale: Fp8BlockScaleTensorRef,
    pub(crate) input_scale: Option<InputScaleTensorRef>,
}

fn elem_count(shape: &[usize]) -> usize {
    if shape.is_empty() {
        1
    } else {
        shape.iter().product()
    }
}

fn div_ceil_u64(value: u64, divisor: u64) -> u64 {
    if divisor == 0 {
        value
    } else {
        value.div_ceil(divisor)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ModelOptNvfp4LoadError {
    MissingFile {
        alias: String,
        root: PathBuf,
        file: String,
    },
    MissingShard {
        alias: String,
        root: PathBuf,
        shard: String,
    },
    InvalidJson {
        alias: String,
        path: PathBuf,
        detail: String,
    },
    InvalidSafetensorsHeader {
        path: PathBuf,
        detail: String,
    },
    UnsupportedQuantization {
        alias: String,
        detail: String,
    },
    TensorNotFound {
        alias: String,
        tensor: String,
    },
    TensorNotFoundInShard {
        alias: String,
        tensor: String,
        shard: PathBuf,
    },
    MissingQuantizationComponent {
        alias: String,
        base: String,
        component: String,
    },
}

impl fmt::Display for ModelOptNvfp4LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingFile { alias, root, file } => write!(
                f,
                "ModelOpt/NVFP4 model `{alias}` is missing required file `{}` under `{}`",
                file,
                root.display()
            ),
            Self::MissingShard { alias, root, shard } => write!(
                f,
                "ModelOpt/NVFP4 model `{alias}` index references missing shard `{}` under `{}`",
                shard,
                root.display()
            ),
            Self::InvalidJson {
                alias,
                path,
                detail,
            } => write!(
                f,
                "ModelOpt/NVFP4 model `{alias}` has invalid JSON `{}`: {detail}",
                path.display()
            ),
            Self::InvalidSafetensorsHeader { path, detail } => write!(
                f,
                "invalid safetensors header in `{}`: {detail}",
                path.display()
            ),
            Self::UnsupportedQuantization { alias, detail } => write!(
                f,
                "ModelOpt/NVFP4 model `{alias}` has unsupported quantization layout: {detail}"
            ),
            Self::TensorNotFound { alias, tensor } => write!(
                f,
                "ModelOpt/NVFP4 model `{alias}` does not contain tensor `{tensor}`"
            ),
            Self::TensorNotFoundInShard {
                alias,
                tensor,
                shard,
            } => write!(
                f,
                "ModelOpt/NVFP4 model `{alias}` maps tensor `{tensor}` to `{}` but the shard header does not contain it",
                shard.display()
            ),
            Self::MissingQuantizationComponent {
                alias,
                base,
                component,
            } => write!(
                f,
                "ModelOpt/NVFP4 model `{alias}` linear `{base}` is missing quantization component `{component}`"
            ),
        }
    }
}

impl Error for ModelOptNvfp4LoadError {}

fn read_json(path: &Path, alias: &str) -> Result<Value> {
    let data = std::fs::read(path).map_err(|error| ModelOptNvfp4LoadError::InvalidJson {
        alias: alias.to_owned(),
        path: path.to_path_buf(),
        detail: error.to_string(),
    })?;
    serde_json::from_slice(&data).map_err(|error| {
        ModelOptNvfp4LoadError::InvalidJson {
            alias: alias.to_owned(),
            path: path.to_path_buf(),
            detail: error.to_string(),
        }
        .into()
    })
}

fn json_option_contains_text(value: Option<&Value>, needle: &str) -> bool {
    value.is_some_and(|value| json_contains_text(value, needle))
}

fn json_contains_text(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(value) => value.contains(needle),
        Value::Array(values) => values.iter().any(|value| json_contains_text(value, needle)),
        Value::Object(values) => values
            .values()
            .any(|value| json_contains_text(value, needle)),
        _ => false,
    }
}

fn find_group_size(value: &Value) -> Option<usize> {
    match value {
        Value::Object(map) => {
            if let Some(group_size) = map.get("group_size").and_then(Value::as_u64) {
                if let Ok(group_size) = usize::try_from(group_size) {
                    return Some(group_size);
                }
            }
            map.values().find_map(find_group_size)
        }
        Value::Array(values) => values.iter().find_map(find_group_size),
        _ => None,
    }
}

fn find_usize_key(value: &Value, key: &str) -> Option<usize> {
    match value {
        Value::Object(map) => {
            if let Some(value) = map
                .get(key)
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
            {
                return Some(value);
            }
            map.values().find_map(|value| find_usize_key(value, key))
        }
        Value::Array(values) => values.iter().find_map(|value| find_usize_key(value, key)),
        _ => None,
    }
}

fn layer_index_from_tensor_name(tensor: &str) -> Option<usize> {
    let marker = ".layers.";
    let start = tensor.find(marker)? + marker.len();
    let digits = tensor[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

pub(crate) fn e4m3_to_f32(byte: u8) -> f32 {
    let sign = if byte & 0x80 != 0 { -1.0 } else { 1.0 };
    let exp = ((byte >> 3) & 0x0f) as i32;
    let mant_bits = byte & 0x07;
    if exp == 0x0f && mant_bits == 0x07 {
        return f32::NAN;
    }
    if exp == 0 {
        sign * (mant_bits as f32 / 8.0) * (1.0 / 64.0)
    } else {
        sign * (1.0 + mant_bits as f32 / 8.0) * 2.0_f32.powi(exp - 7)
    }
}

pub(crate) fn dequantize_nvfp4_e4m3(
    packed: &[u8],
    block_scales_e4m3: &[u8],
    tensor_scale: f32,
    n_rows: usize,
    n_cols: usize,
    output_dtype: Nvfp4OutputDType,
) -> Result<Nvfp4DenseValues> {
    let block_scales = block_scales_e4m3
        .iter()
        .map(|&byte| e4m3_to_f32(byte))
        .collect::<Vec<_>>();
    dequantize_nvfp4_with_f32_scales(
        packed,
        &block_scales,
        tensor_scale,
        n_rows,
        n_cols,
        output_dtype,
    )
}

pub(crate) fn dequantize_nvfp4_with_f32_scales(
    packed: &[u8],
    block_scales: &[f32],
    tensor_scale: f32,
    n_rows: usize,
    n_cols: usize,
    output_dtype: Nvfp4OutputDType,
) -> Result<Nvfp4DenseValues> {
    if !n_cols.is_multiple_of(NVFP4_BLOCK_SIZE) {
        return Err(ModelOptNvfp4LoadError::UnsupportedQuantization {
            alias: "dequantize".to_owned(),
            detail: format!(
                "NVFP4 K dimension {n_cols} must be a multiple of block size {NVFP4_BLOCK_SIZE}"
            ),
        }
        .into());
    }
    let pack_cols = n_cols / 2;
    let scale_cols = n_cols / NVFP4_BLOCK_SIZE;
    let expected_packed = n_rows.checked_mul(pack_cols).ok_or_else(|| {
        ModelOptNvfp4LoadError::UnsupportedQuantization {
            alias: "dequantize".to_owned(),
            detail: "packed shape overflows usize".to_owned(),
        }
    })?;
    let expected_scales = n_rows.checked_mul(scale_cols).ok_or_else(|| {
        ModelOptNvfp4LoadError::UnsupportedQuantization {
            alias: "dequantize".to_owned(),
            detail: "scale shape overflows usize".to_owned(),
        }
    })?;
    if packed.len() != expected_packed {
        return Err(ModelOptNvfp4LoadError::UnsupportedQuantization {
            alias: "dequantize".to_owned(),
            detail: format!(
                "packed buffer has {} bytes, expected {expected_packed} for [{n_rows}, {pack_cols}]",
                packed.len()
            ),
        }
        .into());
    }
    if block_scales.len() != expected_scales {
        return Err(ModelOptNvfp4LoadError::UnsupportedQuantization {
            alias: "dequantize".to_owned(),
            detail: format!(
                "block scale buffer has {} entries, expected {expected_scales} for [{n_rows}, {scale_cols}]",
                block_scales.len()
            ),
        }
        .into());
    }

    let mut dense = Vec::with_capacity(n_rows * n_cols);
    let bytes_per_block = NVFP4_BLOCK_SIZE / 2;
    for row in 0..n_rows {
        let packed_row = &packed[row * pack_cols..(row + 1) * pack_cols];
        let scale_row = &block_scales[row * scale_cols..(row + 1) * scale_cols];
        for (block_idx, &scale) in scale_row.iter().enumerate() {
            let packed_offset = block_idx * bytes_per_block;
            for byte in &packed_row[packed_offset..packed_offset + bytes_per_block] {
                let hi = (byte >> 4) as usize;
                let lo = (byte & 0x0f) as usize;
                dense.push(E2M1_LUT[hi] * scale * tensor_scale);
                dense.push(E2M1_LUT[lo] * scale * tensor_scale);
            }
        }
    }

    match output_dtype {
        Nvfp4OutputDType::F32 => Ok(Nvfp4DenseValues::F32(dense)),
        Nvfp4OutputDType::BF16 => Ok(Nvfp4DenseValues::BF16(
            dense.into_iter().map(f32_to_bf16_bits).collect(),
        )),
    }
}

fn f32_to_bf16_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let lsb = (bits >> 16) & 1;
    let rounded = bits.wrapping_add(0x7fff + lsb);
    (rounded >> 16) as u16
}

fn f16_bits_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits & 0x8000) << 16;
    let exponent = (bits >> 10) & 0x1f;
    let fraction = bits & 0x03ff;
    let value = match exponent {
        0 if fraction == 0 => sign,
        0 => {
            let mut fraction = u32::from(fraction);
            let mut exponent = 113u32;
            while fraction & 0x0400 == 0 {
                fraction <<= 1;
                exponent -= 1;
            }
            sign | (exponent << 23) | ((fraction & 0x03ff) << 13)
        }
        0x1f => sign | 0x7f80_0000 | (u32::from(fraction) << 13),
        _ => sign | (u32::from(exponent + 112) << 23) | (u32::from(fraction) << 13),
    };
    f32::from_bits(value)
}

fn read_scalar_f32(tensor: &SafetensorsTensorRef) -> Result<f32> {
    let values = tensor.read_f32()?;
    if values.len() != 1 {
        bail!(
            "tensor `{}` must contain one scalar, got {} values",
            tensor.name,
            values.len()
        );
    }
    Ok(values[0])
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use serde_json::json;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    pub(crate) const TEST_LINEAR: &str = "model.layers.0.mlp.experts.0.down_proj";
    pub(crate) const TEST_SHARD: &str = "model-00001-of-00001.safetensors";

    pub(crate) fn temp_model_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("tachyon-{label}-{nanos}"));
        std::fs::create_dir_all(&dir).expect("temp model dir should be created");
        dir
    }

    pub(crate) fn write_minimal_modelopt_nvfp4_dir(root: &Path) {
        write_modelopt_config(root, true, NVFP4_BLOCK_SIZE);
        write_safetensors_shard(
            &root.join(TEST_SHARD),
            vec![
                (
                    &format!("{TEST_LINEAR}.weight"),
                    "U8",
                    vec![1, 8],
                    vec![0u8; 8],
                ),
                (
                    &format!("{TEST_LINEAR}.weight_scale"),
                    "F8_E4M3",
                    vec![1, 1],
                    vec![0x38],
                ),
                (
                    &format!("{TEST_LINEAR}.weight_scale_2"),
                    "F32",
                    vec![1],
                    1.0f32.to_le_bytes().to_vec(),
                ),
                (
                    "model.layers.0.input_layernorm.weight",
                    "BF16",
                    vec![4],
                    vec![0u8; 8],
                ),
            ],
        );
        write_index(
            root,
            TEST_SHARD,
            &[
                &format!("{TEST_LINEAR}.weight"),
                &format!("{TEST_LINEAR}.weight_scale"),
                &format!("{TEST_LINEAR}.weight_scale_2"),
                "model.layers.0.input_layernorm.weight",
            ],
        );
    }

    pub(crate) fn write_modelopt_config(root: &Path, include_nvfp4: bool, group_size: usize) {
        let quantization_config = if include_nvfp4 {
            json!({
                "config_groups": {
                    "group_0": {
                        "weights": { "num_bits": 4, "type": "float", "group_size": group_size },
                        "targets": ["model.layers.0.mlp.experts"]
                    }
                },
                "quant_method": "modelopt",
                "quant_algo": NVFP4_ALGO
            })
        } else {
            json!({})
        };
        let config = json!({
            "architectures": ["AnyForCausalLM"],
            "model_type": "any",
            "num_hidden_layers": 2,
            "quantization_config": quantization_config
        });
        std::fs::write(
            root.join(CONFIG_JSON),
            serde_json::to_vec(&config).expect("test config JSON should serialize"),
        )
        .expect("config should be written");
    }

    pub(crate) fn write_index(root: &Path, shard: &str, tensors: &[&str]) {
        let weight_map = tensors
            .iter()
            .map(|tensor| ((*tensor).to_owned(), Value::String(shard.to_owned())))
            .collect::<serde_json::Map<_, _>>();
        let index = json!({
            "metadata": {
                "total_parameters": 32,
                "total_size": 32
            },
            "weight_map": weight_map
        });
        std::fs::write(
            root.join(SAFETENSORS_INDEX_JSON),
            serde_json::to_vec(&index).expect("test index JSON should serialize"),
        )
        .expect("index should be written");
    }

    pub(crate) fn write_safetensors_shard(
        path: &Path,
        tensors: Vec<(&str, &str, Vec<usize>, Vec<u8>)>,
    ) {
        let mut offset = 0usize;
        let mut header = serde_json::Map::new();
        let mut payload = Vec::new();
        for (name, dtype, shape, bytes) in tensors {
            let start = offset;
            offset += bytes.len();
            let end = offset;
            header.insert(
                name.to_owned(),
                json!({
                    "dtype": dtype,
                    "shape": shape,
                    "data_offsets": [start, end],
                }),
            );
            payload.extend(bytes);
        }
        let header_bytes = serde_json::to_vec(&Value::Object(header))
            .expect("test safetensors header should serialize");
        let mut file = File::create(path).expect("shard should be created");
        file.write_all(&(header_bytes.len() as u64).to_le_bytes())
            .expect("header prefix should be written");
        file.write_all(&header_bytes)
            .expect("header should be written");
        file.write_all(&payload).expect("payload should be written");
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::*;
    use super::*;

    #[test]
    fn detects_valid_modelopt_nvfp4_directory() {
        let root = temp_model_dir("modelopt-nvfp4-valid");
        write_minimal_modelopt_nvfp4_dir(&root);

        let model = ModelOptNvfp4Directory::try_load("nvfp4", &root)
            .expect("loader should not error")
            .expect("directory should be detected");

        assert_eq!(model.alias(), "nvfp4");
        assert_eq!(model.quantization().group_size, NVFP4_BLOCK_SIZE);
        assert_eq!(model.quantization().layer_count, 2);
        assert!(model.quantization().has_nvfp4_layers);
        assert_eq!(model.shard_index.tensor_count(), 4);
        assert_eq!(model.shard_index.shard_count(), 1);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn non_nvfp4_directory_is_not_claimed() {
        let root = temp_model_dir("modelopt-non-nvfp4");
        write_modelopt_config(&root, false, NVFP4_BLOCK_SIZE);
        write_safetensors_shard(
            &root.join(TEST_SHARD),
            vec![(
                "model.layers.0.input_layernorm.weight",
                "BF16",
                vec![4],
                vec![0u8; 8],
            )],
        );
        write_index(
            &root,
            TEST_SHARD,
            &["model.layers.0.input_layernorm.weight"],
        );

        let model =
            ModelOptNvfp4Directory::try_load("other", &root).expect("loader should not error");

        assert!(model.is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn nvfp4_metadata_requires_safetensors_index() {
        let root = temp_model_dir("modelopt-missing-index");
        write_modelopt_config(&root, true, NVFP4_BLOCK_SIZE);

        let error = ModelOptNvfp4Directory::try_load("nvfp4", &root)
            .expect_err("missing index should fail");
        let message = error.to_string();

        assert!(message.contains(SAFETENSORS_INDEX_JSON));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn missing_shard_is_rejected() {
        let root = temp_model_dir("modelopt-missing-shard");
        write_modelopt_config(&root, true, NVFP4_BLOCK_SIZE);
        write_index(
            &root,
            "missing.safetensors",
            &[&format!("{TEST_LINEAR}.weight")],
        );

        let error = ModelOptNvfp4Directory::try_load("nvfp4", &root)
            .expect_err("missing shard should fail");
        let message = error.to_string();

        assert!(message.contains("missing shard"));
        assert!(message.contains("missing.safetensors"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unsupported_group_size_is_rejected() {
        let root = temp_model_dir("modelopt-bad-group-size");
        write_minimal_modelopt_nvfp4_dir(&root);
        write_modelopt_config(&root, true, 32);

        let error = ModelOptNvfp4Directory::try_load("nvfp4", &root)
            .expect_err("bad group size should fail");
        let message = error.to_string();

        assert!(message.contains("group size"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn tensor_location_and_header_lookup_use_shard_index() {
        let root = temp_model_dir("modelopt-tensor-lookup");
        write_minimal_modelopt_nvfp4_dir(&root);
        let model = ModelOptNvfp4Directory::try_load("nvfp4", &root)
            .expect("loader should not error")
            .expect("directory should be detected");
        let tensor_name = format!("{TEST_LINEAR}.weight");

        let location = model
            .tensor_location(&tensor_name)
            .expect("tensor location should resolve");
        let tensor = model
            .tensor_info(&tensor_name)
            .expect("tensor info should resolve");

        assert_eq!(location.shard_name, TEST_SHARD);
        assert_eq!(tensor.name, tensor_name);
        assert_eq!(tensor.info.dtype, SafetensorsDType::U8);
        assert_eq!(tensor.info.shape, vec![1, 8]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn modelopt_linear_groups_nvfp4_components() {
        let root = temp_model_dir("modelopt-linear-components");
        write_minimal_modelopt_nvfp4_dir(&root);
        let model = ModelOptNvfp4Directory::try_load("nvfp4", &root)
            .expect("loader should not error")
            .expect("directory should be detected");

        let component = model
            .modelopt_linear(TEST_LINEAR)
            .expect("linear component should load");

        match component {
            ModelOptLinearTensors::Nvfp4(linear) => {
                assert_eq!(linear.base_name, TEST_LINEAR);
                assert_eq!(linear.packed_weight.tensor.info.dtype, SafetensorsDType::U8);
                assert_eq!(
                    linear.block_scales.tensor.info.dtype,
                    SafetensorsDType::F8E4M3
                );
                assert_eq!(linear.tensor_scale.tensor.info.dtype, SafetensorsDType::F32);
            }
            other => panic!("expected NVFP4 component, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn modelopt_linear_rejects_missing_nvfp4_component() {
        let root = temp_model_dir("modelopt-linear-missing-component");
        write_modelopt_config(&root, true, NVFP4_BLOCK_SIZE);
        write_safetensors_shard(
            &root.join(TEST_SHARD),
            vec![
                (
                    &format!("{TEST_LINEAR}.weight"),
                    "U8",
                    vec![1, 8],
                    vec![0u8; 8],
                ),
                (
                    &format!("{TEST_LINEAR}.weight_scale_2"),
                    "F32",
                    vec![1],
                    1.0f32.to_le_bytes().to_vec(),
                ),
            ],
        );
        write_index(
            &root,
            TEST_SHARD,
            &[
                &format!("{TEST_LINEAR}.weight"),
                &format!("{TEST_LINEAR}.weight_scale_2"),
            ],
        );
        let model = ModelOptNvfp4Directory::try_load("nvfp4", &root)
            .expect("loader should not error")
            .expect("directory should be detected");

        let error = model
            .modelopt_linear(TEST_LINEAR)
            .expect_err("missing weight_scale should fail");

        assert!(error.to_string().contains("weight_scale"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn modelopt_linear_rejects_malformed_nvfp4_shapes() {
        let root = temp_model_dir("modelopt-linear-bad-shape");
        write_modelopt_config(&root, true, NVFP4_BLOCK_SIZE);
        write_safetensors_shard(
            &root.join(TEST_SHARD),
            vec![
                (
                    &format!("{TEST_LINEAR}.weight"),
                    "U8",
                    vec![8],
                    vec![0u8; 8],
                ),
                (
                    &format!("{TEST_LINEAR}.weight_scale"),
                    "F8_E4M3",
                    vec![1, 1],
                    vec![0x38],
                ),
                (
                    &format!("{TEST_LINEAR}.weight_scale_2"),
                    "F32",
                    vec![1],
                    1.0f32.to_le_bytes().to_vec(),
                ),
            ],
        );
        write_index(
            &root,
            TEST_SHARD,
            &[
                &format!("{TEST_LINEAR}.weight"),
                &format!("{TEST_LINEAR}.weight_scale"),
                &format!("{TEST_LINEAR}.weight_scale_2"),
            ],
        );
        let model = ModelOptNvfp4Directory::try_load("nvfp4", &root)
            .expect("loader should not error")
            .expect("directory should be detected");

        let error = model
            .modelopt_linear(TEST_LINEAR)
            .expect_err("bad packed shape should fail");

        assert!(error.to_string().contains("shape [N, K/2]"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn e4m3_known_values_convert_to_f32() {
        assert_eq!(e4m3_to_f32(0x00), 0.0);
        assert_eq!(e4m3_to_f32(0x80), 0.0);
        assert_eq!(e4m3_to_f32(0x38), 1.0);
        assert_eq!(e4m3_to_f32(0xb8), -1.0);
        assert_eq!(e4m3_to_f32(0x40), 2.0);
        assert_eq!(e4m3_to_f32(0x30), 0.5);
        assert!(e4m3_to_f32(0x7f).is_nan());
    }

    #[test]
    fn dequantize_nvfp4_respects_nibble_order_block_scale_and_tensor_scale() {
        let packed = [0x23, 0x45, 0x67, 0x9a, 0xbc, 0xde, 0xf1, 0x02];
        let values = dequantize_nvfp4_e4m3(&packed, &[0x40], 0.5, 1, 16, Nvfp4OutputDType::F32)
            .expect("dequant should succeed");

        let Nvfp4DenseValues::F32(values) = values else {
            panic!("expected f32 output")
        };
        assert_eq!(
            values,
            vec![
                1.0, 1.5, 2.0, 3.0, 4.0, 6.0, -0.5, -1.0, -1.5, -2.0, -3.0, -4.0, -6.0, 0.5, 0.0,
                1.0
            ]
        );
    }

    #[test]
    fn dequantize_nvfp4_can_emit_bf16_bits() {
        let packed = [0x22; 8];
        let values = dequantize_nvfp4_e4m3(&packed, &[0x38], 1.0, 1, 16, Nvfp4OutputDType::BF16)
            .expect("dequant should succeed");

        let Nvfp4DenseValues::BF16(values) = values else {
            panic!("expected bf16 output")
        };
        assert_eq!(values.len(), 16);
        assert!(values.iter().all(|&bits| bits == 0x3f80));
    }

    #[test]
    fn dequantize_nvfp4_rejects_invalid_k_dimension() {
        let error = dequantize_nvfp4_e4m3(&[0; 8], &[0x38], 1.0, 1, 15, Nvfp4OutputDType::F32)
            .expect_err("invalid K should fail");

        assert!(error.to_string().contains("multiple of block size"));
    }

    #[test]
    fn quantized_linear_fallback_matvec_is_bounded() {
        let root = temp_model_dir("modelopt-linear-matvec");
        write_minimal_modelopt_nvfp4_dir(&root);
        let model = ModelOptNvfp4Directory::try_load("nvfp4", &root)
            .expect("loader should not error")
            .expect("directory should be detected");
        let linear = model
            .modelopt_linear(TEST_LINEAR)
            .expect("linear should load");

        let output = linear
            .matvec_f32(&[1.0; 16], Some(1024))
            .expect("bounded fallback should run");
        assert_eq!(output, vec![0.0]);

        let bounded = linear
            .matvec_f32(&[1.0; 16], Some(1))
            .expect("packed matvec must not allocate a dense weight window");
        assert_eq!(bounded, vec![0.0]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn f16_conversion_handles_normal_subnormal_and_infinity() {
        assert_eq!(f16_bits_to_f32(0x3c00), 1.0);
        assert!(f16_bits_to_f32(0x0001) > 0.0);
        assert!(f16_bits_to_f32(0x7c00).is_infinite());
    }

    #[test]
    fn mixed_dense_fp8_nvfp4_graph_matches_expected_output() {
        let mut dense_weight = vec![0.0; 16 * 16];
        let mut fp8_weight = vec![0u8; 16 * 16];
        for index in 0..16 {
            dense_weight[index * 16 + index] = 1.0;
            fp8_weight[index * 16 + index] = 0x38;
        }
        let dense = PreparedLinear::Dense {
            rows: 16,
            cols: 16,
            weight: dense_weight,
        };
        let fp8 = PreparedLinear::Fp8 {
            rows: 16,
            cols: 16,
            weight: fp8_weight,
            scale: 1.0,
        };
        let nvfp4 = PreparedLinear::Nvfp4 {
            rows: 1,
            cols: 16,
            packed: vec![0x22; 8],
            scales: vec![0x38],
            tensor_scale: 1.0,
        };
        let dense_output = dense.matvec(&[1.0; 16]).expect("dense");
        let fp8_output = fp8.matvec(&dense_output).expect("fp8");
        let output = nvfp4.matvec(&fp8_output).expect("nvfp4");
        assert_eq!(output, vec![16.0]);
    }

    #[cfg(feature = "nvfp4-cuda")]
    #[test]
    fn nvfp4_cuda_host_linear_matches_fallback() {
        if std::env::var("TACHYON_NVFP4_CUDA_SMOKE").as_deref() != Ok("1") {
            return;
        }
        let linear = PreparedLinear::Nvfp4 {
            rows: 2,
            cols: 16,
            packed: vec![0x22; 16],
            scales: vec![0x38; 2],
            tensor_scale: 1.0,
        };
        let input = vec![1.0f32; 16];
        let fallback = linear.matvec(&input).expect("fallback");
        let native = linear
            .matvec_native_nvfp4(&input)
            .expect("native CUDA linear");
        for (actual, expected) in native.iter().zip(fallback) {
            assert!((actual - expected).abs() < 1e-4);
        }
    }

    #[test]
    fn fallback_memory_estimate_covers_eager_and_layer_window() {
        let root = temp_model_dir("modelopt-memory-estimate");
        write_minimal_modelopt_nvfp4_dir(&root);
        let model = ModelOptNvfp4Directory::try_load("nvfp4", &root)
            .expect("loader should not error")
            .expect("directory should be detected");

        let eager =
            model.estimate_fallback_memory(Nvfp4OutputDType::F32, Nvfp4FallbackScope::Eager);
        let window = model
            .estimate_fallback_memory(Nvfp4OutputDType::F32, Nvfp4FallbackScope::LayerWindow(1));

        assert_eq!(eager.active_layers, 2);
        assert_eq!(window.active_layers, 1);
        assert!(eager.dense_active_bytes > window.dense_active_bytes);
        assert_eq!(eager.packed_model_bytes, 32);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn fallback_memory_limit_rejects_oversized_dequantization() {
        let root = temp_model_dir("modelopt-memory-limit");
        write_minimal_modelopt_nvfp4_dir(&root);
        let model = ModelOptNvfp4Directory::try_load("nvfp4", &root)
            .expect("loader should not error")
            .expect("directory should be detected");

        let error = model
            .ensure_fallback_memory_within_limits(
                Nvfp4OutputDType::F32,
                Nvfp4FallbackScope::Eager,
                Nvfp4FallbackMemoryLimits {
                    max_host_ram_bytes: Some(1),
                    max_accelerator_bytes: None,
                },
            )
            .expect_err("too small limit should fail");

        assert!(error.to_string().contains("host RAM"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn native_kernel_plan_is_selected_when_capabilities_are_complete() {
        let root = temp_model_dir("modelopt-native-plan");
        write_minimal_modelopt_nvfp4_dir(&root);
        let model = ModelOptNvfp4Directory::try_load("nvfp4", &root)
            .expect("loader should not error")
            .expect("directory should be detected");
        let capabilities = Nvfp4AcceleratorCapabilities::native_fp4([
            Nvfp4KernelKind::Dequantize,
            Nvfp4KernelKind::Matmul,
        ]);

        let plan = model
            .select_execution_plan(
                &capabilities,
                Nvfp4OutputDType::BF16,
                Nvfp4FallbackScope::LayerWindow(1),
                Nvfp4FallbackMemoryLimits::default(),
                Nvfp4KernelSelectionMode::NativePreferred,
            )
            .expect("native plan should be selected");

        assert_eq!(
            plan,
            Nvfp4ExecutionPlan::Native {
                required_kernels: vec![Nvfp4KernelKind::Dequantize, Nvfp4KernelKind::Matmul],
            }
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn fallback_plan_is_selected_when_native_kernels_are_missing() {
        let root = temp_model_dir("modelopt-fallback-plan");
        write_minimal_modelopt_nvfp4_dir(&root);
        let model = ModelOptNvfp4Directory::try_load("nvfp4", &root)
            .expect("loader should not error")
            .expect("directory should be detected");

        let plan = model
            .select_execution_plan(
                &Nvfp4AcceleratorCapabilities::fallback_only(),
                Nvfp4OutputDType::BF16,
                Nvfp4FallbackScope::LayerWindow(1),
                Nvfp4FallbackMemoryLimits {
                    max_host_ram_bytes: Some(1024),
                    max_accelerator_bytes: Some(1024),
                },
                Nvfp4KernelSelectionMode::NativePreferred,
            )
            .expect("fallback plan should be selected");

        match plan {
            Nvfp4ExecutionPlan::Fallback(estimate) => {
                assert_eq!(estimate.active_layers, 1);
                assert!(estimate.host_ram_bytes <= 1024);
            }
            other => panic!("expected fallback plan, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn native_required_rejects_missing_kernel_support() {
        let root = temp_model_dir("modelopt-native-required");
        write_minimal_modelopt_nvfp4_dir(&root);
        let model = ModelOptNvfp4Directory::try_load("nvfp4", &root)
            .expect("loader should not error")
            .expect("directory should be detected");

        let error = model
            .select_execution_plan(
                &Nvfp4AcceleratorCapabilities::fallback_only(),
                Nvfp4OutputDType::BF16,
                Nvfp4FallbackScope::LayerWindow(1),
                Nvfp4FallbackMemoryLimits::default(),
                Nvfp4KernelSelectionMode::NativeRequired,
            )
            .expect_err("native-required should reject missing kernels");

        assert!(error
            .to_string()
            .contains("native NVFP4 execution requires"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(not(feature = "nvfp4-cuda"))]
    #[test]
    fn cuda_cutlass_capability_is_unavailable_without_feature() {
        let capabilities = Nvfp4AcceleratorCapabilities::cuda_cutlass();

        assert!(!capabilities.native_fp4);
        assert!(!capabilities.runtime_available);
        assert!(capabilities.compiled_kernels.is_empty());
    }

    #[test]
    fn layer_tensor_maps_group_components_by_tensor_name() {
        let root = temp_model_dir("modelopt-layer-map");
        write_modelopt_config(&root, true, NVFP4_BLOCK_SIZE);
        let layer0 = "model.layers.0.mlp.down_proj";
        let layer1 = "model.layers.1.mlp.down_proj";
        write_safetensors_shard(
            &root.join(TEST_SHARD),
            vec![
                (&format!("{layer0}.weight"), "U8", vec![1, 8], vec![0u8; 8]),
                (
                    &format!("{layer0}.weight_scale_2"),
                    "F32",
                    vec![1],
                    1.0f32.to_le_bytes().to_vec(),
                ),
                (&format!("{layer1}.weight"), "U8", vec![1, 8], vec![0u8; 8]),
                (
                    &format!("{layer1}.weight_scale_2"),
                    "F32",
                    vec![1],
                    1.0f32.to_le_bytes().to_vec(),
                ),
            ],
        );
        write_index(
            &root,
            TEST_SHARD,
            &[
                &format!("{layer0}.weight"),
                &format!("{layer0}.weight_scale_2"),
                &format!("{layer1}.weight"),
                &format!("{layer1}.weight_scale_2"),
            ],
        );
        let model = ModelOptNvfp4Directory::try_load("nvfp4", &root)
            .expect("loader should not error")
            .expect("directory should be detected");

        let maps = model.layer_tensor_maps();
        assert_eq!(maps.len(), 2);
        assert_eq!(maps[0].layer_index, 0);
        assert_eq!(maps[1].layer_index, 1);
        assert!(maps[1]
            .tensor_names
            .iter()
            .all(|name| name.contains(".layers.1.")));

        let refs = model
            .layer_tensor_refs(1)
            .expect("layer refs should resolve through shard headers");
        assert_eq!(refs.len(), 2);
        assert!(refs.iter().all(|tensor| tensor.name.contains(".layers.1.")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn sharded_layer_refs_resolve_without_full_model_residency() {
        let root = temp_model_dir("modelopt-sharded-layer-map");
        write_modelopt_config(&root, true, NVFP4_BLOCK_SIZE);
        let shard0 = "model-00001-of-00002.safetensors";
        let shard1 = "model-00002-of-00002.safetensors";
        let layer0_weight = "model.layers.0.mlp.down_proj.weight";
        let layer0_scale = "model.layers.0.mlp.down_proj.weight_scale_2";
        let layer1_weight = "model.layers.1.mlp.down_proj.weight";
        let layer1_scale = "model.layers.1.mlp.down_proj.weight_scale_2";
        write_safetensors_shard(
            &root.join(shard0),
            vec![
                (layer0_weight, "U8", vec![1, 8], vec![0u8; 8]),
                (layer0_scale, "F32", vec![1], 1.0f32.to_le_bytes().to_vec()),
            ],
        );
        write_safetensors_shard(
            &root.join(shard1),
            vec![
                (layer1_weight, "U8", vec![1, 8], vec![0u8; 8]),
                (layer1_scale, "F32", vec![1], 1.0f32.to_le_bytes().to_vec()),
            ],
        );
        let index = serde_json::json!({
            "metadata": { "total_parameters": 64, "total_size": 64 },
            "weight_map": {
                layer0_weight: shard0,
                layer0_scale: shard0,
                layer1_weight: shard1,
                layer1_scale: shard1
            }
        });
        std::fs::write(
            root.join(SAFETENSORS_INDEX_JSON),
            serde_json::to_vec(&index).expect("test index JSON should serialize"),
        )
        .expect("index should be written");
        let model = ModelOptNvfp4Directory::try_load("nvfp4", &root)
            .expect("loader should not error")
            .expect("directory should be detected");

        let layer1_refs = model
            .layer_tensor_refs(1)
            .expect("layer refs should resolve through the second shard");
        assert_eq!(layer1_refs.len(), 2);
        assert!(layer1_refs
            .iter()
            .all(|tensor| tensor.shard_path.ends_with(shard1)));

        let eager =
            model.estimate_fallback_memory(Nvfp4OutputDType::BF16, Nvfp4FallbackScope::Eager);
        let layer_window = model
            .estimate_fallback_memory(Nvfp4OutputDType::BF16, Nvfp4FallbackScope::LayerWindow(1));
        assert!(layer_window.accelerator_bytes < eager.accelerator_bytes);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn gated_real_checkpoint_probe_loads_modelopt_nvfp4_directory() {
        let Ok(path) = std::env::var("TACHYON_MODEL_OPT_NVFP4_DIR") else {
            return;
        };

        let model = ModelOptNvfp4Directory::try_load("probe", path)
            .expect("probe checkpoint should load without parser errors")
            .expect("probe checkpoint should be detected as ModelOpt/NVFP4");

        assert!(model.quantization().has_nvfp4_layers);
        assert!(model.shard_index.tensor_count() > 0);
        assert!(model.shard_index.shard_count() > 0);
    }
}
