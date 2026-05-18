//! Lifecycle smoke test for the `system-faas-sql-engine` component.

use std::path::Path;

const ARTIFACT_RELATIVE_PATH: &str = "../target/wasm32-wasip2/release/system_faas_sql_engine.wasm";

#[test]
fn sql_engine_wasm_artifact_is_present_if_built() {
    let artifact = Path::new(ARTIFACT_RELATIVE_PATH);
    if !artifact.exists() {
        eprintln!(
            "skipping sql-engine artifact check: `{}` not built locally",
            ARTIFACT_RELATIVE_PATH
        );
        return;
    }
    let metadata = std::fs::metadata(artifact)
        .expect("sql-engine artifact metadata should be readable when present");
    assert!(metadata.len() > 0, "sql-engine artifact must not be empty");
}

#[test]
fn sql_engine_source_stays_in_workspace() {
    assert!(
        Path::new("../systems/system-faas-sql-engine/Cargo.toml").exists(),
        "sql-engine package must stay in the workspace"
    );
}
