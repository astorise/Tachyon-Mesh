fn main() {
    println!("cargo:rerun-if-changed=Cargo.toml");

    let version =
        std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION must be set by Cargo");
    let bytes: Vec<String> = version.bytes().map(|b| b.to_string()).collect();
    let n = bytes.len();
    let bytes_str = bytes.join(", ");

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR must be set by Cargo");
    std::fs::write(
        format!("{out_dir}/version_section.rs"),
        format!(
            "/// WASM custom section — embeds the crate version for offline inspection.\n\
             #[cfg(target_arch = \"wasm32\")]\n\
             #[link_section = \"tachyon.version\"]\n\
             #[used]\n\
             static _TACHYON_VERSION: [u8; {n}] = [{bytes_str}];\n"
        ),
    )
    .expect("failed to write version_section.rs");
}
