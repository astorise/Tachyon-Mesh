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

    prepare_ebpf_artifact();
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

fn prepare_ebpf_artifact() {
    let Some(out_dir) = std::env::var_os("OUT_DIR") else {
        return;
    };
    let out_path = PathBuf::from(out_dir).join("tachyon-ebpf");
    let artifact_path = PathBuf::from("../target/bpfel-unknown-none/release/tachyon-ebpf");
    println!("cargo:rerun-if-changed={}", artifact_path.display());

    if artifact_path.exists() {
        match fs::copy(&artifact_path, &out_path) {
            Ok(_) => {
                println!("cargo:rustc-env=TACHYON_EBPF_ARTIFACT_PRESENT=1");
                return;
            }
            Err(error) => {
                println!(
                    "cargo:warning=failed to stage eBPF artifact from {}: {error}",
                    artifact_path.display()
                );
            }
        }
    }

    if let Err(error) = fs::write(&out_path, []) {
        println!(
            "cargo:warning=failed to create placeholder eBPF artifact at {}: {error}",
            out_path.display()
        );
    }
    println!("cargo:rustc-env=TACHYON_EBPF_ARTIFACT_PRESENT=0");
}
