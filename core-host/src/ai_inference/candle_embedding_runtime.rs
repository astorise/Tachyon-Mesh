use std::{
    collections::HashMap,
    fs,
    path::{Component, Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};
use candle_core::{DType, Device, Tensor};
use candle_onnx::onnx::ModelProto;
use prost::Message as _;
use serde::Deserialize;
use tokenizers::Tokenizer;

const MODEL_META_JSON: &str = ".tachyon-model.json";
const TOKENIZER_JSON: &str = "tokenizer.json";
const DEFAULT_ONNX_FILE: &str = "model.onnx";
const DEFAULT_MAX_LENGTH: usize = 512;

#[derive(Debug, Deserialize)]
struct EmbeddingModelMeta {
    #[serde(default)]
    format: String,
    #[serde(default)]
    task: String,
    #[serde(default)]
    model_file: Option<String>,
    #[serde(default)]
    output: Option<String>,
    #[serde(default)]
    pooling: Option<PoolingMode>,
    #[serde(default)]
    normalize: Option<bool>,
    #[serde(default)]
    max_length: Option<usize>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PoolingMode {
    #[default]
    Mean,
    Cls,
}

pub(crate) struct CandleEmbeddingRuntime {
    alias: String,
    root: PathBuf,
    model: ModelProto,
    tokenizer: Tokenizer,
    input_names: Vec<String>,
    output_names: Vec<String>,
    output_name: String,
    pooling: PoolingMode,
    normalize: bool,
    max_length: usize,
    device: Device,
}

impl CandleEmbeddingRuntime {
    pub(crate) fn try_load(alias: &str, root: impl AsRef<Path>) -> Result<Option<Self>> {
        let root = root.as_ref();
        if !root.is_dir() {
            return Ok(None);
        }

        let meta = read_meta(root)?;
        let onnx_path = match resolve_onnx_path(root, meta.as_ref())? {
            Some(path) => path,
            None => return Ok(None),
        };
        let tokenizer_path = root.join(TOKENIZER_JSON);
        if !tokenizer_path.is_file() {
            if declares_embedding_onnx(meta.as_ref()) {
                bail!(
                    "embedding ONNX model `{alias}` at `{}` requires `{TOKENIZER_JSON}`",
                    root.display()
                );
            }
            return Ok(None);
        }

        let bytes = fs::read(&onnx_path)
            .with_context(|| format!("failed to read ONNX model `{}`", onnx_path.display()))?;
        let model = ModelProto::decode(bytes.as_slice())
            .with_context(|| format!("failed to parse ONNX model `{}`", onnx_path.display()))?;
        let input_names = graph_input_names(&model);
        let output_names = graph_output_names(&model);
        if output_names.is_empty() {
            bail!("embedding ONNX model `{alias}` declares no graph outputs");
        }
        let output_name = resolve_output_name(meta.as_ref(), &output_names)?;
        let tokenizer = Tokenizer::from_file(&tokenizer_path).map_err(|error| {
            anyhow!(
                "failed to load tokenizer `{}` for embedding model `{alias}`: {error}",
                tokenizer_path.display()
            )
        })?;

        Ok(Some(Self {
            alias: alias.to_owned(),
            root: root.to_path_buf(),
            model,
            tokenizer,
            input_names,
            output_names,
            output_name,
            pooling: meta
                .as_ref()
                .and_then(|meta| meta.pooling)
                .unwrap_or_default(),
            normalize: meta
                .as_ref()
                .and_then(|meta| meta.normalize)
                .unwrap_or(true),
            max_length: meta
                .as_ref()
                .and_then(|meta| meta.max_length)
                .filter(|value| *value > 0)
                .unwrap_or(DEFAULT_MAX_LENGTH),
            device: Device::Cpu,
        }))
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn embed(&self, input: &str) -> Result<Vec<f32>> {
        let encoding = self
            .tokenizer
            .encode(input, true)
            .map_err(|error| anyhow!("tokenization failed for `{}`: {error}", self.alias))?;
        let mut input_ids = encoding
            .get_ids()
            .iter()
            .map(|id| *id as i64)
            .collect::<Vec<_>>();
        let mut attention_mask = encoding
            .get_attention_mask()
            .iter()
            .map(|id| *id as i64)
            .collect::<Vec<_>>();
        let mut token_type_ids = encoding
            .get_type_ids()
            .iter()
            .map(|id| *id as i64)
            .collect::<Vec<_>>();

        truncate_to_max_length(&mut input_ids, self.max_length);
        truncate_to_max_length(&mut attention_mask, self.max_length);
        truncate_to_max_length(&mut token_type_ids, self.max_length);
        if input_ids.is_empty() {
            bail!(
                "tokenizer produced no tokens for embedding model `{}`",
                self.alias
            );
        }

        let seq_len = input_ids.len();
        let mut inputs = HashMap::new();
        insert_i64_input_if_present(
            &mut inputs,
            &self.input_names,
            "input_ids",
            &input_ids,
            seq_len,
            &self.device,
        )?;
        insert_i64_input_if_present(
            &mut inputs,
            &self.input_names,
            "attention_mask",
            &attention_mask,
            seq_len,
            &self.device,
        )?;
        insert_i64_input_if_present(
            &mut inputs,
            &self.input_names,
            "token_type_ids",
            &token_type_ids,
            seq_len,
            &self.device,
        )?;

        let outputs = candle_onnx::simple_eval(&self.model, inputs, &self.device)
            .with_context(|| format!("embedding ONNX forward failed for `{}`", self.alias))?;
        let output = outputs.get(&self.output_name).ok_or_else(|| {
            anyhow!(
                "embedding ONNX output `{}` was not produced; available outputs: {}",
                self.output_name,
                self.output_names.join(", ")
            )
        })?;
        let mut embedding = match output.dims() {
            [1, hidden] => output
                .to_dtype(DType::F32)?
                .reshape((*hidden,))?
                .to_vec1::<f32>()?,
            [1, seq, hidden] => {
                let values = output
                    .to_dtype(DType::F32)?
                    .reshape((*seq, *hidden))?
                    .to_vec2::<f32>()?;
                pool_sequence(&values, &attention_mask, self.pooling)?
            }
            dims => bail!(
                "embedding ONNX output `{}` has unsupported shape {:?}; expected [1, hidden] or [1, seq, hidden]",
                self.output_name,
                dims
            ),
        };
        if self.normalize {
            l2_normalize(&mut embedding);
        }
        Ok(embedding)
    }
}

fn read_meta(root: &Path) -> Result<Option<EmbeddingModelMeta>> {
    let path = root.join(MODEL_META_JSON);
    if !path.is_file() {
        return Ok(None);
    }
    let raw = fs::read(&path)
        .with_context(|| format!("failed to read model metadata `{}`", path.display()))?;
    serde_json::from_slice(&raw)
        .with_context(|| format!("failed to parse model metadata `{}`", path.display()))
        .map(Some)
}

fn resolve_onnx_path(root: &Path, meta: Option<&EmbeddingModelMeta>) -> Result<Option<PathBuf>> {
    if let Some(meta) = meta {
        if let Some(file) = meta.model_file.as_deref() {
            validate_relative_model_file(file)?;
            let path = root.join(file);
            if !path.is_file() {
                bail!(
                    "`{MODEL_META_JSON}` points to ONNX file `{}` but it does not exist",
                    path.display()
                );
            }
            return Ok(Some(path));
        }
    }
    let default = root.join(DEFAULT_ONNX_FILE);
    if default.is_file() {
        return Ok(Some(default));
    }
    let mut onnx_files = fs::read_dir(root)
        .with_context(|| format!("failed to scan model directory `{}`", root.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("onnx"))
        .collect::<Vec<_>>();
    onnx_files.sort();
    Ok(onnx_files.into_iter().next())
}

fn validate_relative_model_file(file: &str) -> Result<()> {
    let path = Path::new(file);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("embedding ONNX `model_file` must be a simple relative file name");
    }
    Ok(())
}

fn declares_embedding_onnx(meta: Option<&EmbeddingModelMeta>) -> bool {
    meta.is_some_and(|meta| {
        meta.format.eq_ignore_ascii_case("onnx") || meta.task.eq_ignore_ascii_case("embeddings")
    })
}

fn resolve_output_name(
    meta: Option<&EmbeddingModelMeta>,
    output_names: &[String],
) -> Result<String> {
    if let Some(configured) = meta.and_then(|meta| meta.output.as_deref()) {
        if output_names.iter().any(|name| name == configured) {
            return Ok(configured.to_owned());
        }
        bail!(
            "configured embedding output `{configured}` is not present; available outputs: {}",
            output_names.join(", ")
        );
    }
    for preferred in [
        "last_hidden_state",
        "token_embeddings",
        "sentence_embedding",
    ] {
        if output_names.iter().any(|name| name == preferred) {
            return Ok(preferred.to_owned());
        }
    }
    Ok(output_names[0].clone())
}

fn graph_input_names(model: &ModelProto) -> Vec<String> {
    model
        .graph
        .as_ref()
        .map(|graph| graph.input.iter().map(|input| input.name.clone()).collect())
        .unwrap_or_default()
}

fn graph_output_names(model: &ModelProto) -> Vec<String> {
    model
        .graph
        .as_ref()
        .map(|graph| {
            graph
                .output
                .iter()
                .map(|output| output.name.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn insert_i64_input_if_present(
    inputs: &mut HashMap<String, Tensor>,
    input_names: &[String],
    name: &str,
    values: &[i64],
    seq_len: usize,
    device: &Device,
) -> Result<()> {
    if input_names.iter().any(|input| input == name) {
        let tensor = Tensor::from_vec(values.to_vec(), (1, seq_len), device)
            .with_context(|| format!("failed to build `{name}` tensor"))?;
        inputs.insert(name.to_owned(), tensor);
    }
    Ok(())
}

fn truncate_to_max_length(values: &mut Vec<i64>, max_length: usize) {
    if values.len() > max_length {
        values.truncate(max_length);
    }
}

fn pool_sequence(
    values: &[Vec<f32>],
    attention_mask: &[i64],
    pooling: PoolingMode,
) -> Result<Vec<f32>> {
    let hidden = values
        .first()
        .map(Vec::len)
        .ok_or_else(|| anyhow!("embedding output sequence is empty"))?;
    match pooling {
        PoolingMode::Cls => Ok(values[0].clone()),
        PoolingMode::Mean => {
            let mut pooled = vec![0.0; hidden];
            let mut count = 0.0f32;
            for (row, mask) in values.iter().zip(attention_mask.iter()) {
                if *mask == 0 {
                    continue;
                }
                for (slot, value) in pooled.iter_mut().zip(row) {
                    *slot += *value;
                }
                count += 1.0;
            }
            if count == 0.0 {
                bail!("attention mask selected no tokens for mean pooling");
            }
            for value in &mut pooled {
                *value /= count;
            }
            Ok(pooled)
        }
    }
}

fn l2_normalize(values: &mut [f32]) {
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm == 0.0 {
        return;
    }
    for value in values {
        *value /= norm;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mean_pooling_uses_attention_mask() {
        let values = vec![vec![1.0, 3.0], vec![3.0, 5.0], vec![100.0, 100.0]];
        let pooled = pool_sequence(&values, &[1, 1, 0], PoolingMode::Mean).expect("pool");

        assert_eq!(pooled, vec![2.0, 4.0]);
    }

    #[test]
    fn l2_normalize_scales_nonzero_vector() {
        let mut values = vec![3.0, 4.0];
        l2_normalize(&mut values);

        assert!((values[0] - 0.6).abs() < 0.0001);
        assert!((values[1] - 0.8).abs() < 0.0001);
    }
}
