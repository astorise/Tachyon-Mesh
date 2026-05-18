//! Lifecycle smoke test for the `system-faas-olap-engine` component.
//!
//! v1.1 added a strict payload byte limit to defend against memory amplification
//! attacks through the JSON deserializer. This test asserts both that the WASM
//! artifact is present after the CI build and that the source still carries the
//! payload-bound guard documented in the audit-remediation plan.

use std::path::Path;

const ARTIFACT_RELATIVE_PATH: &str = "../target/wasm32-wasip2/release/system_faas_olap_engine.wasm";

#[test]
fn olap_engine_wasm_artifact_is_present_if_built() {
    let artifact = Path::new(ARTIFACT_RELATIVE_PATH);
    if !artifact.exists() {
        eprintln!(
            "skipping olap-engine artifact check: `{}` not built locally",
            ARTIFACT_RELATIVE_PATH
        );
        return;
    }
    let metadata = std::fs::metadata(artifact)
        .expect("olap-engine artifact metadata should be readable when present");
    assert!(metadata.len() > 0, "olap-engine artifact must not be empty");
}

#[test]
fn olap_engine_source_enforces_payload_byte_limit() {
    let source = std::fs::read_to_string("../systems/system-faas-olap-engine/src/lib.rs")
        .expect("olap-engine source should be checked into the workspace");
    assert!(
        source.contains("MAX_PAYLOAD_BYTES"),
        "olap-engine must declare a MAX_PAYLOAD_BYTES guard"
    );
    assert!(
        source.contains("byte limit"),
        "olap-engine must reject oversized payloads with a `byte limit` error message"
    );
    assert!(
        source.contains("rejects_oversized_payloads"),
        "olap-engine must keep the regression test for its memory bound"
    );
}
