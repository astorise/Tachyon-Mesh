use anyhow::{anyhow, bail, Context, Result};
use magnetar_loader_huggingface::{
    parse_tokenizer_config, HuggingFaceChatTemplateFormatter, HuggingFaceIngestor,
};
use magnetar_runtime::model::{ModelTrustStatus, ModelTrustStore};
use magnetar_runtime::production_model_ingestion::{
    ProductionModelArtifactIngestor, ProductionModelSource,
};
use magnetar_runtime::tokenizer::Tokenizer;
use magnetar_runtime::{
    ChatMessage, GenerationParameters, GenerationStreamEvent, ModelArtifactSource,
    ProductionGenerationRequest, PromptInput, Provider, StopConditions,
};
use serde_json::Value;
use std::{
    fmt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use super::{StreamControl, TokenUsage};

pub(crate) const MAGNETAR_PATH_PREFIX: &str = "magnetar:";
pub(crate) const TACHYON_MODEL_TRUST_STORE_ENV: &str = "TACHYON_MODEL_TRUST_STORE";

const QWEN_COMPONENT_BYTES: &[u8] = include_bytes!(
    "../../../vendor/Magnetar/magnetar-runtime/fixtures/components/qwen-real.component.wasm"
);
const QWEN_COMPONENT_MANIFEST_BYTES: &[u8] = include_bytes!(
    "../../../vendor/Magnetar/magnetar-runtime/fixtures/components/qwen-real.component.wasm.magnetar-component.yaml"
);

static QWEN_COMPONENT_REGISTERED: OnceLock<()> = OnceLock::new();

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProviderAdvertisement {
    pub(crate) provider_name: String,
    pub(crate) provider_version: String,
    pub(crate) device_ids: Vec<String>,
    pub(crate) device_class: ProviderDeviceClass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProviderDeviceClass {
    ReferenceCpu,
    Cuda,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MagnetarProviderTarget {
    ReferenceCpu,
    Cuda,
}

pub(crate) struct MagnetarRuntime {
    alias: String,
    root: PathBuf,
    chat_formatter: Option<Arc<HuggingFaceChatTemplateFormatter>>,
    provider: ProviderAdvertisement,
    loaded_model: Mutex<magnetar_runtime::ProductionQwenLoadedModel>,
}

impl std::fmt::Debug for MagnetarRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MagnetarRuntime")
            .field("alias", &self.alias)
            .field("root", &self.root)
            .field("provider", &self.provider)
            .field("target", &self.provider.device_class)
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
        let tokenizer_config_path = root.join("tokenizer_config.json");
        let tokenizer_config_bytes =
            if tokenizer_config_path.is_file() {
                Some(std::fs::read(&tokenizer_config_path).with_context(|| {
                    format!("failed to read `{}`", tokenizer_config_path.display())
                })?)
            } else {
                None
            };
        let tokenizer_config_metadata = tokenizer_config_bytes
            .as_deref()
            .map(parse_tokenizer_config)
            .transpose()
            .with_context(|| {
                format!("Magnetar failed to parse tokenizer_config.json for `{alias}`")
            })?;
        let chat_formatter = tokenizer_config_metadata
            .as_ref()
            .and_then(|metadata| metadata.chat_template_reference.as_deref())
            .map(HuggingFaceChatTemplateFormatter::new)
            .transpose()
            .with_context(|| format!("Magnetar failed to load chat template for `{alias}`"))?
            .map(Arc::new);
        let real_tokenizer = magnetar_loader_huggingface::HuggingFaceTokenizer::from_bytes(
            &tokenizer_bytes,
            tokenizer_config_metadata.as_ref(),
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
        let provider_for_generation = provider_for_target(target, alias)?;
        let loaded_model = magnetar_runtime::ProductionQwenLoadedModel::load(
            fixture.clone(),
            ingested.payload_source.as_ref(),
            trust_store.clone(),
            provider_for_generation,
        )
        .with_context(|| {
            format!("Magnetar failed to materialize resident Qwen model for `{alias}`")
        })?;

        Ok(Some(Self {
            alias: alias.to_owned(),
            root,
            chat_formatter,
            provider,
            loaded_model: Mutex::new(loaded_model),
        }))
    }

    pub(crate) fn executed_on(&self) -> &str {
        self.provider.provider_name.as_str()
    }

    pub(crate) fn provider(&self) -> &ProviderAdvertisement {
        &self.provider
    }

    #[cfg(test)]
    pub(crate) fn resident_debug(&self) -> Result<(String, usize)> {
        let loaded = self
            .loaded_model
            .lock()
            .map_err(|_| anyhow!("resident Magnetar model lock poisoned for `{}`", self.alias))?;
        Ok((
            loaded.model_instance_id().to_string(),
            loaded.materialization_count(),
        ))
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
        let request = request.into_magnetar_request()?;

        let outcome = self
            .loaded_model
            .lock()
            .map_err(|_| anyhow!("resident Magnetar model lock poisoned for `{}`", self.alias))?
            .generate(request, self.chat_formatter())
            .map_err(|error| {
                anyhow!(
                    "Magnetar Qwen generation for `{}` failed: {error}",
                    self.alias
                )
            })?;

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
        if prompts.len() != 1 {
            bail!(
                "Magnetar Qwen streaming generation for `{}` expects exactly one prompt, got {}",
                self.alias,
                prompts.len()
            );
        }
        let request = GenerationRequestView::parse(prompts[0])?.into_magnetar_request()?;
        let mut streamed_usage = None;
        let mut on_event = |event: GenerationStreamEvent| -> std::ops::ControlFlow<()> {
            match event {
                GenerationStreamEvent::Token {
                    text_delta: Some(delta),
                    ..
                } if !delta.is_empty() => {
                    if on_token(&delta).is_stop() {
                        std::ops::ControlFlow::Break(())
                    } else {
                        std::ops::ControlFlow::Continue(())
                    }
                }
                GenerationStreamEvent::Finished { usage, .. } => {
                    streamed_usage = Some(TokenUsage {
                        prompt_tokens: usage.prompt_tokens.min(u32::MAX as usize) as u32,
                        completion_tokens: usage.generated_tokens.min(u32::MAX as usize) as u32,
                    });
                    std::ops::ControlFlow::Continue(())
                }
                _ => std::ops::ControlFlow::Continue(()),
            }
        };
        let outcome = self
            .loaded_model
            .lock()
            .map_err(|_| anyhow!("resident Magnetar model lock poisoned for `{}`", self.alias))?
            .generate_streaming(request, self.chat_formatter(), &mut on_event)
            .map_err(|error| {
                anyhow!(
                    "Magnetar Qwen streaming generation for `{}` failed: {error}",
                    self.alias
                )
            })?;
        Ok(streamed_usage.unwrap_or_else(|| {
            let usage = outcome.result.output.usage;
            TokenUsage {
                prompt_tokens: usage.prompt_tokens.min(u32::MAX as usize) as u32,
                completion_tokens: usage.generated_tokens.min(u32::MAX as usize) as u32,
            }
        }))
    }

    fn chat_formatter(&self) -> Option<&dyn magnetar_runtime::ChatTemplateFormatter> {
        self.chat_formatter
            .as_deref()
            .map(|formatter| formatter as &dyn magnetar_runtime::ChatTemplateFormatter)
    }
}

#[derive(Debug)]
pub(crate) struct InvalidGenerationRequest(String);

impl fmt::Display for InvalidGenerationRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InvalidGenerationRequest {}

pub(crate) fn is_invalid_generation_request(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.is::<InvalidGenerationRequest>())
}

fn invalid_request(message: impl Into<String>) -> anyhow::Error {
    anyhow!(InvalidGenerationRequest(message.into()))
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

fn tachyon_model_trust_store(root: &Path) -> Result<ModelTrustStore> {
    let Some(trust_path) = std::env::var_os(TACHYON_MODEL_TRUST_STORE_ENV).map(PathBuf::from)
    else {
        return Ok(ModelTrustStore::default());
    };
    reject_trust_store_inside_artifact(root, &trust_path)?;
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

fn reject_trust_store_inside_artifact(root: &Path, trust_path: &Path) -> Result<()> {
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize model root `{}`", root.display()))?;
    let canonical_trust_path = trust_path.canonicalize().with_context(|| {
        format!(
            "failed to canonicalize Tachyon trust store `{}`",
            trust_path.display()
        )
    })?;
    if canonical_trust_path.starts_with(&canonical_root) {
        bail!(
            "Tachyon model trust store `{}` must be controlled outside artifact root `{}`",
            canonical_trust_path.display(),
            canonical_root.display()
        );
    }
    Ok(())
}

fn provider_target(requested_target: &str) -> Result<MagnetarProviderTarget> {
    match requested_target.trim().to_ascii_lowercase().as_str() {
        "" | "cpu" => Ok(MagnetarProviderTarget::ReferenceCpu),
        "cuda" | "gpu" => Ok(MagnetarProviderTarget::Cuda),
        other => bail!("Magnetar production Qwen supports cpu and cuda placements, got `{other}`"),
    }
}

fn capability_advertisement(target: MagnetarProviderTarget) -> Result<ProviderAdvertisement> {
    match target {
        MagnetarProviderTarget::ReferenceCpu => {
            let provider = magnetar_runtime::ReferenceCpuProvider::new();
            Ok(provider_advertisement(
                &provider,
                ProviderDeviceClass::ReferenceCpu,
            ))
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
                Ok(provider_advertisement(&provider, ProviderDeviceClass::Cuda))
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

fn provider_for_target(target: MagnetarProviderTarget, alias: &str) -> Result<Arc<dyn Provider>> {
    match target {
        MagnetarProviderTarget::ReferenceCpu => {
            Ok(Arc::new(magnetar_runtime::ReferenceCpuProvider::new()) as Arc<dyn Provider>)
        }
        MagnetarProviderTarget::Cuda => {
            #[cfg(feature = "magnetar-cuda")]
            {
                let provider = magnetar_provider_cuda::CudaProvider::new();
                if !provider.is_available() {
                    bail!(
                        "Magnetar CUDA provider is unavailable for `{alias}`; explicit CUDA placement cannot fall back to CPU"
                    );
                }
                Ok(Arc::new(provider) as Arc<dyn Provider>)
            }
            #[cfg(not(feature = "magnetar-cuda"))]
            {
                let _ = alias;
                bail!(
                    "Magnetar CUDA provider is not compiled into this host; rebuild with `magnetar-cuda` for explicit CUDA placement"
                )
            }
        }
    }
}

fn provider_advertisement(
    provider: &dyn Provider,
    device_class: ProviderDeviceClass,
) -> ProviderAdvertisement {
    let metadata = provider.metadata();
    let device_ids = provider
        .devices()
        .iter()
        .map(|device| device.metadata().id.as_str().to_owned())
        .collect();
    ProviderAdvertisement {
        provider_name: metadata.name,
        provider_version: metadata.version,
        device_ids,
        device_class,
    }
}

#[derive(Debug)]
struct GenerationRequestView {
    prompt: PromptInput,
    parameters: GenerationParameters,
    stop_conditions: StopConditions,
    max_new_tokens: Option<usize>,
    max_generation_millis: Option<u64>,
}

impl GenerationRequestView {
    fn parse(prompt: &[u8]) -> Result<Self> {
        let text = std::str::from_utf8(prompt)
            .map_err(|error| anyhow!("Qwen prompt must be UTF-8: {error}"))?;
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            return Ok(Self {
                prompt: PromptInput::PlainText(text.to_owned()),
                parameters: GenerationParameters::greedy(),
                stop_conditions: StopConditions::default(),
                max_new_tokens: None,
                max_generation_millis: None,
            });
        };
        let Value::Object(object) = value else {
            return Ok(Self {
                prompt: PromptInput::PlainText(text.to_owned()),
                parameters: GenerationParameters::greedy(),
                stop_conditions: StopConditions::default(),
                max_new_tokens: None,
                max_generation_millis: None,
            });
        };
        fail_on_unsupported_generation_fields(&object)?;
        let prompt = if let Some(messages) = object.get("messages") {
            PromptInput::ChatMessages(parse_chat_messages(messages)?)
        } else {
            let prompt = object
                .get("prompt")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| text.to_owned());
            PromptInput::PlainText(prompt)
        };
        let prompt = apply_host_generation_controls(prompt, &object)?;
        let max_new_tokens = object
            .get("max_new_tokens")
            .or_else(|| object.get("max_tokens"))
            .and_then(Value::as_u64)
            .map(|value| value.min(usize::MAX as u64) as usize);
        let mut parameters = GenerationParameters::greedy();
        if let Some(temperature) = json_f32(&object, "temperature")? {
            if temperature == 0.0 {
                parameters = GenerationParameters::greedy();
            } else {
                parameters.temperature = temperature;
                parameters.greedy = false;
                parameters.sampling_enabled = true;
            }
        }
        if let Some(top_p) = json_f32(&object, "top_p")? {
            parameters.top_p = Some(top_p);
        }
        if let Some(top_k) = object
            .get("top_k")
            .and_then(Value::as_u64)
            .map(|value| value.min(u64::from(u32::MAX)) as u32)
        {
            parameters.top_k = Some(top_k);
        }
        if let Some(seed) = object.get("seed").and_then(Value::as_u64) {
            parameters.seed = Some(seed);
            parameters.deterministic = true;
        }
        parameters.frequency_penalty = json_f32(&object, "frequency_penalty")?;
        parameters.presence_penalty = json_f32(&object, "presence_penalty")?;
        parameters.repetition_penalty = json_f32(&object, "repetition_penalty")?;

        let mut stop_conditions = StopConditions::default();
        if let Some(stop) = object.get("stop") {
            stop_conditions.stop_text_sequences = parse_stop_sequences(stop)?;
        }
        Ok(Self {
            prompt,
            parameters,
            stop_conditions,
            max_new_tokens,
            max_generation_millis: parse_max_generation_millis(&object)?,
        })
    }

    fn into_magnetar_request(self) -> Result<ProductionGenerationRequest> {
        self.parameters
            .validate()
            .map_err(|error| anyhow!("invalid Magnetar generation parameters: {error}"))?;
        Ok(ProductionGenerationRequest {
            prompt: self.prompt,
            parameters: self.parameters,
            stop_conditions: self.stop_conditions,
            max_new_tokens: self.max_new_tokens,
            max_generation_millis: self.max_generation_millis,
        })
    }
}

fn fail_on_unsupported_generation_fields(object: &serde_json::Map<String, Value>) -> Result<()> {
    const SUPPORTED: &[&str] = &[
        "prompt",
        "messages",
        "max_new_tokens",
        "max_tokens",
        "temperature",
        "top_p",
        "top_k",
        "seed",
        "stop",
        "frequency_penalty",
        "presence_penalty",
        "repetition_penalty",
        "include_usage",
        "tools",
        "tool_choice",
        "tool_call_parser",
        "max_generation_ms",
        "json_schema",
    ];
    if let Some(key) = object.keys().find(|key| !SUPPORTED.contains(&key.as_str())) {
        return Err(invalid_request(format!(
            "Magnetar generation request field `{key}` is not supported by Tachyon"
        )));
    }
    Ok(())
}

fn apply_host_generation_controls(
    prompt: PromptInput,
    object: &serde_json::Map<String, Value>,
) -> Result<PromptInput> {
    let mut instructions = Vec::new();
    if let Some(schema) = object.get("json_schema") {
        instructions.push(structured_output_instruction(schema)?);
    }
    let tool_choice = object.get("tool_choice");
    if tool_choice.is_some_and(|choice| choice.as_str() == Some("none")) {
        return prepend_system_instructions(prompt, instructions);
    }
    if let Some(tools) = object.get("tools").filter(|tools| !is_empty_json(tools)) {
        instructions.push(tool_instruction(tools, tool_choice)?);
    } else if let Some(choice) = tool_choice {
        instructions.push(format!(
            "Tool choice requested without a tool list: {}.",
            serde_json::to_string(choice).unwrap_or_else(|_| "null".to_owned())
        ));
    }
    if object.get("tool_call_parser").is_some() {
        // Host-only parser selection. guest-openai consumes the generated text
        // after generation; Magnetar only needs the tool instructions above.
    }
    prepend_system_instructions(prompt, instructions)
}

fn structured_output_instruction(schema: &Value) -> Result<String> {
    let schema_text = schema
        .as_str()
        .ok_or_else(|| invalid_request("`json_schema` must be a JSON schema string"))?;
    let schema_value = serde_json::from_str::<Value>(schema_text)
        .map_err(|error| invalid_request(format!("invalid `json_schema`: {error}")))?;
    if schema_value == serde_json::json!({"type":"object"}) {
        Ok("Respond with a single valid JSON object and no surrounding prose.".to_owned())
    } else {
        Err(invalid_request(
            "`response_format: json_schema` is not supported by local Magnetar generation; use `json_object` or route to an OpenAI-compatible upstream",
        ))
    }
}

fn tool_instruction(tools: &Value, tool_choice: Option<&Value>) -> Result<String> {
    let tools = tools
        .as_array()
        .ok_or_else(|| invalid_request("`tools` must be an array"))?;
    let mut lines = vec![
        "Tools are available. When calling a tool, respond only with a JSON object in this shape: {\"tool_calls\":[{\"name\":\"function_name\",\"arguments\":{}}]}.".to_owned(),
        "Available tools:".to_owned(),
    ];
    for tool in tools {
        let name = tool
            .get("function")
            .and_then(|function| function.get("name"))
            .and_then(Value::as_str)
            .ok_or_else(|| invalid_request("each tool must contain `function.name`"))?;
        let description = tool
            .get("function")
            .and_then(|function| function.get("description"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let parameters = tool
            .get("function")
            .and_then(|function| function.get("parameters"))
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        lines.push(format!(
            "- {name}: {description} parameters={}",
            serde_json::to_string(&parameters).unwrap_or_else(|_| "{}".to_owned())
        ));
    }
    if let Some(choice) = tool_choice {
        lines.push(format!(
            "Tool choice: {}.",
            serde_json::to_string(choice).unwrap_or_else(|_| "null".to_owned())
        ));
    }
    Ok(lines.join("\n"))
}

fn prepend_system_instructions(
    prompt: PromptInput,
    instructions: Vec<String>,
) -> Result<PromptInput> {
    if instructions.is_empty() {
        return Ok(prompt);
    }
    let instruction = instructions.join("\n\n");
    Ok(match prompt {
        PromptInput::ChatMessages(mut messages) => {
            messages.insert(0, ChatMessage::new("system", instruction));
            PromptInput::ChatMessages(messages)
        }
        PromptInput::PlainText(text) => PromptInput::PlainText(format!("{instruction}\n\n{text}")),
        PromptInput::TokenIds(_) | PromptInput::TestTokenSequence(_) => {
            return Err(invalid_request(
                "host generation controls require text or chat message input",
            ));
        }
    })
}

fn parse_max_generation_millis(object: &serde_json::Map<String, Value>) -> Result<Option<u64>> {
    let Some(value) = object.get("max_generation_ms") else {
        return Ok(None);
    };
    let millis = value
        .as_u64()
        .ok_or_else(|| invalid_request("`max_generation_ms` must be an integer"))?;
    if millis == 0 {
        return Err(invalid_request(
            "`max_generation_ms` must be greater than zero",
        ));
    }
    Ok(Some(millis))
}

fn is_empty_json(value: &Value) -> bool {
    matches!(value, Value::Null)
        || value.as_array().is_some_and(Vec::is_empty)
        || value.as_object().is_some_and(serde_json::Map::is_empty)
}

fn parse_chat_messages(value: &Value) -> Result<Vec<ChatMessage>> {
    let messages = value
        .as_array()
        .ok_or_else(|| anyhow!("`messages` must be an array"))?;
    messages
        .iter()
        .map(|message| {
            let object = message
                .as_object()
                .ok_or_else(|| anyhow!("chat message entries must be objects"))?;
            let role = object
                .get("role")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("chat message entries require string `role`"))?;
            let content = object
                .get("content")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("chat message entries require string `content`"))?;
            Ok(ChatMessage::new(role, content))
        })
        .collect()
}

fn parse_stop_sequences(value: &Value) -> Result<Vec<String>> {
    if let Some(stop) = value.as_str() {
        return Ok(vec![stop.to_owned()]);
    }
    let stops = value
        .as_array()
        .ok_or_else(|| anyhow!("`stop` must be a string or string array"))?;
    stops
        .iter()
        .map(|stop| {
            stop.as_str()
                .map(str::to_owned)
                .ok_or_else(|| anyhow!("`stop` array entries must be strings"))
        })
        .collect()
}

fn json_f32(object: &serde_json::Map<String, Value>, key: &str) -> Result<Option<f32>> {
    object
        .get(key)
        .map(|value| {
            value
                .as_f64()
                .ok_or_else(|| anyhow!("`{key}` must be a number"))
                .map(|value| value as f32)
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_request_maps_openai_chat_controls_to_magnetar_contracts() {
        let request = GenerationRequestView::parse(
            br#"{
                "messages":[{"role":"system","content":"brief"},{"role":"user","content":"hi"}],
                "temperature":0.7,
                "top_p":0.9,
                "top_k":12,
                "seed":42,
                "frequency_penalty":0.1,
                "presence_penalty":0.2,
                "repetition_penalty":1.1,
                "max_new_tokens":16,
                "stop":["<eos>"],
                "include_usage":true
            }"#,
        )
        .expect("OpenAI-shaped request should parse")
        .into_magnetar_request()
        .expect("supported controls should build a Magnetar request");

        match request.prompt {
            PromptInput::ChatMessages(messages) => {
                assert_eq!(messages.len(), 2);
                assert_eq!(messages[0].role, "system");
                assert_eq!(messages[1].content, "hi");
            }
            other => panic!("expected chat messages, got {other:?}"),
        }
        assert_eq!(request.max_new_tokens, Some(16));
        assert_eq!(request.parameters.temperature, 0.7);
        assert_eq!(request.parameters.top_p, Some(0.9));
        assert_eq!(request.parameters.top_k, Some(12));
        assert_eq!(request.parameters.seed, Some(42));
        assert!(request.parameters.deterministic);
        assert_eq!(request.parameters.frequency_penalty, Some(0.1));
        assert_eq!(request.parameters.presence_penalty, Some(0.2));
        assert_eq!(request.parameters.repetition_penalty, Some(1.1));
        assert_eq!(
            request.stop_conditions.stop_text_sequences,
            vec!["<eos>".to_owned()]
        );
    }

    #[test]
    fn generation_request_rejects_unsupported_local_fields() {
        let error = GenerationRequestView::parse(br#"{"prompt":"hi","unknown_local":true}"#)
            .expect_err("unsupported local fields must fail closed");

        assert!(
            error.to_string().contains("unknown_local"),
            "unexpected unsupported-field error: {error}"
        );
        assert!(is_invalid_generation_request(&error));
    }

    #[test]
    fn generation_request_preserves_tools_as_prompt_instructions() {
        let request = GenerationRequestView::parse(
            br#"{
                "messages":[{"role":"user","content":"weather?"}],
                "tools":[{
                    "type":"function",
                    "function":{
                        "name":"get_weather",
                        "description":"Fetch weather",
                        "parameters":{"type":"object","properties":{"city":{"type":"string"}}}
                    }
                }],
                "tool_choice":"auto",
                "tool_call_parser":"qwen"
            }"#,
        )
        .expect("tools are Tachyon host controls, not unsupported Magnetar fields")
        .into_magnetar_request()
        .expect("tools should map to prompt-visible instructions");

        let PromptInput::ChatMessages(messages) = request.prompt else {
            panic!("expected chat messages");
        };
        assert_eq!(messages[0].role, "system");
        assert!(messages[0].content.contains("get_weather"));
        assert!(messages[0].content.contains("Tool choice"));
        assert_eq!(messages[1].content, "weather?");
    }

    #[test]
    fn generation_request_maps_json_object_and_rejects_json_schema() {
        let request = GenerationRequestView::parse(
            br#"{"prompt":"hi","json_schema":"{\"type\":\"object\"}"}"#,
        )
        .expect("json_object schema should map to prompt instruction")
        .into_magnetar_request()
        .expect("json_object instruction should build request");
        let PromptInput::PlainText(prompt) = request.prompt else {
            panic!("expected plain prompt");
        };
        assert!(prompt.contains("valid JSON object"));

        let error = GenerationRequestView::parse(
            br#"{"prompt":"hi","json_schema":"{\"type\":\"object\",\"properties\":{\"a\":{\"type\":\"integer\"}}}"}"#,
        )
        .expect_err("local Magnetar path must reject unsupported constrained schema");
        assert!(is_invalid_generation_request(&error));
    }

    #[test]
    fn generation_request_carries_max_generation_deadline() {
        let request = GenerationRequestView::parse(br#"{"prompt":"hi","max_generation_ms":5000}"#)
            .expect("deadline should parse")
            .into_magnetar_request()
            .expect("deadline should build request");
        assert_eq!(request.max_generation_millis, Some(5000));

        let error = GenerationRequestView::parse(br#"{"prompt":"hi","max_generation_ms":0}"#)
            .expect_err("zero deadline should fail closed");
        assert!(is_invalid_generation_request(&error));
    }
}
