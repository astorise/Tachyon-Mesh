use anyhow::{Context, Result};
use ed25519_dalek::{Signer, SigningKey};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

const HOST_ADDRESS: &str = "0.0.0.0:8080";
const MAX_STDOUT_BYTES: usize = 64 * 1024;
const GUEST_FUEL_BUDGET: u64 = 250_000;
const GUEST_MEMORY_LIMIT_BYTES: usize = 50 * 1024 * 1024;
const RESOURCE_LIMIT_RESPONSE: &str = "Execution trapped: Resource limit exceeded";

#[derive(Serialize)]
struct SealedConfig<'a> {
    host_address: &'a str,
    max_stdout_bytes: usize,
    guest_fuel_budget: u64,
    guest_memory_limit_bytes: usize,
    resource_limit_response: &'a str,
}

#[derive(Debug, Deserialize, Serialize)]
struct IntegrityManifest {
    config_payload: String,
    public_key: String,
    signature: String,
}

fn main() -> Result<()> {
    let config_payload = canonical_config_payload()?;
    let payload_hash = Sha256::digest(config_payload.as_bytes());

    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();
    let signature = signing_key.sign(&payload_hash);

    let manifest = IntegrityManifest {
        config_payload,
        public_key: hex::encode(verifying_key.to_bytes()),
        signature: hex::encode(signature.to_bytes()),
    };

    let manifest_path = workspace_root().join("integrity.lock");
    fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?).with_context(|| {
        format!(
            "failed to write integrity manifest to {}",
            manifest_path.display()
        )
    })?;

    println!("wrote integrity manifest to {}", manifest_path.display());
    Ok(())
}

fn canonical_config_payload() -> Result<String> {
    serde_json::to_string(&SealedConfig {
        host_address: HOST_ADDRESS,
        max_stdout_bytes: MAX_STDOUT_BYTES,
        guest_fuel_budget: GUEST_FUEL_BUDGET,
        guest_memory_limit_bytes: GUEST_MEMORY_LIMIT_BYTES,
        resource_limit_response: RESOURCE_LIMIT_RESPONSE,
    })
    .context("failed to serialize canonical configuration payload")
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("cli-signer should live directly under the workspace root")
        .to_path_buf()
}
