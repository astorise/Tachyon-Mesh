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
    /// Inside the string body, after the opening `"`. `escaped` is `true`
    /// immediately after an unconsumed `\`, so the following character is
    /// treated as the escaped payload rather than a possible closing `"` or
    /// a fresh `\`.
    String { escaped: bool },
    Number(NumberPhase),
    /// Matching a fixed literal (`true`/`false`) or one alternative of an
    /// `enum`. `consumed` is the exact text matched so far; a candidate is
    /// only eligible to continue matching if it starts with `consumed`, so a
    /// later step can never "switch" to an alternative whose earlier prefix
    /// was never actually emitted (e.g. matching `"a"` of `"ab"` and then
    /// jumping to `"ca"` because both have `a` at the next offset).
    Literal { consumed: String },
}

/// Sub-states of a JSON number's grammar (`-?[0-9]+(\.[0-9]+)?([eE][+-]?[0-9]+)?`),
/// tracked explicitly so malformed sequences like `1+`, `1..2`, or `1e}` are
/// rejected rather than accepted by a single permissive `has_digit` flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NumberPhase {
    /// Nothing consumed yet; `-` or a digit may follow.
    Start,
    /// Leading `-` consumed; a digit is required next.
    Negative,
    /// At least one integer digit consumed; complete, and `.`/`e`/`E`/more
    /// digits may follow.
    IntDigits,
    /// `.` just consumed; a fraction digit is required next.
    FracFirst,
    /// At least one fraction digit consumed; complete, and `e`/`E`/more
    /// digits may follow.
    FracDigits,
    /// `e`/`E` just consumed; a sign or digit is required next.
    ExpFirst,
    /// Exponent sign just consumed; a digit is required next.
    ExpSign,
    /// At least one exponent digit consumed; complete, more digits may follow.
    ExpDigits,
}

impl NumberPhase {
    fn is_complete(self) -> bool {
        matches!(
            self,
            NumberPhase::IntDigits | NumberPhase::FracDigits | NumberPhase::ExpDigits
        )
    }
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
        SchemaNode::Number => ValueState::Number(NumberPhase::Start),
        SchemaNode::Boolean | SchemaNode::Enum(_) => ValueState::Literal {
            consumed: String::new(),
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
                StepOutcome::Consumed(ValueState::String { escaped: false })
            } else {
                StepOutcome::Invalid
            }
        }
        ValueState::String { escaped: true } => {
            // The previous character was an unconsumed `\`; this character is
            // its escaped payload, not a candidate closing `"` or a fresh `\`.
            if ch.is_control() {
                StepOutcome::Invalid
            } else {
                StepOutcome::Consumed(ValueState::String { escaped: false })
            }
        }
        ValueState::String { escaped: false } => {
            if ch == '"' {
                StepOutcome::ConsumedComplete
            } else if ch == '\\' {
                StepOutcome::Consumed(ValueState::String { escaped: true })
            } else if !ch.is_control() {
                StepOutcome::Consumed(ValueState::String { escaped: false })
            } else {
                StepOutcome::Invalid
            }
        }
        ValueState::Number(phase) => step_number(*phase, ch),
        ValueState::Literal { consumed } => {
            // Try every alternative literal that starts with the text
            // already consumed (so a candidate can never be switched to once
            // its earlier prefix diverges from what was actually emitted),
            // given the next character.
            let consumed_len = consumed.chars().count();
            for candidate in value_alternatives(node) {
                if !candidate.starts_with(consumed.as_str()) {
                    continue;
                }
                let Some(next_ch) = candidate.chars().nth(consumed_len) else {
                    continue;
                };
                if next_ch != ch {
                    continue;
                }
                let candidate_len = candidate.chars().count();
                if consumed_len + 1 == candidate_len {
                    return StepOutcome::ConsumedComplete;
                }
                let mut next_consumed = consumed.clone();
                next_consumed.push(ch);
                return StepOutcome::Consumed(ValueState::Literal {
                    consumed: next_consumed,
                });
            }
            StepOutcome::Invalid
        }
    }
}

/// Steps a JSON number grammar (`-?[0-9]+(\.[0-9]+)?([eE][+-]?[0-9]+)?`) by one
/// character. `Terminate` is returned once a complete phase (`IntDigits`,
/// `FracDigits`, `ExpDigits`) sees a character that doesn't extend it, so the
/// caller can reprocess that character against the surrounding grammar
/// instead of treating the number as having accepted it.
fn step_number(phase: NumberPhase, ch: char) -> StepOutcome {
    use NumberPhase::*;
    let digit = ch.is_ascii_digit();
    match phase {
        Start if ch == '-' => StepOutcome::Consumed(ValueState::Number(Negative)),
        Start if digit => StepOutcome::Consumed(ValueState::Number(IntDigits)),
        Negative if digit => StepOutcome::Consumed(ValueState::Number(IntDigits)),
        IntDigits if digit => StepOutcome::Consumed(ValueState::Number(IntDigits)),
        IntDigits if ch == '.' => StepOutcome::Consumed(ValueState::Number(FracFirst)),
        IntDigits if ch == 'e' || ch == 'E' => StepOutcome::Consumed(ValueState::Number(ExpFirst)),
        FracFirst if digit => StepOutcome::Consumed(ValueState::Number(FracDigits)),
        FracDigits if digit => StepOutcome::Consumed(ValueState::Number(FracDigits)),
        FracDigits if ch == 'e' || ch == 'E' => StepOutcome::Consumed(ValueState::Number(ExpFirst)),
        ExpFirst if ch == '+' || ch == '-' => StepOutcome::Consumed(ValueState::Number(ExpSign)),
        ExpFirst if digit => StepOutcome::Consumed(ValueState::Number(ExpDigits)),
        ExpSign if digit => StepOutcome::Consumed(ValueState::Number(ExpDigits)),
        ExpDigits if digit => StepOutcome::Consumed(ValueState::Number(ExpDigits)),
        phase if phase.is_complete() => StepOutcome::Terminate,
        _ => StepOutcome::Invalid,
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
            FsmState::ScalarValue(ValueState::Number(phase)) => phase.is_complete(),
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

    #[test]
    fn enum_matching_cannot_switch_to_an_alternative_whose_prefix_diverged() {
        // Regression test: "ab" and "ca" only share a character at offset 1,
        // not a common prefix. A byte-offset-only matcher could accept "aa"
        // by matching `a` from "ab" then `a` from "ca". The fix tracks the
        // exact consumed prefix and rejects candidates that no longer match it.
        let fsm = fsm_for(r#"{"type":"object","properties":{"v":{"enum":["ab","ca"]}}}"#);
        assert!(run(&fsm, r#"{"v":"ab"}"#).is_some());
        assert!(run(&fsm, r#"{"v":"ca"}"#).is_some());
        assert!(run(&fsm, r#"{"v":"aa"}"#).is_none());
        assert!(run(&fsm, r#"{"v":"cb"}"#).is_none());
    }

    #[test]
    fn malformed_numbers_are_rejected() {
        let fsm = fsm_for(r#"{"type":"number"}"#);
        let complete = |text: &str| run(&fsm, text).is_some_and(|s| fsm.is_complete(&s));
        assert!(complete("1.5e-3"));
        assert!(complete("-0"));
        assert!(run(&fsm, "1+").is_none());
        assert!(run(&fsm, "1..2").is_none());
        assert!(run(&fsm, "1e}").is_none());
        // "1e" is a valid prefix (more exponent input may follow) but is not
        // itself grammar-complete, so generation cannot stop here.
        assert!(!complete("1e"));
        // "-" alone is a valid prefix (a digit must follow) but not complete.
        assert!(!complete("-"));
        assert!(run(&fsm, ".5").is_none());
    }

    #[test]
    fn an_escaped_quote_does_not_close_the_string_but_an_unescaped_one_does() {
        let fsm = fsm_for(r#"{"type":"string"}"#);
        // A literal backslash-quote inside the string body is an escape
        // sequence, not the closing delimiter, so the string continues.
        let mid_string = run(&fsm, r#""a\""#).expect("escaped quote stays inside the string");
        assert!(!fsm.is_complete(&mid_string));
        // The real closing quote completes the string.
        let complete = run(&fsm, r#""a\"b""#).expect("closing quote completes the string");
        assert!(fsm.is_complete(&complete));
    }
}
