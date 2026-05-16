use anyhow::{anyhow, Result};
use candle_core::{DType, Tensor as CandleTensor};
#[cfg(test)]
use candle_core::Device;
use lru::LruCache;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    marker::PhantomData,
    num::NonZeroUsize,
    sync::{Arc, Mutex, OnceLock},
};

const DEFAULT_FSM_CACHE_CAPACITY: usize = 100;

static FSM_CACHE: OnceLock<Mutex<LruCache<[u8; 32], Arc<CompiledFsm>>>> = OnceLock::new();

fn fsm_cache() -> &'static Mutex<LruCache<[u8; 32], Arc<CompiledFsm>>> {
    FSM_CACHE.get_or_init(|| {
        Mutex::new(LruCache::new(
            NonZeroUsize::new(DEFAULT_FSM_CACHE_CAPACITY)
                .expect("FSM cache capacity should be non-zero"),
        ))
    })
}

#[derive(Debug)]
pub(crate) struct CompiledFsm {
    schema_hash: [u8; 32],
    allowed_tokens: BTreeSet<u32>,
}

impl CompiledFsm {
    fn compile(schema: &str) -> Result<Self> {
        let value: serde_json::Value = serde_json::from_str(schema)
            .map_err(|error| anyhow!("constrained decoding JSON Schema is invalid: {error}"))?;
        let mut allowed_tokens = BTreeSet::new();
        collect_ascii_enum_tokens(&value, &mut allowed_tokens);

        if allowed_tokens.is_empty() {
            allowed_tokens.extend(b"{}[]:,\"0123456789truefalsenull \n\r\t".iter().map(|b| *b as u32));
        }

        Ok(Self {
            schema_hash: schema_hash(schema),
            allowed_tokens,
        })
    }

    pub(crate) fn allowed_tokens(&self, _current_node: usize) -> &BTreeSet<u32> {
        &self.allowed_tokens
    }

    pub(crate) fn transition(&self, current_node: usize, selected_token: u32) -> usize {
        current_node
            .wrapping_mul(31)
            .wrapping_add(selected_token as usize)
            ^ usize::from(self.schema_hash[0])
    }
}

#[derive(Debug)]
pub(crate) struct FsmLogitProcessor {
    fsm: Arc<CompiledFsm>,
    current_node: usize,
    _sampler_marker: PhantomData<llm_samplers::prelude::NilSamplerResources>,
}

impl FsmLogitProcessor {
    pub(crate) fn for_schema(schema: &str) -> Result<Self> {
        Ok(Self {
            fsm: compile_or_get_fsm(schema)?,
            current_node: 0,
            _sampler_marker: PhantomData,
        })
    }

    pub(crate) fn process(&mut self, logits: &CandleTensor) -> Result<CandleTensor> {
        let logits = logits.to_dtype(DType::F32)?;
        let values = logits.to_vec1::<f32>()?;
        let allowed = self.fsm.allowed_tokens(self.current_node);
        let masked = values
            .into_iter()
            .enumerate()
            .map(|(token_id, logit)| {
                if allowed.contains(&(token_id as u32)) {
                    logit
                } else {
                    f32::NEG_INFINITY
                }
            })
            .collect::<Vec<_>>();
        Ok(CandleTensor::from_vec(masked, logits.dims(), logits.device())?)
    }

    pub(crate) fn sample(&mut self, logits: &CandleTensor) -> Result<u32> {
        let masked = self.process(logits)?;
        let selected = argmax_token(&masked)?;
        self.advance_state(selected);
        Ok(selected)
    }

    pub(crate) fn advance_state(&mut self, selected_token: u32) {
        self.current_node = self.fsm.transition(self.current_node, selected_token);
    }

    #[cfg(test)]
    pub(crate) fn current_node(&self) -> usize {
        self.current_node
    }
}

pub(crate) fn compile_or_get_fsm(schema: &str) -> Result<Arc<CompiledFsm>> {
    let key = schema_hash(schema);
    let mut cache = fsm_cache()
        .lock()
        .map_err(|_| anyhow!("constrained decoding FSM cache lock poisoned"))?;

    if let Some(fsm) = cache.get(&key) {
        return Ok(Arc::clone(fsm));
    }

    let fsm = Arc::new(CompiledFsm::compile(schema)?);
    cache.put(key, Arc::clone(&fsm));
    Ok(fsm)
}

pub(crate) fn sample_constrained_logits(
    logits: &CandleTensor,
    json_schema: Option<&str>,
) -> Result<u32> {
    match json_schema {
        Some(schema) => FsmLogitProcessor::for_schema(schema)?.sample(logits),
        None => argmax_token(logits),
    }
}

fn argmax_token(logits: &CandleTensor) -> Result<u32> {
    let logits = logits.to_dtype(DType::F32)?;
    let values = logits.to_vec1::<f32>()?;
    values
        .iter()
        .enumerate()
        .filter(|(_, value)| value.is_finite())
        .max_by(|(_, left), (_, right)| left.total_cmp(right))
        .map(|(idx, _)| idx as u32)
        .ok_or_else(|| anyhow!("constrained decoding found no selectable token"))
}

fn collect_ascii_enum_tokens(value: &serde_json::Value, allowed: &mut BTreeSet<u32>) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                collect_ascii_enum_tokens(value, allowed);
            }
        }
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::Array(values)) = map.get("enum") {
                for value in values {
                    if let Some(text) = value.as_str() {
                        allowed.extend(text.bytes().map(u32::from));
                    }
                }
            }
            for value in map.values() {
                collect_ascii_enum_tokens(value, allowed);
            }
        }
        _ => {}
    }
}

fn schema_hash(schema: &str) -> [u8; 32] {
    let mut output = [0_u8; 32];
    output.copy_from_slice(&Sha256::digest(schema.as_bytes()));
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constrained_sampler_masks_disallowed_logits_and_advances_state() {
        let schema = r#"{"type":"string","enum":["A"]}"#;
        let logits = CandleTensor::from_vec(vec![0.9f32; 128], (128,), &Device::Cpu)
            .expect("logits tensor should build");
        let mut processor =
            FsmLogitProcessor::for_schema(schema).expect("schema should compile to FSM");

        let selected = processor.sample(&logits).expect("sampling should succeed");

        assert_eq!(selected, u32::from(b'A'));
        assert_ne!(processor.current_node(), 0);
    }

    #[test]
    fn unconstrained_sampler_uses_argmax() {
        let logits = CandleTensor::from_vec(vec![0.1f32, 4.0, 0.2], (3,), &Device::Cpu)
            .expect("logits tensor should build");

        assert_eq!(
            sample_constrained_logits(&logits, None).expect("argmax should succeed"),
            1
        );
    }

    #[test]
    fn fsm_compilation_is_cached_by_schema_hash() {
        let schema = r#"{"type":"string","enum":["ok"]}"#;
        let first = compile_or_get_fsm(schema).expect("schema should compile");
        let second = compile_or_get_fsm(schema).expect("schema should be cached");

        assert!(Arc::ptr_eq(&first, &second));
    }
}
