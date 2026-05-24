// INVARIANT: tenant data lives exclusively in `DeploymentScopes` stored on
// `ComponentHostState`. Cached `Linker` instances must never capture any
// tenant-specific data in their closures — they remain fully tenant-agnostic.
// Per-call scope checks always read from `store.data().scopes`.

use globset::{GlobSet, GlobSetBuilder};
use serde_json::Value;
use std::{collections::BTreeMap, hash::Hash, sync::Arc};
use thiserror::Error;

// ── Scope categories ─────────────────────────────────────────────────────────

/// One entry in the routing category: a (route-path, destination) pair of globs.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct RoutingEntry {
    pub(crate) route_path: String,
    pub(crate) destination: String,
}

/// The set of authorized import patterns for a single FaaS deployment.
///
/// - A `None` variant in any category field means the interface is **absent**
///   from the linker (link-time denial; the guest cannot call it at all).
/// - A `Some(GlobSet)` with an empty result set means the interface is linked
///   but every value-based call will be denied at runtime.
/// - `allow_all = true` short-circuits every check (migration default).
#[derive(Clone, Debug)]
pub(crate) struct DeploymentScopes {
    pub(crate) allow_all: bool,

    /// `secrets-vault.get-secret(name)` patterns.
    pub(crate) secrets: Option<GlobSet>,
    /// `kv-partition.table::new(name)` patterns.
    pub(crate) kv: Option<GlobSet>,
    /// `vector.*` operations keyed by `index-name`.
    pub(crate) vector: Option<GlobSet>,
    /// `training.submit-training-job` keyed by `dataset.volume-alias`.
    pub(crate) training: Option<GlobSet>,
    /// `bridge-controller.create-bridge` client addresses.
    pub(crate) bridge: Option<GlobSet>,
    /// `routing-control.update-target` — list of (route-path glob, destination glob) pairs.
    pub(crate) routing: Option<Vec<RoutingEntry>>,
    /// `outbound-http.send-request` URL patterns (scheme://host/path, query stripped).
    pub(crate) http: Option<GlobSet>,
    /// `outbox-store.*` keyed by `db-url/table`.
    pub(crate) outbox: Option<GlobSet>,
    /// `storage-broker.*` keyed by `path` or `volume-id`.
    pub(crate) storage: Option<GlobSet>,
    /// `graph.workspace-graph::new(name)` patterns.
    pub(crate) graph: Option<GlobSet>,
}

// ── Scope shape (cache key) ───────────────────────────────────────────────────

/// A normalized, hashable key derived from a `DeploymentScopes`.
/// Two scopes with the same effective grant set produce the same `ScopeShape`.
/// Used as the key for `LinkerCache`.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct ScopeShape {
    /// Sorted map of category-name → sorted deduplicated raw patterns.
    /// `None` entry for a category = interface absent.
    /// Empty vec = interface linked, all patterns denied.
    entries: BTreeMap<&'static str, Option<Vec<String>>>,
    allow_all: bool,
}

impl ScopeShape {
    pub(crate) fn of(scopes: &DeploymentScopes) -> Self {
        if scopes.allow_all {
            return Self {
                entries: BTreeMap::new(),
                allow_all: true,
            };
        }
        let mut entries: BTreeMap<&'static str, Option<Vec<String>>> = BTreeMap::new();
        entries.insert("secrets", scopes.secrets.as_ref().map(|_| vec![]));
        entries.insert("kv", scopes.kv.as_ref().map(|_| vec![]));
        entries.insert("vector", scopes.vector.as_ref().map(|_| vec![]));
        entries.insert("training", scopes.training.as_ref().map(|_| vec![]));
        entries.insert("bridge", scopes.bridge.as_ref().map(|_| vec![]));
        entries.insert(
            "routing",
            scopes.routing.as_ref().map(|entries| {
                let mut pairs: Vec<String> = entries
                    .iter()
                    .map(|e| format!("{}→{}", e.route_path, e.destination))
                    .collect();
                pairs.sort();
                pairs.dedup();
                pairs
            }),
        );
        entries.insert("http", scopes.http.as_ref().map(|_| vec![]));
        entries.insert("outbox", scopes.outbox.as_ref().map(|_| vec![]));
        entries.insert("storage", scopes.storage.as_ref().map(|_| vec![]));
        entries.insert("graph", scopes.graph.as_ref().map(|_| vec![]));
        Self {
            entries,
            allow_all: false,
        }
    }

    /// Returns true if the category is linked (present in the scope set).
    pub(crate) fn grants(&self, category: ScopeCategory) -> bool {
        if self.allow_all {
            return true;
        }
        self.entries
            .get(category.key())
            .is_some_and(|v| v.is_some())
    }
}

// ── Categories ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) enum ScopeCategory {
    Secrets,
    Kv,
    Vector,
    Training,
    Bridge,
    Routing,
    Http,
    Outbox,
    Storage,
    // Reached via Wasmtime WIT host dispatch — invisible to the dead-code lint
    // when the experimental feature is not active.
    #[allow(dead_code)]
    Graph,
}

impl ScopeCategory {
    fn key(self) -> &'static str {
        match self {
            Self::Secrets => "secrets",
            Self::Kv => "kv",
            Self::Vector => "vector",
            Self::Training => "training",
            Self::Bridge => "bridge",
            Self::Routing => "routing",
            Self::Http => "http",
            Self::Outbox => "outbox",
            Self::Storage => "storage",
            Self::Graph => "graph",
        }
    }

    fn known_keys() -> &'static [&'static str] {
        &[
            "secrets", "kv", "vector", "training", "bridge", "routing", "http", "outbox",
            "storage", "graph",
        ]
    }
}

// ── Manifest errors ───────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub(crate) enum ScopeManifestError {
    #[error("unknown scope category `{key}` in manifest; valid categories are: {}", ScopeCategory::known_keys().join(", "))]
    UnknownCategory { key: String },

    #[error("scope category `{category}` contains a non-string pattern: {value:?}")]
    NonStringPattern { category: String, value: Value },

    #[error(
        "scope category `routing` entry #{index} is missing a destination glob (must use `route-path -> destination` format)"
    )]
    RoutingMissingDestination { index: usize },

    #[error("scope category `{category}` pattern `{pattern}` is not a valid glob: {source}")]
    InvalidGlob {
        category: String,
        pattern: String,
        source: globset::Error,
    },
}

// ── Parser ───────────────────────────────────────────────────────────────────

impl DeploymentScopes {
    /// Short-circuit: all imports allowed, no pattern checks.
    /// Emits a warning — callers should log `WARN allow-all active` for the deployment.
    pub(crate) fn allow_all() -> Self {
        Self {
            allow_all: true,
            secrets: None,
            kv: None,
            vector: None,
            training: None,
            bridge: None,
            routing: None,
            http: None,
            outbox: None,
            storage: None,
            graph: None,
        }
    }

    /// Build `DeploymentScopes` from a manifest value.
    ///
    /// Accepts:
    /// - The string `"allow-all"` → returns `allow_all()`.
    /// - A mapping of category → list of pattern strings.
    ///
    /// Unknown keys, non-string patterns, uncompilable globs, and routing entries
    /// without a destination all produce `Err(ScopeManifestError::*)`.
    pub(crate) fn from_manifest(value: &Value) -> Result<Self, ScopeManifestError> {
        // Sentinel: preserve prior behavior during migration.
        if value.as_str() == Some("allow-all") {
            return Ok(Self::allow_all());
        }

        let map = match value.as_object() {
            Some(m) => m,
            None => {
                // Treat a missing / null / invalid block the same as allow-all
                // (handled upstream as "absent field" → allow-all with warning).
                return Ok(Self::allow_all());
            }
        };

        // Validate all keys before building anything.
        for key in map.keys() {
            if !ScopeCategory::known_keys().contains(&key.as_str()) {
                return Err(ScopeManifestError::UnknownCategory { key: key.clone() });
            }
        }

        let secrets = parse_globset(map, "secrets")?;
        let kv = parse_globset(map, "kv")?;
        let vector = parse_globset(map, "vector")?;
        let training = parse_globset(map, "training")?;
        let bridge = parse_globset(map, "bridge")?;
        let http = parse_globset(map, "http")?;
        let outbox = parse_globset(map, "outbox")?;
        let storage = parse_globset(map, "storage")?;
        let graph = parse_globset(map, "graph")?;
        let routing = parse_routing(map)?;

        Ok(Self {
            allow_all: false,
            secrets,
            kv,
            vector,
            training,
            bridge,
            routing,
            http,
            outbox,
            storage,
            graph,
        })
    }

    // ── Per-category checks ───────────────────────────────────────────────────

    pub(crate) fn check_secrets(&self, name: &str) -> bool {
        if self.allow_all {
            return true;
        }
        match &self.secrets {
            None => false,
            Some(gs) => gs.is_match(name),
        }
    }

    pub(crate) fn check_kv(&self, name: &str) -> bool {
        if self.allow_all {
            return true;
        }
        match &self.kv {
            None => false,
            Some(gs) => gs.is_match(name),
        }
    }

    pub(crate) fn check_vector(&self, index_name: &str) -> bool {
        if self.allow_all {
            return true;
        }
        match &self.vector {
            None => false,
            Some(gs) => gs.is_match(index_name),
        }
    }

    pub(crate) fn check_training(&self, volume_alias: &str) -> bool {
        if self.allow_all {
            return true;
        }
        match &self.training {
            None => false,
            Some(gs) => gs.is_match(volume_alias),
        }
    }

    pub(crate) fn check_bridge(&self, addr: &str) -> bool {
        if self.allow_all {
            return true;
        }
        match &self.bridge {
            None => false,
            Some(gs) => gs.is_match(addr),
        }
    }

    /// Routing requires BOTH route-path AND destination to match at least one entry.
    pub(crate) fn check_routing(&self, route_path: &str, destination: &str) -> bool {
        if self.allow_all {
            return true;
        }
        match &self.routing {
            None => false,
            Some(entries) => entries.iter().any(|e| {
                glob_matches(&e.route_path, route_path) && glob_matches(&e.destination, destination)
            }),
        }
    }

    pub(crate) fn check_http(&self, url_without_query: &str) -> bool {
        if self.allow_all {
            return true;
        }
        match &self.http {
            None => false,
            Some(gs) => gs.is_match(url_without_query),
        }
    }

    pub(crate) fn check_outbox(&self, db_url: &str, table: &str) -> bool {
        if self.allow_all {
            return true;
        }
        let key = format!("{db_url}/{table}");
        match &self.outbox {
            None => false,
            Some(gs) => gs.is_match(&key),
        }
    }

    pub(crate) fn check_storage(&self, path_or_volume_id: &str) -> bool {
        if self.allow_all {
            return true;
        }
        match &self.storage {
            None => false,
            Some(gs) => gs.is_match(path_or_volume_id),
        }
    }

    // Reached via Wasmtime WIT host dispatch — invisible to dead-code lint without experimental.
    #[allow(dead_code)]
    pub(crate) fn check_graph(&self, name: &str) -> bool {
        if self.allow_all {
            return true;
        }
        match &self.graph {
            None => false,
            Some(gs) => gs.is_match(name),
        }
    }
}

// ── Linker cache ──────────────────────────────────────────────────────────────

/// Counters for the linker cache, driven by atomic integers.
#[derive(Debug, Default)]
pub(crate) struct LinkerCacheCounters {
    pub(crate) hits: std::sync::atomic::AtomicU64,
    pub(crate) misses: std::sync::atomic::AtomicU64,
}

impl LinkerCacheCounters {
    pub(crate) fn hit(&self) {
        self.hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    pub(crate) fn miss(&self) {
        self.misses
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// LRU-bounded cache of `Linker` instances keyed on `ScopeShape`.
///
/// The `Linker` holds no tenant data; only `StoreData` (i.e., `ComponentHostState`)
/// carries per-deployment `DeploymentScopes`. Two deployments with the same
/// `ScopeShape` safely share one `Arc<Linker>`.
pub(crate) struct LinkerCache<T> {
    inner: moka::sync::Cache<ScopeShapeKey, Arc<T>>,
    pub(crate) counters: Arc<LinkerCacheCounters>,
}

/// Cache key: `(world_tag, ScopeShape)`. The world tag distinguishes
/// linkers for different WIT worlds (faas, udp, websocket, system, background)
/// that share the same `ComponentLinker<ComponentHostState>` Rust type but
/// register different host functions.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct ScopeShapeKey(ScopeShape, &'static str);

impl<T: Send + Sync + 'static> LinkerCache<T> {
    pub(crate) fn new(capacity: u64) -> Self {
        Self {
            inner: moka::sync::Cache::new(capacity),
            counters: Arc::new(LinkerCacheCounters::default()),
        }
    }

    /// Returns a cached or freshly built `Arc<T>` for the given `(world, shape)` pair.
    ///
    /// If `build` returns `Err`, the error is propagated and nothing is inserted into
    /// the cache. Subsequent calls with the same key will try to build again.
    pub(crate) fn get_or_build<E>(
        &self,
        world: &'static str,
        shape: &ScopeShape,
        build: impl FnOnce() -> Result<T, E>,
    ) -> Result<Arc<T>, E> {
        let key = ScopeShapeKey(shape.clone(), world);
        if let Some(cached) = self.inner.get(&key) {
            self.counters.hit();
            return Ok(cached);
        }
        self.counters.miss();
        let value = Arc::new(build()?);
        self.inner.insert(key, Arc::clone(&value));
        Ok(value)
    }

    /// Flush moka's internal eviction queue. Only for use in tests.
    #[cfg(test)]
    pub(crate) fn run_pending_tasks_for_test(&self) {
        self.inner.run_pending_tasks();
    }

    /// Current number of entries in the cache. Only for use in tests.
    #[cfg(test)]
    pub(crate) fn entry_count(&self) -> u64 {
        self.inner.entry_count()
    }

    /// Register `faas_linker_cache_hit_total` and `faas_linker_cache_miss_total`
    /// with the prometheus default registry. Call once at startup; a second call
    /// (e.g. after hot-reload) is a no-op (the registry rejects duplicates silently).
    pub(crate) fn register_prometheus(&self) {
        let collector = LinkerCacheCollector {
            counters: Arc::clone(&self.counters),
            descs: LinkerCacheCollector::make_descs(),
        };
        // Ignore "already registered" errors that occur on hot-reload.
        let _ = prometheus::default_registry().register(Box::new(collector));
    }
}

/// Prometheus `Collector` that exposes `faas_linker_cache_hit_total` and
/// `faas_linker_cache_miss_total` from the `LinkerCacheCounters` atomics.
struct LinkerCacheCollector {
    counters: Arc<LinkerCacheCounters>,
    descs: Vec<prometheus::core::Desc>,
}

impl LinkerCacheCollector {
    fn make_descs() -> Vec<prometheus::core::Desc> {
        vec![
            prometheus::core::Desc::new(
                "faas_linker_cache_hit_total".to_owned(),
                "Total number of linker cache hits (shared Linker reused for matching scope shape)."
                    .to_owned(),
                vec![],
                std::collections::HashMap::new(),
            )
            .expect("faas_linker_cache_hit_total desc should be valid"),
            prometheus::core::Desc::new(
                "faas_linker_cache_miss_total".to_owned(),
                "Total number of linker cache misses (new Linker built for unseen scope shape)."
                    .to_owned(),
                vec![],
                std::collections::HashMap::new(),
            )
            .expect("faas_linker_cache_miss_total desc should be valid"),
        ]
    }

    fn atomic_counter_mf(
        desc: &prometheus::core::Desc,
        value: u64,
    ) -> prometheus::proto::MetricFamily {
        let counter = prometheus::proto::Counter {
            value: Some(value as f64),
            ..Default::default()
        };
        let mut metric = prometheus::proto::Metric::default();
        metric.set_counter(counter);
        let mut mf = prometheus::proto::MetricFamily {
            name: Some(desc.fq_name.clone()),
            help: Some(desc.help.clone()),
            ..Default::default()
        };
        mf.set_field_type(prometheus::proto::MetricType::COUNTER);
        mf.set_metric(vec![metric]);
        mf
    }
}

impl prometheus::core::Collector for LinkerCacheCollector {
    fn desc(&self) -> Vec<&prometheus::core::Desc> {
        self.descs.iter().collect()
    }

    fn collect(&self) -> Vec<prometheus::proto::MetricFamily> {
        use std::sync::atomic::Ordering::Relaxed;
        vec![
            Self::atomic_counter_mf(&self.descs[0], self.counters.hits.load(Relaxed)),
            Self::atomic_counter_mf(&self.descs[1], self.counters.misses.load(Relaxed)),
        ]
    }
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Parse a YAML/JSON map key into an `Option<GlobSet>`.
///
/// - Key absent → `None` (interface not linked).
/// - Key present → compile all pattern strings into a `GlobSet`.
fn parse_globset(
    map: &serde_json::Map<String, Value>,
    category: &'static str,
) -> Result<Option<GlobSet>, ScopeManifestError> {
    let Some(patterns_value) = map.get(category) else {
        return Ok(None);
    };
    let patterns = match patterns_value.as_array() {
        Some(arr) => arr,
        None => {
            return Err(ScopeManifestError::NonStringPattern {
                category: category.to_owned(),
                value: patterns_value.clone(),
            })
        }
    };
    let mut builder = GlobSetBuilder::new();
    for pat_value in patterns {
        let pat = match pat_value.as_str() {
            Some(s) => s,
            None => {
                return Err(ScopeManifestError::NonStringPattern {
                    category: category.to_owned(),
                    value: pat_value.clone(),
                })
            }
        };
        let glob = globset::Glob::new(pat).map_err(|e| ScopeManifestError::InvalidGlob {
            category: category.to_owned(),
            pattern: pat.to_owned(),
            source: e,
        })?;
        builder.add(glob);
    }
    Ok(Some(builder.build().map_err(|e| {
        ScopeManifestError::InvalidGlob {
            category: category.to_owned(),
            pattern: "<set>".to_owned(),
            source: e,
        }
    })?))
}

/// Parse the `routing` category. Each element must be `"<route-path> -> <destination>"`.
fn parse_routing(
    map: &serde_json::Map<String, Value>,
) -> Result<Option<Vec<RoutingEntry>>, ScopeManifestError> {
    let Some(value) = map.get("routing") else {
        return Ok(None);
    };
    let arr = match value.as_array() {
        Some(a) => a,
        None => {
            return Err(ScopeManifestError::NonStringPattern {
                category: "routing".to_owned(),
                value: value.clone(),
            })
        }
    };
    let mut entries = Vec::with_capacity(arr.len());
    for (index, item) in arr.iter().enumerate() {
        let s = match item.as_str() {
            Some(s) => s,
            None => {
                return Err(ScopeManifestError::NonStringPattern {
                    category: "routing".to_owned(),
                    value: item.clone(),
                })
            }
        };
        let (route_path, destination) = parse_routing_entry(s)
            .ok_or(ScopeManifestError::RoutingMissingDestination { index })?;
        entries.push(RoutingEntry {
            route_path: route_path.to_owned(),
            destination: destination.to_owned(),
        });
    }
    Ok(Some(entries))
}

fn parse_routing_entry(s: &str) -> Option<(&str, &str)> {
    let arrow = s.find(" -> ")?;
    let route_path = s[..arrow].trim();
    let destination = s[arrow + 4..].trim();
    if route_path.is_empty() || destination.is_empty() {
        return None;
    }
    Some((route_path, destination))
}

/// Match `value` against a raw glob pattern string (for routing tuples).
fn glob_matches(pattern: &str, value: &str) -> bool {
    globset::Glob::new(pattern)
        .and_then(|g| {
            let mut b = GlobSetBuilder::new();
            b.add(g);
            b.build()
        })
        .map(|gs| gs.is_match(value))
        .unwrap_or(false)
}

/// Strip query string: `https://host/path?query=1` → `https://host/path`.
pub(crate) fn strip_url_query(url: &str) -> &str {
    match url.find('?') {
        Some(pos) => &url[..pos],
        None => url,
    }
}

// ── Denial counters ───────────────────────────────────────────────────────────

/// Per-deployment scope denial counters, keyed by category name.
#[derive(Debug, Default)]
pub(crate) struct ScopeDenialCounters {
    pub(crate) secrets: std::sync::atomic::AtomicU64,
    pub(crate) kv: std::sync::atomic::AtomicU64,
    pub(crate) vector: std::sync::atomic::AtomicU64,
    pub(crate) training: std::sync::atomic::AtomicU64,
    pub(crate) bridge: std::sync::atomic::AtomicU64,
    pub(crate) routing: std::sync::atomic::AtomicU64,
    pub(crate) http: std::sync::atomic::AtomicU64,
    pub(crate) outbox: std::sync::atomic::AtomicU64,
    pub(crate) storage: std::sync::atomic::AtomicU64,
    pub(crate) graph: std::sync::atomic::AtomicU64,
}

impl ScopeDenialCounters {
    pub(crate) fn increment(&self, category: ScopeCategory) {
        use std::sync::atomic::Ordering::Relaxed;
        match category {
            ScopeCategory::Secrets => self.secrets.fetch_add(1, Relaxed),
            ScopeCategory::Kv => self.kv.fetch_add(1, Relaxed),
            ScopeCategory::Vector => self.vector.fetch_add(1, Relaxed),
            ScopeCategory::Training => self.training.fetch_add(1, Relaxed),
            ScopeCategory::Bridge => self.bridge.fetch_add(1, Relaxed),
            ScopeCategory::Routing => self.routing.fetch_add(1, Relaxed),
            ScopeCategory::Http => self.http.fetch_add(1, Relaxed),
            ScopeCategory::Outbox => self.outbox.fetch_add(1, Relaxed),
            ScopeCategory::Storage => self.storage.fetch_add(1, Relaxed),
            ScopeCategory::Graph => self.graph.fetch_add(1, Relaxed),
        };
    }

    /// Increment the per-category atomic AND the global labeled prometheus counter.
    pub(crate) fn increment_with_deployment(&self, category: ScopeCategory, deployment: &str) {
        self.increment(category);
        record_scope_denial_prometheus(deployment, category);
    }

    #[allow(dead_code)]
    pub(crate) fn total_secrets(&self) -> u64 {
        self.secrets.load(std::sync::atomic::Ordering::Relaxed)
    }
}

// ── Global prometheus scope metrics ──────────────────────────────────────────

/// Lazily-registered `CounterVec` for per-deployment, per-category runtime denials.
/// Initialized once at startup; additional registration attempts are silently ignored.
static SCOPE_DENIAL_VEC: std::sync::OnceLock<Option<prometheus::CounterVec>> =
    std::sync::OnceLock::new();

/// Lazily-registered `CounterVec` for deployments that resolved to `allow-all` scopes.
static SCOPE_ALLOW_ALL_VEC: std::sync::OnceLock<Option<prometheus::CounterVec>> =
    std::sync::OnceLock::new();

fn scope_denial_vec() -> Option<&'static prometheus::CounterVec> {
    SCOPE_DENIAL_VEC
        .get_or_init(|| {
            let cv = prometheus::CounterVec::new(
                prometheus::Opts::new(
                    "faas_scope_denials_total",
                    "Total number of runtime WIT import denials, broken down by deployment and category.",
                ),
                &["deployment", "category"],
            )
            .ok()?;
            // Ignore "already registered" errors (hot-reload path).
            let _ = prometheus::default_registry().register(Box::new(cv.clone()));
            Some(cv)
        })
        .as_ref()
}

fn scope_allow_all_vec() -> Option<&'static prometheus::CounterVec> {
    SCOPE_ALLOW_ALL_VEC
        .get_or_init(|| {
            let cv = prometheus::CounterVec::new(
                prometheus::Opts::new(
                    "faas_scopes_allow_all_total",
                    "Total number of guest invocations whose deployment resolved to allow-all scopes.",
                ),
                &["deployment"],
            )
            .ok()?;
            let _ = prometheus::default_registry().register(Box::new(cv.clone()));
            Some(cv)
        })
        .as_ref()
}

// ── Global scope denial lifetime counter (task 6.3) ─────────────────────────

/// Lifetime total scope denials across all deployments and categories.
/// Exposed via `scope_denial_total_lifetime()` for operator-facing admin snapshots.
static SCOPE_DENIAL_LIFETIME: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Total scope denials recorded since process start, across all deployments and categories.
/// Suitable for surfacing in the admin `/admin/metrics` JSON response.
pub(crate) fn scope_denial_total_lifetime() -> u64 {
    SCOPE_DENIAL_LIFETIME.load(std::sync::atomic::Ordering::Relaxed)
}

// ── Per-deployment denial rate tracker (task 6.2) ────────────────────────────

/// Denial threshold before a sampled WARN is emitted (denials per 60-second window).
const DENIAL_WARN_THRESHOLD_PER_MIN: u64 = 100;

struct DenialRateState {
    window_start_secs: std::sync::atomic::AtomicU64,
    window_count: std::sync::atomic::AtomicU64,
    warned_this_window: std::sync::atomic::AtomicBool,
}

type DenialRateMap = std::sync::Mutex<std::collections::HashMap<String, Arc<DenialRateState>>>;

static DENIAL_RATE_MAP: std::sync::OnceLock<DenialRateMap> = std::sync::OnceLock::new();

fn denial_rate_map() -> &'static DenialRateMap {
    DENIAL_RATE_MAP.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Check the per-deployment denial rate; emit a sampled WARN if the threshold is crossed.
fn maybe_warn_denial_rate(deployment: &str, category: ScopeCategory) {
    use std::sync::atomic::Ordering::{AcqRel, Relaxed};

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let state = {
        let mut guard = match denial_rate_map().lock() {
            Ok(g) => g,
            Err(_) => return, // poisoned; skip rate-limiting silently
        };
        Arc::clone(guard.entry(deployment.to_owned()).or_insert_with(|| {
            Arc::new(DenialRateState {
                window_start_secs: std::sync::atomic::AtomicU64::new(now_secs),
                window_count: std::sync::atomic::AtomicU64::new(0),
                warned_this_window: std::sync::atomic::AtomicBool::new(false),
            })
        }))
    };

    // Reset window if the 60-second window has expired.
    let window_start = state.window_start_secs.load(Relaxed);
    if now_secs.saturating_sub(window_start) >= 60
        && state
            .window_start_secs
            .compare_exchange(window_start, now_secs, AcqRel, Relaxed)
            .is_ok()
    {
        state.window_count.store(0, Relaxed);
        state.warned_this_window.store(false, Relaxed);
    }

    let count = state.window_count.fetch_add(1, Relaxed) + 1;
    if count >= DENIAL_WARN_THRESHOLD_PER_MIN && !state.warned_this_window.swap(true, AcqRel) {
        tracing::warn!(
            deployment = %deployment,
            category = ?category,
            denials_per_min = count,
            "scope denial rate threshold crossed for deployment; check scopes configuration"
        );
    }
}

/// Record a runtime scope denial in the global prometheus counter.
/// Call alongside `ScopeDenialCounters::increment` with the route path as `deployment`.
pub(crate) fn record_scope_denial_prometheus(deployment: &str, category: ScopeCategory) {
    let category_str = match category {
        ScopeCategory::Secrets => "secrets",
        ScopeCategory::Kv => "kv",
        ScopeCategory::Vector => "vector",
        ScopeCategory::Training => "training",
        ScopeCategory::Bridge => "bridge",
        ScopeCategory::Routing => "routing",
        ScopeCategory::Http => "http",
        ScopeCategory::Outbox => "outbox",
        ScopeCategory::Storage => "storage",
        ScopeCategory::Graph => "graph",
    };
    if let Some(cv) = scope_denial_vec() {
        cv.with_label_values(&[deployment, category_str]).inc();
    }
    SCOPE_DENIAL_LIFETIME.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    maybe_warn_denial_rate(deployment, category);
}

/// Record that a guest invocation resolved to allow-all scopes.
/// Call once per invocation when `DeploymentScopes::allow_all` is true.
pub(crate) fn record_scope_allow_all_prometheus(deployment: &str) {
    if let Some(cv) = scope_allow_all_vec() {
        cv.with_label_values(&[deployment]).inc();
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(v: Value) -> Result<DeploymentScopes, ScopeManifestError> {
        DeploymentScopes::from_manifest(&v)
    }

    // ── Happy path ────────────────────────────────────────────────────────────

    #[test]
    fn allow_all_sentinel() {
        let scopes = parse(json!("allow-all")).unwrap();
        assert!(scopes.allow_all);
        assert!(scopes.check_secrets("anything"));
        assert!(scopes.check_kv("any/table"));
    }

    #[test]
    fn absent_key_means_none() {
        let scopes = parse(json!({})).unwrap();
        assert!(!scopes.allow_all);
        assert!(scopes.secrets.is_none());
        assert!(scopes.kv.is_none());
    }

    #[test]
    fn secrets_glob_match() {
        let scopes = parse(json!({ "secrets": ["db/prod/*"] })).unwrap();
        assert!(scopes.check_secrets("db/prod/password"));
        assert!(!scopes.check_secrets("auth/master"));
    }

    #[test]
    fn kv_glob_match() {
        let scopes = parse(json!({ "kv": ["tenant-a/*"] })).unwrap();
        assert!(scopes.check_kv("tenant-a/users"));
        assert!(!scopes.check_kv("tenant-b/users"));
    }

    #[test]
    fn routing_tuple_match_requires_both() {
        let scopes = parse(json!({ "routing": ["/api/* -> https://backend/*"] })).unwrap();
        assert!(scopes.check_routing("/api/v1", "https://backend/v1"));
        assert!(!scopes.check_routing("/api/v1", "https://evil.com/"));
        assert!(!scopes.check_routing("/admin", "https://backend/admin"));
    }

    #[test]
    fn http_strips_query_on_caller_side() {
        let scopes = parse(json!({ "http": ["https://api.example.com/v1/*"] })).unwrap();
        // caller strips query before passing to check_http
        let url = "https://api.example.com/v1/users?token=abc";
        let stripped = strip_url_query(url);
        assert_eq!(stripped, "https://api.example.com/v1/users");
        assert!(scopes.check_http(stripped));
    }

    #[test]
    fn empty_pattern_list_links_but_denies_all() {
        let scopes = parse(json!({ "kv": [] })).unwrap();
        assert!(scopes.kv.is_some(), "interface should be linked");
        assert!(!scopes.check_kv("any-table"), "no patterns → deny all");
    }

    // ── ScopeShape equivalence ────────────────────────────────────────────────

    #[test]
    fn scope_shape_equal_for_same_grants() {
        let a = parse(json!({ "secrets": ["db/*"] })).unwrap();
        let b = parse(json!({ "secrets": ["db/*"] })).unwrap();
        assert_eq!(ScopeShape::of(&a), ScopeShape::of(&b));
    }

    #[test]
    fn scope_shape_differs_for_different_categories() {
        let a = parse(json!({ "secrets": ["db/*"] })).unwrap();
        let b = parse(json!({ "kv": ["tenant/*"] })).unwrap();
        assert_ne!(ScopeShape::of(&a), ScopeShape::of(&b));
    }

    #[test]
    fn scope_shape_allow_all_differs_from_explicit() {
        let a = parse(json!("allow-all")).unwrap();
        let b = parse(json!({ "secrets": ["**"] })).unwrap();
        assert_ne!(ScopeShape::of(&a), ScopeShape::of(&b));
    }

    // ── Error paths ───────────────────────────────────────────────────────────

    #[test]
    fn unknown_category_key_is_rejected() {
        let err = parse(json!({ "secrest": ["db/*"] })).unwrap_err();
        assert!(
            matches!(err, ScopeManifestError::UnknownCategory { ref key } if key == "secrest"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn routing_entry_without_destination_is_rejected() {
        let err = parse(json!({ "routing": ["/api/*"] })).unwrap_err();
        assert!(
            matches!(
                err,
                ScopeManifestError::RoutingMissingDestination { index: 0 }
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn non_string_pattern_is_rejected() {
        let err = parse(json!({ "secrets": [42] })).unwrap_err();
        assert!(
            matches!(err, ScopeManifestError::NonStringPattern { .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn invalid_glob_is_rejected() {
        let err = parse(json!({ "secrets": ["[invalid"] })).unwrap_err();
        assert!(
            matches!(err, ScopeManifestError::InvalidGlob { .. }),
            "unexpected error: {err}"
        );
    }

    // ── ScopeShape grants ─────────────────────────────────────────────────────

    #[test]
    fn grants_returns_true_for_present_category() {
        let scopes = parse(json!({ "kv": ["tenant/*"] })).unwrap();
        let shape = ScopeShape::of(&scopes);
        assert!(shape.grants(ScopeCategory::Kv));
        assert!(!shape.grants(ScopeCategory::Bridge));
    }

    #[test]
    fn allow_all_grants_everything() {
        let scopes = parse(json!("allow-all")).unwrap();
        let shape = ScopeShape::of(&scopes);
        assert!(shape.grants(ScopeCategory::Bridge));
        assert!(shape.grants(ScopeCategory::Routing));
    }

    // ── LinkerCache ───────────────────────────────────────────────────────────

    #[test]
    fn linker_cache_hit_on_same_shape() {
        let cache: LinkerCache<u32> = LinkerCache::new(10);
        let scopes = parse(json!({ "secrets": ["db/*"] })).unwrap();
        let shape = ScopeShape::of(&scopes);
        let mut built = 0u32;
        let _ = cache.get_or_build("faas", &shape, || {
            built += 1;
            Ok::<u32, ()>(42u32)
        });
        let _ = cache.get_or_build("faas", &shape, || {
            built += 1;
            Ok::<u32, ()>(99u32)
        });
        assert_eq!(
            cache
                .counters
                .hits
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert_eq!(
            cache
                .counters
                .misses
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn linker_cache_miss_on_different_shape() {
        let cache: LinkerCache<u32> = LinkerCache::new(10);
        let a = ScopeShape::of(&parse(json!({ "secrets": ["db/*"] })).unwrap());
        let b = ScopeShape::of(&parse(json!({ "kv": ["t/*"] })).unwrap());
        let _ = cache.get_or_build("faas", &a, || Ok::<u32, ()>(1u32));
        let _ = cache.get_or_build("faas", &b, || Ok::<u32, ()>(2u32));
        assert_eq!(
            cache
                .counters
                .misses
                .load(std::sync::atomic::Ordering::Relaxed),
            2
        );
    }

    #[test]
    fn linker_cache_different_worlds_miss() {
        let cache: LinkerCache<u32> = LinkerCache::new(10);
        let shape = ScopeShape::of(&parse(json!({ "secrets": ["db/*"] })).unwrap());
        let _ = cache.get_or_build("faas", &shape, || Ok::<u32, ()>(1u32));
        let _ = cache.get_or_build("system", &shape, || Ok::<u32, ()>(2u32));
        assert_eq!(
            cache
                .counters
                .misses
                .load(std::sync::atomic::Ordering::Relaxed),
            2,
            "same shape for different worlds must not share a linker"
        );
    }

    #[test]
    fn linker_cache_concurrent_gets_return_same_arc() {
        let cache: Arc<LinkerCache<u32>> = Arc::new(LinkerCache::new(10));
        let shape = ScopeShape::of(&parse(json!({ "kv": ["t/*"] })).unwrap());
        // Build the entry once so all threads hit the cached path.
        let first = cache
            .get_or_build("faas", &shape, || Ok::<u32, ()>(42))
            .unwrap();
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let c = Arc::clone(&cache);
                let s = shape.clone();
                std::thread::spawn(move || {
                    c.get_or_build("faas", &s, || Ok::<u32, ()>(99)).unwrap()
                })
            })
            .collect();
        for h in handles {
            let got = h.join().expect("thread should not panic");
            assert!(
                Arc::ptr_eq(&first, &got),
                "concurrent gets must return the same Arc pointer"
            );
        }
    }

    #[test]
    fn linker_cache_lru_eviction_at_capacity() {
        // Insert 10 entries into a capacity-2 cache. After calling
        // `run_pending_tasks`, the moka cache must not exceed its capacity bound.
        // We verify boundedness rather than which specific entry was evicted,
        // because moka uses W-TinyLFU (not pure LRU) and eviction ordering is
        // probabilistic for entries with equal access frequency.
        const CAP: u64 = 2;
        let cache: LinkerCache<u32> = LinkerCache::new(CAP);
        // ScopeShape encodes which categories are granted (not the patterns),
        // so shapes must differ by which categories are present.
        let shape_manifests = [
            json!({ "secrets": ["*"] }),
            json!({ "kv": ["*"] }),
            json!({ "vector": ["*"] }),
            json!({ "training": ["*"] }),
            json!({ "bridge": ["*"] }),
            json!({ "http": ["*"] }),
            json!({ "outbox": ["*"] }),
            json!({ "storage": ["*"] }),
            json!({ "graph": ["*"] }),
            json!({ "secrets": ["*"], "kv": ["*"] }),
        ];
        let shapes: Vec<ScopeShape> = shape_manifests
            .iter()
            .map(|m| ScopeShape::of(&parse(m.clone()).unwrap()))
            .collect();

        for (i, shape) in shapes.iter().enumerate() {
            cache
                .get_or_build("faas", shape, || Ok::<u32, ()>(i as u32))
                .unwrap();
        }
        // Flush moka's internal eviction queue to materialise pending evictions.
        cache.run_pending_tasks_for_test();

        assert!(
            cache.entry_count() <= CAP,
            "cache must not grow past its capacity of {CAP}, got {}",
            cache.entry_count()
        );
        assert_eq!(
            cache
                .counters
                .misses
                .load(std::sync::atomic::Ordering::Relaxed),
            10,
            "all 10 inserts must be cache misses (never seen before)"
        );
    }

    // ── Task 10.3: multi-tenant cross-isolation smoke ─────────────────────────
    //
    // Simulates two FaaS deployments with disjoint KV scopes and verifies that
    // neither tenant can access the other's namespace — the same invariant that
    // a live E2E deploy would exercise.

    #[test]
    fn multitenant_kv_isolation_smoke() {
        // tenant-a is granted kv["tenant-a/*"], tenant-b is granted kv["tenant-b/*"].
        let scopes_a = parse(json!({ "kv": ["tenant-a/*"] })).unwrap();
        let scopes_b = parse(json!({ "kv": ["tenant-b/*"] })).unwrap();

        // Within-tenant access is allowed.
        assert!(
            scopes_a.check_kv("tenant-a/users"),
            "tenant-a may read tenant-a/users"
        );
        assert!(
            scopes_b.check_kv("tenant-b/orders"),
            "tenant-b may read tenant-b/orders"
        );

        // Cross-tenant access is denied.
        assert!(
            !scopes_a.check_kv("tenant-b/users"),
            "tenant-a must NOT read tenant-b/users"
        );
        assert!(
            !scopes_b.check_kv("tenant-a/orders"),
            "tenant-b must NOT read tenant-a/orders"
        );

        // Wildcard sibling is also denied.
        assert!(
            !scopes_a.check_kv("tenant-b/*"),
            "tenant-a must NOT access tenant-b wildcard"
        );
    }

    #[test]
    fn multitenant_denial_counter_increments_on_cross_access() {
        use std::sync::atomic::Ordering::Relaxed;

        let scopes_a = parse(json!({ "kv": ["tenant-a/*"] })).unwrap();
        let counters = ScopeDenialCounters::default();

        // Simulate the host calling check_kv and incrementing the counter on denial.
        if !scopes_a.check_kv("tenant-b/secret") {
            counters.increment(ScopeCategory::Kv);
        }
        if !scopes_a.check_kv("tenant-b/private") {
            counters.increment(ScopeCategory::Kv);
        }
        // Permitted access must not increment.
        if !scopes_a.check_kv("tenant-a/data") {
            counters.increment(ScopeCategory::Kv);
        }

        assert_eq!(
            counters.kv.load(Relaxed),
            2,
            "exactly two cross-tenant KV denials should be recorded"
        );
    }

    // ── Task 10.2: per-call overhead micro-bench ──────────────────────────────
    //
    // Runs 1 000 000 iterations of the hot-path scope check and asserts the
    // average per-call cost is under 100 ns.  GlobSet pattern matching on
    // pre-compiled sets is typically 5–20 ns; 100 ns gives 5× headroom for
    // slower CI environments.
    //
    // Guarded by `not(debug_assertions)`: unoptimised debug builds routinely
    // exceed 100 ns even for trivial GlobSet lookups, so the assertion is
    // only meaningful in release mode.

    #[test]
    #[cfg(not(debug_assertions))]
    fn scope_check_kv_avg_under_100ns() {
        let scopes = parse(json!({ "kv": ["tenant-a/*", "db/prod/*", "cache/**"] })).unwrap();
        let iters = 1_000_000_u64;
        let start = std::time::Instant::now();
        for _ in 0..iters {
            let _ = std::hint::black_box(scopes.check_kv("tenant-a/users"));
        }
        let elapsed = start.elapsed();
        let avg_ns = elapsed.as_nanos() as u64 / iters;
        assert!(
            avg_ns < 100,
            "check_kv average per-call overhead is {avg_ns} ns — exceeds 100 ns budget"
        );
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn scope_check_secrets_avg_under_100ns() {
        let scopes = parse(json!({ "secrets": ["db/prod/*", "auth/**", "cert/*"] })).unwrap();
        let iters = 1_000_000_u64;
        let start = std::time::Instant::now();
        for _ in 0..iters {
            let _ = std::hint::black_box(scopes.check_secrets("db/prod/password"));
        }
        let elapsed = start.elapsed();
        let avg_ns = elapsed.as_nanos() as u64 / iters;
        assert!(
            avg_ns < 100,
            "check_secrets average per-call overhead is {avg_ns} ns — exceeds 100 ns budget"
        );
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn scope_check_routing_avg_under_100ns() {
        let scopes =
            parse(json!({ "routing": ["/api/* -> https://backend/*", "/rpc/* -> grpc://svc/*"] }))
                .unwrap();
        let iters = 1_000_000_u64;
        let start = std::time::Instant::now();
        for _ in 0..iters {
            let _ = std::hint::black_box(
                scopes.check_routing("/api/v2/users", "https://backend/v2/users"),
            );
        }
        let elapsed = start.elapsed();
        let avg_ns = elapsed.as_nanos() as u64 / iters;
        assert!(
            avg_ns < 100,
            "check_routing average per-call overhead is {avg_ns} ns — exceeds 100 ns budget"
        );
    }
}
