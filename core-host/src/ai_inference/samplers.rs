//! Native JSON-Schema-constrained decoding.
//!
//! Compiles a (deliberately scoped) subset of JSON Schema into a grammar
//! automaton, then masks disallowed token logits to `-inf` before sampling so
//! the model can only ever emit text the grammar accepts.
//!
//! ## Scope
//! The compiled grammar only covers:
//! - top-level `object` schemas with a flat `properties` map (no nested
//!   objects/arrays, no `$ref`/`oneOf`/`anyOf`) whose values are
//!   `string`/`number`/`integer`/`boolean` or a string `enum`;
//! - top-level scalar schemas (`string`/`number`/`integer`/`boolean`/`enum`).
//! - output is always compact JSON: no whitespace is permitted between
//!   structural tokens, and every declared property is emitted exactly once,
//!   in alphabetical key order (the project's `serde_json` does not enable
//!   `preserve_order`, so schema property order is not preserved past
//!   parsing; JSON Schema's `required` therefore has no effect on the
//!   compiled grammar — every declared property is, in effect, required by
//!   this pass, in alphabetical order).
//!
//! Anything outside that shape is rejected by [`compile_schema`] with a
//! descriptive error rather than silently producing an incorrect grammar.
//! Native FP4 kernels and a tokenizer-vocabulary-trie precomputation (to
//! avoid the current full-vocab-scan-per-step cost) are both out of scope for
//! this pass.

use std::sync::{Arc, Mutex};

use lru::LruCache;
use sha2::{Digest, Sha256};

/// A compiled, immutable grammar. Cheap to clone (`Arc`-shared) and safe to
/// reuse across concurrent requests for the same schema.
#[derive(Debug)]
pub(crate) struct CompiledFsm {
    root: GrammarRoot,
}

#[derive(Debug)]
enum GrammarRoot {
    Object(Vec<(String, SchemaNode)>),
    Scalar(SchemaNode),
}

#[derive(Debug, Clone)]
enum SchemaNode {
    String,
    Number,
    Boolean,
    /// A closed set of string-literal alternatives, pre-rendered to their
    /// exact JSON text (including the surrounding quotes).
    Enum(Vec<String>),
}

/// In-flight position within a [`CompiledFsm`]. Cheap to clone; advanced one
/// character at a time by [`CompiledFsm::step`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FsmState {
    ObjBeforeOpen,
    /// Matching the literal text of `properties[idx]`'s `"name":` prefix (or
    /// the closing `}` when `idx == properties.len()`); `matched` counts how
    /// many bytes of that literal have been consumed so far.
    ObjKey { idx: usize, matched: usize },
    ObjValue { idx: usize, value: ValueState },
    ObjAfterValue { idx: usize },
    ScalarValue(ValueState),
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ValueState {
    /// Before the value's opening `"` has been consumed.
    StringOpen,
    /// Inside the string body, after the opening `"`.
    String,
    Number {
        has_digit: bool,
    },
    /// Matching a fixed literal (`true`/`false`) or one alternative of an
    /// `enum`; `matched` counts bytes consumed so far.
    Literal { text: String, matched: usize },
}

enum StepOutcome {
    Consumed(ValueState),
    ConsumedComplete,
    /// The character was not part of the value (e.g. the byte after a
    /// number's last digit); the caller must reprocess it against the
    /// surrounding grammar.
    Terminate,
    Invalid,
}

fn start_value_state(node: &SchemaNode) -> ValueState {
    match node {
        SchemaNode::String => ValueState::StringOpen,
        SchemaNode::Number => ValueState::Number { has_digit: false },
        SchemaNode::Boolean => ValueState::Literal {
            text: "true".to_owned(),
            matched: 0,
        },
        SchemaNode::Enum(alternatives) => ValueState::Literal {
            text: alternatives.first().cloned().unwrap_or_default(),
            matched: 0,
        },
    }
}

/// `Boolean` and `Enum` nodes admit more than one literal spelling
/// (`true`/`false`, or any enum alternative). [`start_value_state`] only seeds
/// the first; this returns every alternative so the step function can try
/// them all from byte zero.
fn value_alternatives(node: &SchemaNode) -> Vec<String> {
    match node {
        SchemaNode::String | SchemaNode::Number => Vec::new(),
        SchemaNode::Boolean => vec!["true".to_owned(), "false".to_owned()],
        SchemaNode::Enum(alternatives) => alternatives.clone(),
    }
}

fn step_value(state: &ValueState, node: &SchemaNode, ch: char) -> StepOutcome {
    match state {
        ValueState::StringOpen => {
            if ch == '"' {
                StepOutcome::Consumed(ValueState::String)
            } else {
                StepOutcome::Invalid
            }
        }
        ValueState::String => {
            if ch == '"' {
                StepOutcome::ConsumedComplete
            } else if ch == '\\' || !ch.is_control() {
                StepOutcome::Consumed(ValueState::String)
            } else {
                StepOutcome::Invalid
            }
        }
        ValueState::Number { has_digit } => {
            if ch.is_ascii_digit()
                || ((ch == '-' || ch == '.' || ch == 'e' || ch == 'E' || ch == '+') && *has_digit)
            {
                StepOutcome::Consumed(ValueState::Number { has_digit: true })
            } else if ch == '-' && !*has_digit {
                StepOutcome::Consumed(ValueState::Number { has_digit: false })
            } else if *has_digit {
                StepOutcome::Terminate
            } else {
                StepOutcome::Invalid
            }
        }
        ValueState::Literal { matched, .. } => {
            // Try every alternative literal that still matches the bytes
            // consumed so far, given the next character.
            let mut candidates: Vec<String> = value_alternatives(node);
            if candidates.is_empty() {
                candidates.push(match node {
                    SchemaNode::Boolean => "true".to_owned(),
                    _ => String::new(),
                });
            }
            for candidate in candidates {
                let bytes: Vec<char> = candidate.chars().collect();
                if *matched < bytes.len() && bytes[*matched] == ch {
                    if matched + 1 == bytes.len() {
                        return StepOutcome::ConsumedComplete;
                    }
                    return StepOutcome::Consumed(ValueState::Literal {
                        text: candidate,
                        matched: matched + 1,
                    });
                }
            }
            StepOutcome::Invalid
        }
    }
}

fn key_literal(name: &str) -> String {
    format!("\"{name}\":")
}

impl CompiledFsm {
    pub(crate) fn initial_state(&self) -> FsmState {
        match &self.root {
            GrammarRoot::Object(_) => FsmState::ObjBeforeOpen,
            GrammarRoot::Scalar(node) => FsmState::ScalarValue(start_value_state(node)),
        }
    }

    /// Whether `state` is a valid place to stop generating (i.e. an EOS token
    /// would produce grammar-complete output).
    pub(crate) fn is_complete(&self, state: &FsmState) -> bool {
        match state {
            FsmState::Done => true,
            FsmState::ScalarValue(ValueState::Number { has_digit }) => *has_digit,
            _ => false,
        }
    }

    fn node_for_idx(&self, idx: usize) -> Option<&SchemaNode> {
        match &self.root {
            GrammarRoot::Object(properties) => properties.get(idx).map(|(_, node)| node),
            GrammarRoot::Scalar(_) => None,
        }
    }

    fn step(&self, state: &FsmState, ch: char) -> Option<FsmState> {
        let properties = match &self.root {
            GrammarRoot::Object(properties) => Some(properties),
            GrammarRoot::Scalar(_) => None,
        };
        match state {
            FsmState::ObjBeforeOpen => (ch == '{').then_some(FsmState::ObjKey {
                idx: 0,
                matched: 0,
            }),
            FsmState::ObjKey { idx, matched } => {
                let properties = properties?;
                let literal = if *idx == properties.len() {
                    "}".to_owned()
                } else {
                    key_literal(&properties[*idx].0)
                };
                let expected = literal.chars().nth(*matched)?;
                if ch != expected {
                    return None;
                }
                let next_matched = matched + 1;
                if next_matched == literal.chars().count() {
                    if *idx == properties.len() {
                        Some(FsmState::Done)
                    } else {
                        let node = self.node_for_idx(*idx)?;
                        Some(FsmState::ObjValue {
                            idx: *idx,
                            value: start_value_state(node),
                        })
                    }
                } else {
                    Some(FsmState::ObjKey {
                        idx: *idx,
                        matched: next_matched,
                    })
                }
            }
            FsmState::ObjValue { idx, value } => {
                let node = self.node_for_idx(*idx)?;
                match step_value(value, node, ch) {
                    StepOutcome::Consumed(next) => Some(FsmState::ObjValue { idx: *idx, value: next }),
                    StepOutcome::ConsumedComplete => Some(FsmState::ObjAfterValue { idx: *idx }),
                    StepOutcome::Terminate => self.step(&FsmState::ObjAfterValue { idx: *idx }, ch),
                    StepOutcome::Invalid => None,
                }
            }
            FsmState::ObjAfterValue { idx } => {
                let properties = properties?;
                let next_idx = idx + 1;
                if next_idx < properties.len() {
                    (ch == ',').then_some(FsmState::ObjKey {
                        idx: next_idx,
                        matched: 0,
                    })
                } else {
                    (ch == '}').then_some(FsmState::Done)
                }
            }
            FsmState::ScalarValue(value) => {
                let node = match &self.root {
                    GrammarRoot::Scalar(node) => node,
                    GrammarRoot::Object(_) => return None,
                };
                match step_value(value, node, ch) {
                    StepOutcome::Consumed(next) => Some(FsmState::ScalarValue(next)),
                    StepOutcome::ConsumedComplete => Some(FsmState::Done),
                    StepOutcome::Terminate | StepOutcome::Invalid => None,
                }
            }
            FsmState::Done => None,
        }
    }

    /// Feed `text` through the grammar from `state`. Returns the resulting
    /// state if every character is accepted, or `None` if any character would
    /// violate the grammar (in which case the whole token must be masked).
    pub(crate) fn advance(&self, state: &FsmState, text: &str) -> Option<FsmState> {
        let mut current = state.clone();
        for ch in text.chars() {
            current = self.step(&current, ch)?;
        }
        Some(current)
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SchemaCompileError {
    #[error("schema is not valid JSON: {0}")]
    InvalidJson(String),
    #[error("unsupported schema shape: {0}")]
    Unsupported(String),
}

/// Compile a JSON Schema string into a [`CompiledFsm`]. See the module-level
/// scope note for exactly what is and isn't supported.
pub(crate) fn compile_schema(schema: &str) -> Result<CompiledFsm, SchemaCompileError> {
    let value: serde_json::Value =
        serde_json::from_str(schema).map_err(|error| SchemaCompileError::InvalidJson(error.to_string()))?;
    let root = compile_root(&value)?;
    Ok(CompiledFsm { root })
}

fn compile_root(value: &serde_json::Value) -> Result<GrammarRoot, SchemaCompileError> {
    let object = value
        .as_object()
        .ok_or_else(|| SchemaCompileError::Unsupported("schema must be a JSON object".to_owned()))?;
    let schema_type = object.get("type").and_then(serde_json::Value::as_str);
    if schema_type == Some("object") || (schema_type.is_none() && object.contains_key("properties")) {
        let properties_value = object
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| SchemaCompileError::Unsupported("object schema requires `properties`".to_owned()))?;
        let mut properties = Vec::with_capacity(properties_value.len());
        for (name, node_value) in properties_value {
            let node = compile_scalar_node(node_value)?;
            properties.push((name.clone(), node));
        }
        if properties.is_empty() {
            return Err(SchemaCompileError::Unsupported(
                "object schema must declare at least one property".to_owned(),
            ));
        }
        return Ok(GrammarRoot::Object(properties));
    }
    Ok(GrammarRoot::Scalar(compile_scalar_node(value)?))
}

fn compile_scalar_node(value: &serde_json::Value) -> Result<SchemaNode, SchemaCompileError> {
    let object = value
        .as_object()
        .ok_or_else(|| SchemaCompileError::Unsupported("property schema must be a JSON object".to_owned()))?;
    if let Some(enum_values) = object.get("enum").and_then(serde_json::Value::as_array) {
        let alternatives = enum_values
            .iter()
            .map(|literal| {
                literal
                    .as_str()
                    .map(|text| serde_json::to_string(text).expect("string serializes"))
                    .ok_or_else(|| {
                        SchemaCompileError::Unsupported(
                            "only string `enum` alternatives are supported".to_owned(),
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if alternatives.is_empty() {
            return Err(SchemaCompileError::Unsupported(
                "`enum` must declare at least one alternative".to_owned(),
            ));
        }
        return Ok(SchemaNode::Enum(alternatives));
    }
    match object.get("type").and_then(serde_json::Value::as_str) {
        Some("string") => Ok(SchemaNode::String),
        Some("number") | Some("integer") => Ok(SchemaNode::Number),
        Some("boolean") => Ok(SchemaNode::Boolean),
        Some(other) => Err(SchemaCompileError::Unsupported(format!(
            "unsupported property type `{other}` (nested objects/arrays are not supported)"
        ))),
        None => Err(SchemaCompileError::Unsupported(
            "property schema must declare a `type` or `enum`".to_owned(),
        )),
    }
}

/// Thread-safe cache of compiled FSMs, keyed by the SHA-256 hash of the
/// schema's source text, so repeated requests for the same schema (e.g.
/// recurring tool-calls) never pay the compilation cost twice.
pub(crate) struct FsmCache {
    cache: Mutex<LruCache<[u8; 32], Arc<CompiledFsm>>>,
}

impl FsmCache {
    pub(crate) fn new(capacity: usize) -> Self {
        let capacity = std::num::NonZeroUsize::new(capacity.max(1)).expect("capacity is at least 1");
        Self {
            cache: Mutex::new(LruCache::new(capacity)),
        }
    }

    /// Look up (or compile-and-insert) the FSM for `schema`.
    pub(crate) fn get_or_compile(
        &self,
        schema: &str,
    ) -> Result<Arc<CompiledFsm>, SchemaCompileError> {
        let key: [u8; 32] = Sha256::digest(schema.as_bytes()).into();
        let mut cache = self.cache.lock().expect("fsm cache mutex is never poisoned");
        if let Some(existing) = cache.get(&key) {
            return Ok(Arc::clone(existing));
        }
        let compiled = Arc::new(compile_schema(schema)?);
        cache.put(key, Arc::clone(&compiled));
        Ok(compiled)
    }
}

impl Default for FsmCache {
    fn default() -> Self {
        Self::new(64)
    }
}

/// Masks every vocabulary logit that the grammar would reject for the next
/// token, given the FSM's current state, then advances that state once the
/// caller has sampled and committed to a token.
///
/// Allowed/disallowed status is computed by feeding each candidate token's
/// decoded text through the grammar from the current state; this scans the
/// full vocabulary on every decode step, which is the known performance
/// cost of this first pass (see the module-level scope note).
pub(crate) struct FsmLogitProcessor {
    fsm: Arc<CompiledFsm>,
    state: FsmState,
}

impl FsmLogitProcessor {
    pub(crate) fn new(fsm: Arc<CompiledFsm>) -> Self {
        let state = fsm.initial_state();
        Self { fsm, state }
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.fsm.is_complete(&self.state)
    }

    /// Build the set of vocabulary token ids that are valid to emit next,
    /// given the grammar's current state. `decode_one` decodes a single
    /// token id to its surface text (e.g. via the model's tokenizer).
    pub(crate) fn allowed_token_ids(
        &self,
        vocab_size: usize,
        eos_tokens: &[u32],
        decode_one: impl Fn(u32) -> String,
    ) -> Vec<u32> {
        let mut allowed = Vec::new();
        if self.fsm.is_complete(&self.state) {
            allowed.extend(eos_tokens.iter().copied());
        }
        for id in 0..vocab_size as u32 {
            if eos_tokens.contains(&id) {
                continue;
            }
            let text = decode_one(id);
            if text.is_empty() {
                continue;
            }
            if self.fsm.advance(&self.state, &text).is_some() {
                allowed.push(id);
            }
        }
        allowed
    }

    /// Advance the grammar state after `token_text` has been committed as
    /// the sampled next token. Must only be called with text the grammar
    /// actually accepted (i.e. a token returned by [`Self::allowed_token_ids`]).
    pub(crate) fn commit(&mut self, token_text: &str) {
        if let Some(next) = self.fsm.advance(&self.state, token_text) {
            self.state = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fsm_for(schema: &str) -> CompiledFsm {
        compile_schema(schema).expect("schema compiles")
    }

    fn run(fsm: &CompiledFsm, text: &str) -> Option<FsmState> {
        fsm.advance(&fsm.initial_state(), text)
    }

    #[test]
    fn flat_object_schema_accepts_only_its_exact_shape() {
        let fsm = fsm_for(
            r#"{"type":"object","properties":{"name":{"type":"string"},"age":{"type":"integer"}}}"#,
        );
        // Properties are emitted in alphabetical key order ("age" before
        // "name"), regardless of the order they were declared in the schema.
        let accepted = run(&fsm, r#"{"age":37,"name":"Ada"}"#).expect("matches grammar");
        assert!(fsm.is_complete(&accepted));

        // Declaration order ("name" before "age") is rejected.
        assert!(run(&fsm, r#"{"name":"Ada","age":37}"#).is_none());
        // An extra, undeclared key is rejected.
        assert!(run(&fsm, r#"{"age":37,"name":"Ada","extra":1}"#).is_none());
        // Whitespace between tokens is rejected (compact JSON only).
        assert!(run(&fsm, r#"{"age":37,"name": "Ada"}"#).is_none());
    }

    #[test]
    fn string_enum_only_accepts_declared_alternatives() {
        let fsm = fsm_for(r#"{"type":"object","properties":{"color":{"enum":["red","blue"]}}}"#);
        assert!(run(&fsm, r#"{"color":"red"}"#).is_some());
        assert!(run(&fsm, r#"{"color":"blue"}"#).is_some());
        assert!(run(&fsm, r#"{"color":"green"}"#).is_none());
    }

    #[test]
    fn scalar_top_level_number_schema() {
        let fsm = fsm_for(r#"{"type":"number"}"#);
        let state = run(&fsm, "42").expect("digits accepted");
        assert!(fsm.is_complete(&state));
        assert!(run(&fsm, "abc").is_none());
    }

    #[test]
    fn unsupported_nested_object_is_rejected_at_compile_time() {
        let error = compile_schema(
            r#"{"type":"object","properties":{"inner":{"type":"object","properties":{}}}}"#,
        )
        .expect_err("nested objects are out of scope");
        assert!(matches!(error, SchemaCompileError::Unsupported(_)));
    }

    #[test]
    fn fsm_cache_reuses_a_compiled_schema_for_an_identical_hash() {
        let cache = FsmCache::new(4);
        let schema = r#"{"type":"object","properties":{"ok":{"type":"boolean"}}}"#;
        let first = cache.get_or_compile(schema).expect("compiles");
        let second = cache.get_or_compile(schema).expect("cache hit");
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn logit_processor_allows_only_grammar_consistent_tokens() {
        let fsm = Arc::new(fsm_for(r#"{"type":"object","properties":{"ok":{"type":"boolean"}}}"#));
        let processor = FsmLogitProcessor::new(fsm);
        // A tiny synthetic vocabulary: id 0 is the correct next fragment,
        // id 1 is grammar-violating, id 2 decodes to an empty string.
        let vocab = ["{\"ok\":t", "xyz", ""];
        let allowed = processor.allowed_token_ids(vocab.len(), &[], |id| vocab[id as usize].to_owned());
        assert_eq!(allowed, vec![0]);
    }

    #[test]
    fn logit_processor_commit_advances_state_to_completion() {
        let fsm = Arc::new(fsm_for(r#"{"type":"string"}"#));
        let mut processor = FsmLogitProcessor::new(fsm);
        assert!(!processor.is_complete());
        processor.commit("\"hi\"");
        assert!(processor.is_complete());
    }
}
