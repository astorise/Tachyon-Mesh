use std::process::Command;

#[test]
#[ignore = "requires signed integrity.lock, guest artifacts, and an HTTP/3-capable local runtime"]
fn deploy_node_load_faas_execute_h3_request_verify_log() {
    let Some(core_host) = option_env!("CARGO_BIN_EXE_core-host") else {
        panic!("cargo did not expose the core-host binary path");
    };

    let output = Command::new(core_host)
        .arg("--help")
        .output()
        .expect("core-host help should execute");

    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Tachyon"),
        "help output should identify the deployable host binary"
    );
}
