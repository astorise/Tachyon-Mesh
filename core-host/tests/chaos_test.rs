use std::{fs, path::Path, time::Duration};

use reqwest::{blocking::Client, StatusCode};

const ATTACKS: [(&str, &str); 3] = [
    ("infinite-loop", ""),
    ("memory-allocation", "allocate"),
    ("panic-unwind", "panic"),
];

#[test]
fn guest_malicious_declares_all_chaos_vectors() {
    let source = fs::read_to_string("../examples/guest-malicious/src/lib.rs")
        .expect("guest-malicious source should be readable");

    assert!(source.contains("loop_forever"));
    assert!(source.contains("allocate_excessive_memory"));
    assert!(source.contains("intentional guest panic for chaos testing"));
}

#[test]
fn guest_malicious_package_is_available_to_ci() {
    assert!(
        Path::new("../examples/guest-malicious/Cargo.toml").exists(),
        "guest-malicious crate must stay in the workspace for chaos builds"
    );
}

#[test]
#[ignore = "set TACHYON_CHAOS_BASE_URL to a running host that routes /api/guest-malicious"]
fn malicious_guest_failures_do_not_kill_host() {
    let base_url = std::env::var("TACHYON_CHAOS_BASE_URL")
        .expect("TACHYON_CHAOS_BASE_URL must point to a running core-host");
    let base_url = base_url.trim_end_matches('/');
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("chaos HTTP client should build");

    for (name, payload) in ATTACKS {
        let response = client
            .post(format!("{base_url}/api/guest-malicious"))
            .body(payload.to_owned())
            .send()
            .unwrap_or_else(|error| panic!("{name} request should receive host response: {error}"));

        assert!(
            matches!(
                response.status(),
                StatusCode::INTERNAL_SERVER_ERROR | StatusCode::REQUEST_TIMEOUT
            ),
            "{name} should fail inside the guest without crashing the host"
        );

        let health = client
            .get(format!("{base_url}/health"))
            .send()
            .expect("host should remain reachable after chaos vector");
        assert_eq!(health.status(), StatusCode::OK);
    }
}
