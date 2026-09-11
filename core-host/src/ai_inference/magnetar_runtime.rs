use anyhow::{anyhow, bail, Context, Result};
use magnetar_loader_huggingface::HuggingFaceIngestor;
use magnetar_runtime::model::{ModelTrustStatus, ModelTrustStore};
use magnetar_runtime::production_model_ingestion::{
    ProductionArtifactPayloadSource, ProductionModelArtifactIngestor, ProductionModelSource,
};
use magnetar_runtime::tokenizer::Tokenizer;
use magnetar_runtime::{ModelArtifactSource, Provider};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use super::{StreamControl, TokenUsage};

pub(crate) const MAGNETAR_PATH_PREFIX: &str = "magnetar:";
const TACHYON_MODEL_TRUST_JSON: &str = "tachyon-model-trust.json";
const TACHYON_HIDDEN_MODEL_TRUST_JSON: &str = ".tachyon-model-trust.json";

const QWEN_COMPONENT_BYTES: &[u8] = include_bytes!(
    "../../../vendor/Magnetar/magnetar-runtime/fixtures/components/qwen-real.component.wasm"
);
const QWEN_COMPONENT_MANIFEST_BYTES: &[u8] = include_bytes!(
    "../../../vendor/Magnetar/magnetar-runtime/fixtures/components/qwen-real.component.wasm.magnetar-component.yaml"
);

static QWEN_COMPONENT_REGISTERED: OnceLock<()> = OnceLock::new();

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CapabilityAdvertisement {
    pub(crate) provider_name: String,
    pub(crate) provider_version: String,
    pub(crate) device_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MagnetarProviderTarget {
    ReferenceCpu,
    Cuda,
}

#[derive(Clone)]
pub(crate) struct MagnetarRuntime {
    alias: String,
    root: PathBuf,
    fixture: magnetar_runtime::E2eFixture,
    payload_source: Arc<dyn ProductionArtifactPayloadSource>,
    trust_store: ModelTrustStore,
    provider: CapabilityAdvertisement,
    target: MagnetarProviderTarget,
}

impl std::fmt::Debug for MagnetarRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MagnetarRuntime")
            .field("alias", &self.alias)
            .field("root", &self.root)
            .field("provider", &self.provider)
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

impl MagnetarRuntime {
    pub(crate) fn try_load(
        alias: &str,
        path: &str,
        requested_target: &str,
    ) -> Result<Option<Self>> {
        if !is_magnetar_path(path) {
            return Ok(None);
        }
        register_qwen_component();

        let root = magnetar_root(path);
        let target = provider_target(requested_target)?;
        let source = ProductionModelSource::authorized_local_bundle(
            ModelArtifactSource::Tachyon(format!("tachyon:{alias}")),
            root.clone(),
        );
        let ingested = HuggingFaceIngestor::new()
            .ingest(&source)
            .with_context(|| {
                format!(
                    "Magnetar failed to ingest Qwen bundle at `{}`",
                    root.display()
                )
            })?;

        let tokenizer_path = root.join("tokenizer.json");
        let tokenizer_bytes = std::fs::read(&tokenizer_path)
            .with_context(|| format!("failed to read `{}`", tokenizer_path.display()))?;
        let real_tokenizer = magnetar_loader_huggingface::HuggingFaceTokenizer::from_bytes(
            &tokenizer_bytes,
            None,
            format!("{alias}-tokenizer"),
            ingested
                .manifest
                .architecture_config
                .as_ref()
                .map(|config| config.vocab_size),
        )
        .with_context(|| format!("Magnetar failed to load tokenizer for `{alias}`"))?;
        let tokenizer_metadata = real_tokenizer.metadata().clone();
        let real_tokenizer: Arc<dyn Tokenizer + Send + Sync> = Arc::new(real_tokenizer);

        let trust_store = tachyon_model_trust_store(&root)?;
        let trust_decision = trust_store.evaluate(&ingested.manifest);
        if trust_decision.status() != ModelTrustStatus::Trusted {
            bail!(
                "Magnetar model artifact trust rejected for `{alias}`: {}",
                trust_decision.reason()
            );
        }
        let fixture = magnetar_runtime::production_qwen_fixture(
            ingested.manifest.clone(),
            tokenizer_metadata,
            real_tokenizer,
        )
        .with_context(|| {
            format!("Magnetar failed to build production Qwen fixture for `{alias}`")
        })?;
        let provider = capability_advertisement(target)?;

        Ok(Some(Self {
            alias: alias.to_owned(),
            root,
            fixture,
            payload_source: Arc::clone(&ingested.payload_source),
            trust_store,
            provider,
            target,
        }))
    }

    pub(crate) fn executed_on(&self) -> &str {
        self.provider.provider_name.as_str()
    }

    pub(crate) fn provider(&self) -> &CapabilityAdvertisement {
        &self.provider
    }

    pub(crate) fn generate(&self, prompts: &[&[u8]]) -> Result<Vec<(Vec<u8>, TokenUsage)>> {
        if prompts.len() != 1 {
            bail!(
                "Magnetar Qwen generation for `{}` expects exactly one prompt, got {}",
                self.alias,
                prompts.len()
            );
        }
        let request = GenerationRequestView::parse(prompts[0])?;
        let mut fixture = self.fixture.clone();
        if let Some(max_new_tokens) = request.max_new_tokens {
            let defaults = fixture
                .manifest
                .generation
                .get_or_insert_with(Default::default);
            defaults.max_tokens = Some(max_new_tokens);
        }

        let outcome = match self.target {
            MagnetarProviderTarget::ReferenceCpu => magnetar_runtime::run_production_qwen_generation(
                fixture,
                self.payload_source.as_ref(),
                self.trust_store.clone(),
                request.prompt.as_str(),
            ),
            MagnetarProviderTarget::Cuda => {
                let requested = requested_max_tokens(&fixture, request.max_new_tokens);
                if requested != Some(1) {
                    bail!(
                        "Magnetar CUDA generation for `{}` supports only prefill / first-token execution until device-resident multi-step decode is available; request max_new_tokens=1 or route to cpu",
                        self.alias
                    );
                }
                #[cfg(feature = "magnetar-cuda")]
                {
                    let provider = magnetar_provider_cuda::CudaProvider::new();
                    if !provider.is_available() {
                        bail!(
                            "Magnetar CUDA provider is unavailable for `{}`; explicit CUDA placement cannot fall back to CPU",
                            self.alias
                        );
                    }
                    magnetar_runtime::run_production_qwen_generation_for_provider(
                        fixture,
                        self.payload_source.as_ref(),
                        self.trust_store.clone(),
                        request.prompt.as_str(),
                        Arc::new(provider),
                    )
                }
                #[cfg(not(feature = "magnetar-cuda"))]
                {
                    bail!(
                        "Magnetar CUDA provider is not compiled into this host; rebuild with `magnetar-cuda` for explicit CUDA placement"
                    )
                }
            }
        }
        .map_err(|error| anyhow!("Magnetar Qwen generation for `{}` failed: {error}", self.alias))?;

        let usage = outcome.result.output.usage;
        Ok(vec![(
            outcome.text.into_bytes(),
            TokenUsage {
                prompt_tokens: usage.prompt_tokens.min(u32::MAX as usize) as u32,
                completion_tokens: usage.generated_tokens.min(u32::MAX as usize) as u32,
            },
        )])
    }

    pub(crate) fn generate_streaming(
        &self,
        prompts: &[&[u8]],
        on_token: &mut dyn FnMut(&str) -> StreamControl,
    ) -> Result<TokenUsage> {
        let mut outputs = self.generate(prompts)?;
        let (bytes, usage) = outputs.remove(0);
        let text = String::from_utf8(bytes).context("Magnetar output was not UTF-8")?;
        if !text.is_empty() {
            on_token(&text);
        }
        Ok(usage)
    }
}

pub(crate) fn is_magnetar_path(path: &str) -> bool {
    path.trim().starts_with(MAGNETAR_PATH_PREFIX)
}

fn register_qwen_component() {
    QWEN_COMPONENT_REGISTERED.get_or_init(|| {
        magnetar_runtime::register_qwen_component_artifact(
            QWEN_COMPONENT_BYTES.to_vec(),
            QWEN_COMPONENT_MANIFEST_BYTES.to_vec(),
        );
    });
}

fn magnetar_root(path: &str) -> PathBuf {
    let trimmed = path.trim();
    if let Some(rest) = trimmed.strip_prefix(MAGNETAR_PATH_PREFIX) {
        PathBuf::from(rest)
    } else {
        PathBuf::from(trimmed)
    }
}

fn tachyon_model_trust_store(root: &std::path::Path) -> Result<ModelTrustStore> {
    let trust_path = [TACHYON_HIDDEN_MODEL_TRUST_JSON, TACHYON_MODEL_TRUST_JSON]
        .into_iter()
        .map(|file| root.join(file))
        .find(|path| path.is_file());
    let Some(trust_path) = trust_path else {
        return Ok(ModelTrustStore::default());
    };
    let value = serde_json::from_slice::<Value>(
        &std::fs::read(&trust_path)
            .with_context(|| format!("failed to read `{}`", trust_path.display()))?,
    )
    .with_context(|| format!("failed to parse `{}`", trust_path.display()))?;
    let trusted_digests = value
        .get("trusted_digests")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            anyhow!(
                "`{}` must contain a `trusted_digests` array",
                trust_path.display()
            )
        })?;
    let mut trust_store = ModelTrustStore::default();
    for digest in trusted_digests {
        let digest = digest.as_str().ok_or_else(|| {
            anyhow!(
                "`{}` trusted_digests entries must be strings",
                trust_path.display()
            )
        })?;
        trust_store = trust_store.trust_digest(digest);
    }
    Ok(trust_store)
}

fn provider_target(requested_target: &str) -> Result<MagnetarProviderTarget> {
    match requested_target.trim().to_ascii_lowercase().as_str() {
        "" | "cpu" => Ok(MagnetarProviderTarget::ReferenceCpu),
        "cuda" | "gpu" => Ok(MagnetarProviderTarget::Cuda),
        other => bail!("Magnetar production Qwen supports cpu and cuda placements, got `{other}`"),
    }
}

fn capability_advertisement(target: MagnetarProviderTarget) -> Result<CapabilityAdvertisement> {
    match target {
        MagnetarProviderTarget::ReferenceCpu => {
            let provider = magnetar_runtime::ReferenceCpuProvider::new();
            Ok(provider_advertisement(&provider))
        }
        MagnetarProviderTarget::Cuda => {
            #[cfg(feature = "magnetar-cuda")]
            {
                let provider = magnetar_provider_cuda::CudaProvider::new();
                if !provider.is_available() {
                    bail!(
                        "Magnetar CUDA provider is unavailable; explicit CUDA placement cannot fall back to CPU"
                    );
                }
                Ok(provider_advertisement(&provider))
            }
            #[cfg(not(feature = "magnetar-cuda"))]
            {
                bail!(
                    "Magnetar CUDA provider is not compiled into this host; rebuild with `magnetar-cuda` for explicit CUDA placement"
                )
            }
        }
    }
}

fn provider_advertisement(provider: &dyn Provider) -> CapabilityAdvertisement {
    let metadata = provider.metadata();
    let device_ids = provider
        .devices()
        .iter()
        .map(|device| device.metadata().id.as_str().to_owned())
        .collect();
    CapabilityAdvertisement {
        provider_name: metadata.name,
        provider_version: metadata.version,
        device_ids,
    }
}

fn requested_max_tokens(
    fixture: &magnetar_runtime::E2eFixture,
    request_max_new_tokens: Option<u32>,
) -> Option<u32> {
    request_max_new_tokens.or_else(|| {
        fixture
            .manifest
            .generation
            .as_ref()
            .and_then(|generation| generation.max_tokens)
    })
}

struct GenerationRequestView {
    prompt: String,
    max_new_tokens: Option<u32>,
}

impl GenerationRequestView {
    fn parse(prompt: &[u8]) -> Result<Self> {
        let text = std::str::from_utf8(prompt)
            .map_err(|error| anyhow!("Qwen prompt must be UTF-8: {error}"))?;
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            return Ok(Self {
                prompt: text.to_owned(),
                max_new_tokens: None,
            });
        };
        let Value::Object(object) = value else {
            return Ok(Self {
                prompt: text.to_owned(),
                max_new_tokens: None,
            });
        };
        let prompt = object
            .get("prompt")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| text.to_owned());
        let max_new_tokens = object
            .get("max_new_tokens")
            .or_else(|| object.get("max_tokens"))
            .and_then(Value::as_u64)
            .map(|value| value.min(u64::from(u32::MAX)) as u32);
        Ok(Self {
            prompt,
            max_new_tokens,
        })
    }
}
