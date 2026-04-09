use std::env;

fn main() {
    println!("cargo:rerun-if-changed=native/turboquant.cpp");
    println!("cargo:rerun-if-changed=native/turboquant.hpp");
    println!("cargo:rerun-if-changed=tools/generate_fixtures.cpp");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .file("native/turboquant.cpp")
        .include("native");
    build.flag_if_supported("/std:c++17");
    build.flag_if_supported("-std=c++17");

    if env::var_os("TACHYON_TURBOQUANT_ENABLE_CUDA").is_some() {
        build.define("TURBOQUANT_WITH_CUDA", Some("1"));
    }
    if target_os == "macos" || env::var_os("TACHYON_TURBOQUANT_ENABLE_METAL").is_some() {
        build.define("TURBOQUANT_WITH_METAL", Some("1"));
    }

    build.compile("turboquant_native");
}
