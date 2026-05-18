//! Lifecycle smoke test for the `system-faas-view-builder` component.

use std::path::Path;

const ARTIFACT_RELATIVE_PATH: &str =
    "../target/wasm32-wasip2/release/system_faas_view_builder.wasm";

#[test]
fn view_builder_wasm_artifact_is_present_if_built() {
    let artifact = Path::new(ARTIFACT_RELATIVE_PATH);
    if !artifact.exists() {
        eprintln!(
            "skipping view-builder artifact check: `{}` not built locally",
            ARTIFACT_RELATIVE_PATH
        );
        return;
    }
    let metadata = std::fs::metadata(artifact)
        .expect("view-builder artifact metadata should be readable when present");
    assert!(
        metadata.len() > 0,
        "view-builder artifact must not be empty"
    );
}

#[test]
fn view_builder_source_stays_in_workspace() {
    assert!(
        Path::new("../systems/system-faas-view-builder/Cargo.toml").exists(),
        "view-builder package must stay in the workspace"
    );
}
