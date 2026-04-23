use std::{env, fs, path::PathBuf};

fn ensure_frontend_dist_exists() {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should be set"));
    let dist_dir = manifest_dir.join("dist");
    let placeholder_index = dist_dir.join("index.html");

    if let Err(error) = fs::create_dir_all(&dist_dir) {
        panic!(
            "failed to create placeholder frontend dist directory `{}`: {error}",
            dist_dir.display()
        );
    }

    if !placeholder_index.exists() {
        fs::write(
            &placeholder_index,
            "<!doctype html><html><body>Tachyon UI build placeholder</body></html>",
        )
        .unwrap_or_else(|error| {
            panic!(
                "failed to create placeholder frontend entrypoint `{}`: {error}",
                placeholder_index.display()
            )
        });
    }
}

fn main() {
    ensure_frontend_dist_exists();
    tauri_build::build()
}
