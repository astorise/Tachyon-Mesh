//! Native JSON-Schema-constrained decoding.
//!
//! The grammar automaton, its schema compiler and the logit processor that
//! masks disallowed tokens all live in `candle_transformers::generation`
//! (candle #3826). This module is what remains once they do: the re-exports
//! the rest of the host names, and the compilation cache, which is a serving
//! concern rather than a decoding one.
//!
//! ## What upstream accepts
//!
//! The upstream constraint implements JSON Schema's semantics, where the local
//! grammar it replaces implemented a narrower serialization of them: keys in
//! alphabetical order, every declared property present, no whitespace, no
//! nesting. Those were properties of the compiler, not of the schemas, and the
//! model was being held to them.
//!
//! The practical consequence is that a schema now means what it says. In
//! particular `properties` does not imply `required`, and it does not imply
//! `additionalProperties: false` — so an object schema that must have exactly
//! its declared keys has to say so. Schemas that relied on the old grammar to
//! enforce that implicitly will accept more than they used to.
//!
//! Nested objects and arrays, `required`, `additionalProperties`, non-compact
//! whitespace and `\u` escapes (including surrogate pairs) are all supported
//! upstream and were all rejected here.

use std::sync::{Arc, Mutex};

use lru::LruCache;
use sha2::{Digest, Sha256};

pub(crate) use candle_transformers::generation::{
    compile_schema, CompiledFsm, FsmLogitProcessor, SchemaCompileError,
};

/// Thread-safe cache of compiled FSMs, keyed by the SHA-256 hash of the
/// schema's source text, so repeated requests for the same schema (e.g.
/// recurring tool-calls) never pay the compilation cost twice.
pub(crate) struct FsmCache {
    cache: Mutex<LruCache<[u8; 32], Arc<CompiledFsm>>>,
}

impl FsmCache {
    pub(crate) fn new(capacity: usize) -> Self {
        let capacity =
            std::num::NonZeroUsize::new(capacity.max(1)).expect("capacity is at least 1");
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
        let mut cache = self
            .cache
            .lock()
            .expect("fsm cache mutex is never poisoned");
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed `text` through a fresh processor one whole string at a time, and
    /// report whether it was accepted and whether the value is finished. This
    /// is the question the decode loop asks, expressed on a single token.
    fn run(schema: &str, text: &str) -> (bool, bool) {
        let fsm = Arc::new(compile_schema(schema).expect("schema compiles"));
        let mut processor = FsmLogitProcessor::new(fsm);
        let accepted = !processor
            .allowed_token_ids(1, &[], |_| text.to_owned())
            .is_empty();
        if !accepted {
            return (false, false);
        }
        processor.commit(text);
        (true, processor.is_complete())
    }

    #[test]
    fn a_flat_object_schema_accepts_its_declared_shape() {
        const SCHEMA: &str =
            r#"{"type":"object","properties":{"name":{"type":"string"},"age":{"type":"integer"}}}"#;
        assert_eq!(run(SCHEMA, r#"{"age":37,"name":"Ada"}"#), (true, true));
        // Key order is not a schema constraint, and neither is whitespace.
        // The old local grammar rejected both; a model held to them was being
        // constrained by the compiler rather than by the schema.
        assert_eq!(run(SCHEMA, r#"{"name":"Ada","age":37}"#), (true, true));
        assert_eq!(run(SCHEMA, r#"{"age":37,"name": "Ada"}"#), (true, true));
        // A declared property still has to hold its declared type.
        assert_eq!(run(SCHEMA, r#"{"age":"37"}"#), (false, false));
    }

    #[test]
    fn additional_properties_are_refused_only_when_the_schema_refuses_them() {
        const OPEN: &str = r#"{"type":"object","properties":{"a":{"type":"integer"}}}"#;
        const CLOSED: &str = r#"{"type":"object","properties":{"a":{"type":"integer"}},"additionalProperties":false}"#;
        assert_eq!(run(OPEN, r#"{"a":1,"b":2}"#), (true, true));
        assert_eq!(run(CLOSED, r#"{"a":1,"b":2}"#), (false, false));
    }

    #[test]
    fn a_required_property_is_not_complete_until_it_is_present() {
        const SCHEMA: &str = r#"{"type":"object","properties":{"a":{"type":"integer"},"b":{"type":"integer"}},"required":["a","b"]}"#;
        // Accepted and unfinished: `b` can still follow.
        assert_eq!(run(SCHEMA, r#"{"a":1,"#), (true, false));
        assert_eq!(run(SCHEMA, r#"{"a":1,"b":2}"#), (true, true));
        // The closing brace is refused outright rather than accepted into a
        // state that can never complete — which is the difference between a
        // constraint that fails and one that hangs to the token limit.
        assert_eq!(run(SCHEMA, r#"{"a":1}"#), (false, false));
    }

    #[test]
    fn a_string_enum_only_accepts_declared_alternatives() {
        const SCHEMA: &str = r#"{"type":"object","properties":{"color":{"enum":["red","blue"]}}}"#;
        assert_eq!(run(SCHEMA, r#"{"color":"red"}"#), (true, true));
        assert_eq!(run(SCHEMA, r#"{"color":"blue"}"#), (true, true));
        assert_eq!(run(SCHEMA, r#"{"color":"green"}"#), (false, false));
    }

    #[test]
    fn an_enum_cannot_switch_to_an_alternative_whose_prefix_diverged() {
        // "ab" and "ca" share a character at offset 1 but no common prefix. A
        // byte-offset-only matcher accepts "aa" by taking `a` from "ab" and
        // then `a` from "ca".
        const SCHEMA: &str = r#"{"type":"object","properties":{"v":{"enum":["ab","ca"]}}}"#;
        assert_eq!(run(SCHEMA, r#"{"v":"ab"}"#), (true, true));
        assert_eq!(run(SCHEMA, r#"{"v":"ca"}"#), (true, true));
        assert_eq!(run(SCHEMA, r#"{"v":"aa"}"#), (false, false));
        assert_eq!(run(SCHEMA, r#"{"v":"cb"}"#), (false, false));
    }

    #[test]
    fn malformed_numbers_are_rejected() {
        const SCHEMA: &str = r#"{"type":"number"}"#;
        assert_eq!(run(SCHEMA, "42"), (true, true));
        assert_eq!(run(SCHEMA, "1.5e-3"), (true, true));
        assert_eq!(run(SCHEMA, "-0"), (true, true));
        assert_eq!(run(SCHEMA, "abc"), (false, false));
        assert_eq!(run(SCHEMA, "1+"), (false, false));
        assert_eq!(run(SCHEMA, "1..2"), (false, false));
        assert_eq!(run(SCHEMA, "1e}"), (false, false));
        assert_eq!(run(SCHEMA, ".5"), (false, false));
        // Valid prefixes that are not values: more input must follow, so
        // generation cannot stop on either.
        assert_eq!(run(SCHEMA, "1e"), (true, false));
        assert_eq!(run(SCHEMA, "-"), (true, false));
    }

    #[test]
    fn an_escaped_quote_does_not_close_the_string_but_an_unescaped_one_does() {
        const SCHEMA: &str = r#"{"type":"string"}"#;
        assert_eq!(run(SCHEMA, r#""a\""#), (true, false));
        assert_eq!(run(SCHEMA, r#""a\"b""#), (true, true));
    }

    #[test]
    fn nested_objects_and_arrays_compile_rather_than_being_refused() {
        // Both were `Unsupported` under the local compiler, which meant a
        // caller asking for them got an error instead of a constraint.
        const SCHEMA: &str = r#"{"type":"object","properties":{"items":{"type":"array","items":{"type":"object","properties":{"id":{"type":"integer"}}}}}}"#;
        assert_eq!(
            run(SCHEMA, r#"{"items":[{"id":1},{"id":2}]}"#),
            (true, true)
        );
        assert_eq!(run(SCHEMA, r#"{"items":[{"id":"x"}]}"#), (false, false));
    }

    #[test]
    fn an_unsupported_construct_fails_at_compile_time() {
        let error = compile_schema(r##"{"type":"object","properties":{"a":{"$ref":"#/x"}}}"##)
            .expect_err("$ref is out of scope");
        assert!(matches!(
            error,
            SchemaCompileError::Unsupported(_) | SchemaCompileError::InvalidSchema(_)
        ));
    }

    #[test]
    fn the_processor_masks_every_token_the_grammar_would_reject() {
        let fsm = Arc::new(
            compile_schema(r#"{"type":"object","properties":{"ok":{"type":"boolean"}}}"#)
                .expect("schema compiles"),
        );
        let processor = FsmLogitProcessor::new(fsm);
        // A tiny synthetic vocabulary: id 0 is a legal next fragment, id 1
        // violates the grammar, id 2 decodes to an empty string.
        let vocab = ["{\"ok\":t", "xyz", ""];
        let allowed =
            processor.allowed_token_ids(vocab.len(), &[], |id| vocab[id as usize].to_owned());
        assert_eq!(allowed, vec![0]);
    }

    #[test]
    fn eos_is_allowed_only_once_the_value_is_complete() {
        let fsm = Arc::new(compile_schema(r#"{"type":"string"}"#).expect("schema compiles"));
        let mut processor = FsmLogitProcessor::new(fsm);
        let vocab = ["\"hi\"", "<eos>"];
        let decode = |id: u32| vocab[id as usize].to_owned();
        assert!(!processor.allowed_token_ids(2, &[1], decode).contains(&1));
        processor.commit("\"hi\"");
        assert!(processor.is_complete());
        assert!(processor.allowed_token_ids(2, &[1], decode).contains(&1));
    }

    #[test]
    fn the_cache_reuses_a_compiled_schema_for_an_identical_hash() {
        let cache = FsmCache::new(4);
        let schema = r#"{"type":"object","properties":{"ok":{"type":"boolean"}}}"#;
        let first = cache.get_or_compile(schema).expect("compiles");
        let second = cache.get_or_compile(schema).expect("cache hit");
        assert!(Arc::ptr_eq(&first, &second));
    }
}
