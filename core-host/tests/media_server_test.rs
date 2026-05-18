//! Lifecycle smoke test for the `system-faas-media-server` component.

use std::path::Path;

const ARTIFACT_RELATIVE_PATH: &str =
    "../target/wasm32-wasip2/release/system_faas_media_server.wasm";

#[test]
fn media_server_wasm_artifact_is_present_if_built() {
    let artifact = Path::new(ARTIFACT_RELATIVE_PATH);
    if !artifact.exists() {
        eprintln!(
            "skipping media-server artifact check: `{}` not built locally",
            ARTIFACT_RELATIVE_PATH
        );
        return;
    }
    let metadata = std::fs::metadata(artifact)
        .expect("media-server artifact metadata should be readable when present");
    assert!(
        metadata.len() > 0,
        "media-server artifact must not be empty"
    );
}

#[test]
fn media_server_source_stays_in_workspace() {
    assert!(
        Path::new("../systems/system-faas-media-server/Cargo.toml").exists(),
        "media-server package must stay in the workspace"
    );
}
