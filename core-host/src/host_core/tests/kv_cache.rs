use super::support_and_cache::*;
use crate::*;

// ── store::CoreStore KV-cache unit tests ─────────────────────────────────────
// These tests exercise the storage layer directly (no HTTP) and verify that
// the model-based key namespacing isolates entries from different models.

fn open_test_store() -> Arc<store::CoreStore> {
    let dir = unique_test_dir("kv-cache-store");
    Arc::new(
        store::CoreStore::open(&dir.join("core.db"))
            .expect("test core store should open"),
    )
}

#[test]
fn kv_cache_put_and_get_round_trips() {
    let store = open_test_store();
    store
        .kv_cache_put("llama-3", "tenant-a", "ctx:001", b"token_state", None)
        .expect("put should succeed");

    let result = store
        .kv_cache_get("llama-3", "tenant-a", "ctx:001")
        .expect("get should succeed");
    assert_eq!(result.as_deref(), Some(b"token_state".as_slice()));
}

#[test]
fn kv_cache_returns_none_for_missing_key() {
    let store = open_test_store();
    let result = store
        .kv_cache_get("llama-3", "tenant-a", "no-such-key")
        .expect("get should not error");
    assert!(result.is_none());
}

#[test]
fn kv_cache_entries_are_isolated_by_model() {
    let store = open_test_store();
    store
        .kv_cache_put("llama-3", "tenant-a", "ctx:001", b"llama_state", None)
        .expect("llama put");
    store
        .kv_cache_put("mistral-7b", "tenant-a", "ctx:001", b"mistral_state", None)
        .expect("mistral put");

    let llama = store
        .kv_cache_get("llama-3", "tenant-a", "ctx:001")
        .expect("llama get");
    let mistral = store
        .kv_cache_get("mistral-7b", "tenant-a", "ctx:001")
        .expect("mistral get");

    assert_eq!(llama.as_deref(), Some(b"llama_state".as_slice()));
    assert_eq!(mistral.as_deref(), Some(b"mistral_state".as_slice()));
    // Verify they are NOT the same bytes — the isolation is real.
    assert_ne!(llama, mistral);
}

#[test]
fn kv_cache_entries_are_isolated_by_tenant() {
    let store = open_test_store();
    store
        .kv_cache_put("llama-3", "tenant-a", "ctx:001", b"state_a", None)
        .expect("tenant-a put");
    store
        .kv_cache_put("llama-3", "tenant-b", "ctx:001", b"state_b", None)
        .expect("tenant-b put");

    let a = store
        .kv_cache_get("llama-3", "tenant-a", "ctx:001")
        .expect("tenant-a get");
    let b = store
        .kv_cache_get("llama-3", "tenant-b", "ctx:001")
        .expect("tenant-b get");

    assert_eq!(a.as_deref(), Some(b"state_a".as_slice()));
    assert_eq!(b.as_deref(), Some(b"state_b".as_slice()));
    assert_ne!(a, b);
}

#[test]
fn kv_cache_delete_removes_entry() {
    let store = open_test_store();
    store
        .kv_cache_put("llama-3", "tenant-a", "ctx:001", b"state", None)
        .expect("put");
    store
        .kv_cache_delete("llama-3", "tenant-a", "ctx:001")
        .expect("delete");
    let result = store
        .kv_cache_get("llama-3", "tenant-a", "ctx:001")
        .expect("get after delete");
    assert!(result.is_none());
}

#[test]
fn kv_cache_evict_model_removes_only_target_model() {
    let store = open_test_store();
    // Write entries for two different models.
    for i in 0..3 {
        store
            .kv_cache_put("llama-3", "tenant-a", &format!("ctx:{i}"), b"llama", None)
            .expect("llama put");
    }
    for i in 0..2 {
        store
            .kv_cache_put("mistral-7b", "tenant-a", &format!("ctx:{i}"), b"mistral", None)
            .expect("mistral put");
    }

    let evicted = store
        .kv_cache_evict_model("llama-3")
        .expect("eviction should succeed");
    assert_eq!(evicted, 3, "all llama entries should be evicted");

    // Llama entries are gone.
    for i in 0..3 {
        assert!(
            store
                .kv_cache_get("llama-3", "tenant-a", &format!("ctx:{i}"))
                .expect("get")
                .is_none(),
            "llama ctx:{i} should be evicted"
        );
    }

    // Mistral entries are untouched.
    for i in 0..2 {
        assert!(
            store
                .kv_cache_get("mistral-7b", "tenant-a", &format!("ctx:{i}"))
                .expect("get")
                .is_some(),
            "mistral ctx:{i} should survive llama eviction"
        );
    }
}

#[test]
fn kv_cache_expired_entry_returns_none_on_read() {
    let store = open_test_store();
    // TTL of 0 seconds → expires immediately.
    store
        .kv_cache_put("llama-3", "tenant-a", "ctx:ttl", b"state", Some(0))
        .expect("put with zero TTL");
    let result = store
        .kv_cache_get("llama-3", "tenant-a", "ctx:ttl")
        .expect("get expired entry");
    assert!(result.is_none(), "expired entry should be treated as absent");
}

#[test]
fn kv_cache_stats_counts_live_entries_per_model() {
    let store = open_test_store();
    for i in 0..4 {
        store
            .kv_cache_put("llama-3", "tenant-a", &format!("ctx:{i}"), b"x", None)
            .expect("put");
    }
    // One expired entry.
    store
        .kv_cache_put("llama-3", "tenant-a", "ctx:expired", b"y", Some(0))
        .expect("put expired");

    let stats = store.kv_cache_stats("llama-3").expect("stats");
    assert_eq!(stats.entry_count, 4);
    assert_eq!(stats.expired_count, 1);
    assert_eq!(stats.total_bytes, 4); // 4 × b"x"
}

// ── HTTP handler tests ───────────────────────────────────────────────────────

fn make_kv_cache_config(model_ref: &str) -> IntegrityKvCacheConfig {
    IntegrityKvCacheConfig {
        name: format!("cache-for-{model_ref}"),
        model_ref: model_ref.to_owned(),
        max_ttl_seconds: None,
        eviction_policy: KvCacheEvictionPolicy::Lru,
        tenant_isolation: true,
    }
}

fn state_with_kv_cache(model_ref: &str) -> AppState {
    let mut config = IntegrityConfig::default_sealed();
    config.kv_caches.push(make_kv_cache_config(model_ref));
    build_test_state(config, telemetry::init_test_telemetry())
}

#[tokio::test]
async fn kv_cache_get_returns_404_for_unconfigured_model() {
    let state = build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    );
    let response = kv_cache::kv_cache_get_handler(
        State(state),
        axum::extract::Path(("llama-3".to_owned(), "ctx:001".to_owned())),
        axum::http::HeaderMap::new(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn kv_cache_put_returns_404_for_unconfigured_model() {
    let state = build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    );
    let response = kv_cache::kv_cache_put_handler(
        State(state),
        axum::extract::Path(("llama-3".to_owned(), "ctx:001".to_owned())),
        axum::http::HeaderMap::new(),
        bytes::Bytes::from_static(b"token_state"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn kv_cache_put_returns_503_when_model_not_hot() {
    // Model is configured in kv_caches but not loaded (hot_model_aliases() = []
    // when ai-inference feature is disabled, which is the test build).
    let state = state_with_kv_cache("llama-3");
    let response = kv_cache::kv_cache_put_handler(
        State(state),
        axum::extract::Path(("llama-3".to_owned(), "ctx:001".to_owned())),
        axum::http::HeaderMap::new(),
        bytes::Bytes::from_static(b"token_state"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn kv_cache_evict_returns_eviction_count() {
    let state = state_with_kv_cache("llama-3");
    // Pre-populate entries directly through the store.
    for i in 0..3 {
        state
            .core_store
            .kv_cache_put("llama-3", "tenant-a", &format!("ctx:{i}"), b"s", None)
            .expect("pre-populate");
    }

    let response = kv_cache::kv_cache_evict_handler(
        State(state),
        axum::extract::Path("llama-3".to_owned()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["evicted"], 3);
}

#[tokio::test]
async fn kv_cache_stats_returns_json_summary() {
    let state = state_with_kv_cache("llama-3");
    for i in 0..2 {
        state
            .core_store
            .kv_cache_put("llama-3", "tenant-a", &format!("ctx:{i}"), b"ab", None)
            .expect("pre-populate");
    }

    let response = kv_cache::kv_cache_stats_handler(
        State(state),
        axum::extract::Path("llama-3".to_owned()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["entry_count"], 2);
    assert_eq!(json["total_bytes"], 4);
}

// ── validate_kv_caches tests ─────────────────────────────────────────────────

#[test]
fn validate_kv_caches_accepts_valid_config() {
    let mut config = IntegrityConfig::default_sealed();
    config.kv_caches.push(make_kv_cache_config("llama-3"));
    validate_kv_caches(&config).expect("valid config should pass");
}

#[test]
fn validate_kv_caches_rejects_empty_name() {
    let mut config = IntegrityConfig::default_sealed();
    config.kv_caches.push(IntegrityKvCacheConfig {
        name: "  ".to_owned(),
        model_ref: "llama-3".to_owned(),
        max_ttl_seconds: None,
        eviction_policy: KvCacheEvictionPolicy::Lru,
        tenant_isolation: true,
    });
    validate_kv_caches(&config).expect_err("empty name should fail");
}

#[test]
fn validate_kv_caches_rejects_slash_in_model_ref() {
    let mut config = IntegrityConfig::default_sealed();
    config.kv_caches.push(IntegrityKvCacheConfig {
        name: "my-cache".to_owned(),
        model_ref: "bad/model".to_owned(),
        max_ttl_seconds: None,
        eviction_policy: KvCacheEvictionPolicy::Lru,
        tenant_isolation: true,
    });
    validate_kv_caches(&config).expect_err("slash in model_ref should fail");
}

#[test]
fn validate_kv_caches_rejects_duplicate_names() {
    let mut config = IntegrityConfig::default_sealed();
    config.kv_caches.push(make_kv_cache_config("llama-3"));
    config.kv_caches.push(make_kv_cache_config("llama-3"));
    validate_kv_caches(&config).expect_err("duplicate name should fail");
}
