use serde::Deserialize;
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

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
    prepare_nvfp4_cuda_artifact();
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

fn prepare_nvfp4_cuda_artifact() {
    println!("cargo:rerun-if-env-changed=TACHYON_NVFP4_CUDA_HOME");
    println!("cargo:rerun-if-env-changed=CUDA_HOME");
    println!("cargo:rerun-if-env-changed=CUDA_PATH");
    println!("cargo:rerun-if-env-changed=TACHYON_NVCC");
    println!("cargo:rerun-if-env-changed=TACHYON_CUTLASS_INCLUDE_DIR");
    println!("cargo:rerun-if-env-changed=TACHYON_NVFP4_CUDA_ARCH");

    if env::var_os("CARGO_FEATURE_NVFP4_CUDA").is_none() {
        return;
    }

    let out_dir =
        PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR should be provided by Cargo"));
    let source = PathBuf::from("src/ai_inference/cuda/nvfp4_cutlass.cu");
    println!("cargo:rerun-if-changed={}", source.display());

    let cuda_home = env_path("TACHYON_NVFP4_CUDA_HOME")
        .or_else(|| env_path("CUDA_HOME"))
        .or_else(|| env_path("CUDA_PATH"));
    let nvcc = env_path("TACHYON_NVCC")
        .or_else(|| {
            cuda_home
                .as_ref()
                .map(|home| home.join("bin").join(exe("nvcc")))
        })
        .unwrap_or_else(|| PathBuf::from(exe("nvcc")));
    let cutlass_include = env_path("TACHYON_CUTLASS_INCLUDE_DIR").unwrap_or_else(|| {
        panic!("`nvfp4-cuda` requires TACHYON_CUTLASS_INCLUDE_DIR to point at CUTLASS headers")
    });
    if !cutlass_include.exists() {
        panic!(
            "TACHYON_CUTLASS_INCLUDE_DIR does not exist: {}",
            cutlass_include.display()
        );
    }

    let arch = env::var("TACHYON_NVFP4_CUDA_ARCH").unwrap_or_else(|_| "sm_100a".to_owned());
    let object = out_dir.join(obj("tachyon_nvfp4_cutlass"));
    let library = out_dir.join(lib("tachyon_nvfp4_cutlass"));

    let mut compile = Command::new(&nvcc);
    compile
        .arg("-std=c++17")
        .arg("-O3")
        .arg("-lineinfo")
        .arg(format!("-arch={arch}"))
        .arg("-c")
        .arg(&source)
        .arg("-o")
        .arg(&object)
        .arg(format!("-I{}", cutlass_include.display()));
    if let Some(cuda_home) = &cuda_home {
        let include = cuda_home.join("include");
        if include.exists() {
            compile.arg(format!("-I{}", include.display()));
        }
    }
    if cfg!(windows) {
        compile.arg("-Xcompiler=/EHsc");
    } else {
        compile.arg("-Xcompiler=-fPIC");
    }
    run_or_panic(compile, "compile NVFP4 CUDA/CUTLASS source");

    let mut archive = Command::new(&nvcc);
    archive.arg("--lib").arg(&object).arg("-o").arg(&library);
    run_or_panic(archive, "archive NVFP4 CUDA/CUTLASS source");

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=tachyon_nvfp4_cutlass");
    if let Some(cuda_home) = &cuda_home {
        for lib_dir in cuda_library_dirs(cuda_home) {
            if lib_dir.exists() {
                println!("cargo:rustc-link-search=native={}", lib_dir.display());
            }
        }
    }
    println!("cargo:rustc-link-lib=dylib=cudart");
    println!("cargo:rustc-cfg=tachyon_nvfp4_cuda_compiled");
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

fn obj(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.obj")
    } else {
        format!("{name}.o")
    }
}

fn lib(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.lib")
    } else {
        format!("lib{name}.a")
    }
}

fn cuda_library_dirs(cuda_home: &Path) -> Vec<PathBuf> {
    if cfg!(windows) {
        vec![cuda_home.join("lib").join("x64")]
    } else {
        vec![cuda_home.join("lib64"), cuda_home.join("lib")]
    }
}

fn run_or_panic(mut command: Command, action: &str) {
    let program = command.get_program().to_string_lossy().into_owned();
    let args = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    let output = command.output().unwrap_or_else(|error| {
        panic!("failed to run `{program}` while trying to {action}: {error}")
    });
    if !output.status.success() {
        panic!(
            "failed to {action} with `{program} {args}`\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
