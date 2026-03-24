use serde::Deserialize;
use std::{fs, path::PathBuf};

#[derive(Deserialize)]
struct IntegrityManifest {
    config_payload: String,
    public_key: String,
    signature: String,
}

fn main() {
    let manifest_path = PathBuf::from("../integrity.lock");
    println!("cargo:rerun-if-changed={}", manifest_path.display());

    let manifest = read_manifest(&manifest_path);

    println!("cargo:rustc-env=FAAS_CONFIG={}", manifest.config_payload);
    println!("cargo:rustc-env=FAAS_PUBKEY={}", manifest.public_key);
    println!("cargo:rustc-env=FAAS_SIGNATURE={}", manifest.signature);
}

fn read_manifest(path: &PathBuf) -> IntegrityManifest {
    let manifest = fs::read_to_string(path).unwrap_or_else(|error| {
        panic!(
            "failed to read integrity manifest at {}: {error}",
            path.display()
        )
    });

    serde_json::from_str(&manifest).unwrap_or_else(|error| {
        panic!(
            "failed to parse integrity manifest at {}: {error}",
            path.display()
        )
    })
}
