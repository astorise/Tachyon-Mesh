//! Lifecycle smoke test for the `system-faas-vector-search` component.

use std::path::Path;

const ARTIFACT_RELATIVE_PATH: &str =
    "../target/wasm32-wasip2/release/system_faas_vector_search.wasm";

#[test]
fn vector_search_wasm_artifact_is_present_if_built() {
    let artifact = Path::new(ARTIFACT_RELATIVE_PATH);
    if !artifact.exists() {
        eprintln!(
            "skipping vector-search artifact check: `{}` not built locally",
            ARTIFACT_RELATIVE_PATH
        );
        return;
    }
    let metadata = std::fs::metadata(artifact)
        .expect("vector-search artifact metadata should be readable when present");
    assert!(
        metadata.len() > 0,
        "vector-search artifact must not be empty"
    );
}

#[test]
fn vector_search_source_stays_in_workspace() {
    assert!(
        Path::new("../systems/system-faas-vector-search/Cargo.toml").exists(),
        "vector-search package must stay in the workspace"
    );
}
