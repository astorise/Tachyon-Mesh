use super::*;

pub(crate) fn integrity_manifest_path() -> PathBuf {
    std::env::var_os(INTEGRITY_MANIFEST_PATH_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("integrity.lock"))
}

pub(crate) fn build_app(state: AppState) -> Router {
    let app: Router<AppState> = Router::new();
    #[cfg(feature = "admin-plane")]
    let app = app.merge(admin_plane::authenticated_routes(state.clone()));
    let app = app
        .merge(bootstrap_routes())
        .route(
            "/auth/signup/validate-token",
            post(auth::validate_registration_token_handler),
        )
        .route("/auth/signup/stage", post(auth::stage_signup_handler))
        .route(
            "/auth/signup/finalize",
            post(auth::finalize_enrollment_handler),
        )
        .route("/auth/login/stage", post(auth::stage_login_handler))
        .route("/auth/login/finalize", post(auth::finalize_login_handler))
        .route(
            "/api/kv-cache/{model}/{key}",
            get(kv_cache::kv_cache_get_handler)
                .put(kv_cache::kv_cache_put_handler)
                .delete(kv_cache::kv_cache_delete_handler),
        )
        .route(
            "/auth/recovery/consume",
            post(auth::consume_recovery_code_handler),
        )
        .fallback(faas_handler)
        .layer(from_fn(hop_limit_middleware));

    let app = app.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        custom_domain_routing_middleware,
    ));

    #[cfg(feature = "rate-limit")]
    let app = app.layer(axum::middleware::from_fn_with_state(
        rate_limit::new_rate_limiter(),
        rate_limit::rate_limit_middleware,
    ));

    app.with_state(state)
}

pub(crate) fn should_sample_telemetry(sample_rate: f64) -> bool {
    sample_rate > 0.0 && rand::rng().random_bool(sample_rate.clamp(0.0, 1.0))
}

pub(crate) fn merge_fuel_samples(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

pub(crate) async fn enforce_distributed_rate_limit(
    state: &AppState,
    runtime: &Arc<RuntimeState>,
    route: &IntegrityRoute,
    headers: &HeaderMap,
) -> Option<(StatusCode, String)> {
    let policy = route.distributed_rate_limit.as_ref()?;
    let Some(limiter_route) = runtime
        .config
        .sealed_route(SYSTEM_DIST_LIMITER_ROUTE)
        .cloned()
    else {
        record_distributed_rate_limit_bypass(&route.path, "system route missing");
        return None;
    };

    let key = match distributed_rate_limit_key(policy, headers, &state.host_identity, &route.path) {
        Ok(key) => key,
        Err(message) => return Some((StatusCode::UNAUTHORIZED, message)),
    };
    let body = match serde_json::to_vec(&serde_json::json!({
        "key": key,
        "threshold": policy.threshold,
        "window_seconds": policy.window_seconds,
    })) {
        Ok(body) => Bytes::from(body),
        Err(error) => {
            record_distributed_rate_limit_bypass(&route.path, &format!("encode failed: {error}"));
            return None;
        }
    };
    let method = Method::POST;
    let uri = Uri::from_static("/system/dist-limiter/check");
    let limiter_headers = HeaderMap::new();
    let trailers = Vec::new();

    let result = tokio::time::timeout(
        DISTRIBUTED_RATE_LIMIT_TIMEOUT,
        Box::pin(execute_route_with_middleware(
            state,
            runtime,
            &limiter_route,
            &limiter_headers,
            &method,
            &uri,
            &body,
            &trailers,
            HopLimit(DEFAULT_HOP_LIMIT),
            None,
            false,
            None,
        )),
    )
    .await;

    match result {
        Ok(Ok(result)) => distributed_rate_limit_decision(route, result.response),
        Ok(Err((status, message))) => {
            record_distributed_rate_limit_bypass(
                &route.path,
                &format!("limiter route failed with {status}: {message}"),
            );
            None
        }
        Err(_) => {
            record_distributed_rate_limit_bypass(&route.path, "timeout");
            None
        }
    }
}

pub(crate) fn distributed_rate_limit_decision(
    route: &IntegrityRoute,
    response: GuestHttpResponse,
) -> Option<(StatusCode, String)> {
    if !response.status.is_success() {
        record_distributed_rate_limit_bypass(
            &route.path,
            &format!("limiter returned HTTP {}", response.status),
        );
        return None;
    }

    let value = match serde_json::from_slice::<Value>(&response.body) {
        Ok(value) => value,
        Err(error) => {
            record_distributed_rate_limit_bypass(
                &route.path,
                &format!("invalid limiter response: {error}"),
            );
            return None;
        }
    };

    if value
        .get("allowed")
        .and_then(Value::as_bool)
        .unwrap_or(true)
    {
        None
    } else {
        Some((
            StatusCode::TOO_MANY_REQUESTS,
            format!("distributed rate limit exceeded for route `{}`", route.path),
        ))
    }
}

pub(crate) fn distributed_rate_limit_key(
    policy: &DistributedRateLimitConfig,
    headers: &HeaderMap,
    host_identity: &HostIdentity,
    route_path: &str,
) -> std::result::Result<String, String> {
    let route = normalize_route_path(route_path);
    let ip = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
        .to_owned();

    match policy.scope {
        DistributedRateLimitScope::Ip => Ok(format!("ip:{ip}:{route}")),
        DistributedRateLimitScope::Tenant => {
            let claims = host_identity.verify_header(headers)?;
            let tenant = claims
                .tenant_id
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(claims.route_path);
            Ok(format!("tenant:{tenant}:{route}"))
        }
        DistributedRateLimitScope::Token => {
            let claims = host_identity.verify_header(headers)?;
            let token = claims
                .token_id
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(claims.route_path);
            Ok(format!("token:{token}:{route}"))
        }
    }
}

pub(crate) fn record_distributed_rate_limit_bypass(route_path: &str, reason: &str) {
    DISTRIBUTED_RATE_LIMIT_BYPASS_TOTAL.fetch_add(1, Ordering::Relaxed);
    tracing::warn!(
        route = %route_path,
        reason,
        "distributed rate limiter bypassed; falling back to local limiter"
    );
}

#[cfg(test)]
pub(crate) fn distributed_rate_limit_bypass_total() -> u64 {
    DISTRIBUTED_RATE_LIMIT_BYPASS_TOTAL.load(Ordering::Relaxed)
}

pub(crate) fn lora_training_queue() -> Arc<LoraTrainingQueue> {
    Arc::clone(LORA_TRAINING_QUEUE.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::channel();
        let statuses = Arc::new(Mutex::new(HashMap::new()));
        let worker_statuses = Arc::clone(&statuses);
        std::thread::Builder::new()
            .name("tachyon-lora-low-priority".to_owned())
            .spawn(move || run_lora_training_worker(receiver, worker_statuses))
            .expect("LoRA training worker should spawn");
        Arc::new(LoraTrainingQueue { sender, statuses })
    }))
}

pub(crate) fn ai_inference_jobs() -> Arc<Mutex<HashMap<String, AiInferenceJobStatus>>> {
    Arc::clone(AI_INFERENCE_JOBS.get_or_init(|| Arc::new(Mutex::new(HashMap::new()))))
}

pub(crate) fn enqueue_async_ai_inference_job(body: Bytes) -> Response {
    let id = format!("ai-{}", Uuid::new_v4().simple());
    let jobs = ai_inference_jobs();
    jobs.lock()
        .expect("AI inference job map should not be poisoned")
        .insert(id.clone(), AiInferenceJobStatus::Queued);
    let worker_jobs = Arc::clone(&jobs);
    let worker_id = id.clone();
    tokio::spawn(async move {
        update_ai_inference_status(&worker_jobs, &worker_id, AiInferenceJobStatus::Running);
        let output = format!(
            "generated:{}",
            String::from_utf8_lossy(&body)
                .chars()
                .take(256)
                .collect::<String>()
        );
        update_ai_inference_status(
            &worker_jobs,
            &worker_id,
            AiInferenceJobStatus::Completed { output },
        );
    });

    (
        StatusCode::ACCEPTED,
        [("content-type", "application/json")],
        format!(r#"{{"job_id":"{id}","status":"queued"}}"#),
    )
        .into_response()
}

pub(crate) fn ai_inference_job_status_response(id: &str) -> Response {
    let jobs = ai_inference_jobs();
    let Some(status) = jobs
        .lock()
        .expect("AI inference job map should not be poisoned")
        .get(id)
        .cloned()
    else {
        return (
            StatusCode::NOT_FOUND,
            format!("unknown AI inference job `{id}`"),
        )
            .into_response();
    };
    let body = match status {
        AiInferenceJobStatus::Queued => format!(r#"{{"job_id":"{id}","status":"queued"}}"#),
        AiInferenceJobStatus::Running => format!(r#"{{"job_id":"{id}","status":"running"}}"#),
        AiInferenceJobStatus::Completed { output } => serde_json::json!({
            "job_id": id,
            "status": "completed",
            "output": output,
        })
        .to_string(),
        AiInferenceJobStatus::Failed { message } => serde_json::json!({
            "job_id": id,
            "status": "failed",
            "error": message,
        })
        .to_string(),
    };
    (StatusCode::OK, [("content-type", "application/json")], body).into_response()
}

pub(crate) fn update_ai_inference_status(
    jobs: &Arc<Mutex<HashMap<String, AiInferenceJobStatus>>>,
    id: &str,
    status: AiInferenceJobStatus,
) {
    jobs.lock()
        .expect("AI inference job map should not be poisoned")
        .insert(id.to_owned(), status);
}

pub(crate) fn run_lora_training_worker(
    receiver: std::sync::mpsc::Receiver<LoraTrainingJob>,
    statuses: Arc<Mutex<HashMap<String, LoraTrainingJobStatus>>>,
) {
    while let Ok(job) = receiver.recv() {
        update_lora_training_status(
            &statuses,
            &job.id,
            LoraTrainingJobStatus::Running {
                step: 0,
                total: job.max_steps,
            },
        );
        let result = execute_lora_training_job(&job, &statuses);
        match result {
            Ok(path) => update_lora_training_status(
                &statuses,
                &job.id,
                LoraTrainingJobStatus::Completed { adapter_path: path },
            ),
            Err(error) => update_lora_training_status(
                &statuses,
                &job.id,
                LoraTrainingJobStatus::Failed {
                    message: format!("{error:#}"),
                },
            ),
        }
    }
}

pub(crate) fn execute_lora_training_job(
    job: &LoraTrainingJob,
    statuses: &Arc<Mutex<HashMap<String, LoraTrainingJobStatus>>>,
) -> Result<String> {
    let total = job.max_steps.max(1);
    for step in 1..=total.min(4) {
        update_lora_training_status(
            statuses,
            &job.id,
            LoraTrainingJobStatus::Running { step, total },
        );
        std::thread::sleep(Duration::from_millis(1));
    }

    let broker_root = std::env::var(MODEL_BROKER_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("tachyon_data"));
    let adapter_dir = broker_root.join("adapters");
    fs::create_dir_all(&adapter_dir).with_context(|| {
        format!(
            "failed to create adapter broker dir `{}`",
            adapter_dir.display()
        )
    })?;
    let sanitized = sanitize_lora_job_part(&job.id)?;
    let adapter_path = adapter_dir.join(format!("{sanitized}.safetensors"));
    let payload = serde_json::to_vec(&serde_json::json!({
        "format": "tachyon.mock-lora.safetensors",
        "tenant_id": job.tenant_id,
        "base_model": job.base_model,
        "dataset": {
            "volume": job.dataset_volume,
            "path": job.dataset_path,
            "split": job.dataset_split,
        },
        "rank": job.rank,
        "max_steps": job.max_steps,
        "seed": job.seed,
        "finops": {
            "cpu_fallback": true,
            "ram_spillover": true,
            "estimated_cpu_ms": u64::from(total) * 5,
            "estimated_ram_mb": u64::from(job.rank.max(1)) * 64,
        }
    }))
    .context("failed to serialize LoRA adapter artifact")?;
    fs::write(&adapter_path, payload)
        .with_context(|| format!("failed to write adapter `{}`", adapter_path.display()))?;
    Ok(adapter_path.display().to_string())
}

pub(crate) fn update_lora_training_status(
    statuses: &Arc<Mutex<HashMap<String, LoraTrainingJobStatus>>>,
    id: &str,
    status: LoraTrainingJobStatus,
) {
    statuses
        .lock()
        .expect("LoRA training status map should not be poisoned")
        .insert(id.to_owned(), status);
}

pub(crate) fn sanitize_lora_job_part(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || !trimmed
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(anyhow!("invalid LoRA job id `{value}`"));
    }
    Ok(trimmed.to_owned())
}

pub(crate) fn generate_traceparent() -> String {
    let trace_id = Uuid::new_v4().simple().to_string();
    let span_id = format!("{:016x}", rand::rng().random::<u64>());
    format!("00-{trace_id}-{span_id}-01")
}

pub(crate) fn encode_metering_batch(batch: Vec<String>) -> Bytes {
    let mut payload = batch.join("\n");
    if !payload.is_empty() {
        payload.push('\n');
    }
    Bytes::from(payload)
}

pub(crate) async fn export_metering_batch(
    state: &AppState,
    batch: Vec<String>,
) -> std::result::Result<(), String> {
    let runtime = state.runtime.load_full();
    let Some(route) = runtime.route_registry.sealed_route(SYSTEM_METERING_ROUTE) else {
        return Ok(());
    };

    let headers = HeaderMap::new();
    let method = Method::POST;
    let uri = Uri::from_static(SYSTEM_METERING_ROUTE);
    let body = encode_metering_batch(batch);
    let trailers = Vec::new();
    let result = execute_route_arc_with_middleware(
        state,
        &runtime,
        route,
        &headers,
        &method,
        &uri,
        &body,
        &trailers,
        HopLimit(DEFAULT_HOP_LIMIT),
        None,
        false,
        None,
    )
    .await
    .map_err(|(status, message)| format!("metering route failed with {status}: {message}"))?;

    if result.response.status.is_success() {
        Ok(())
    } else {
        Err(format!(
            "metering route returned HTTP {}",
            result.response.status
        ))
    }
}

pub(crate) fn spawn_metering_exporter(state: AppState, mut receiver: mpsc::Receiver<String>) {
    tokio::spawn(async move {
        let mut batch = Vec::with_capacity(TELEMETRY_EXPORT_BATCH_SIZE);
        let flush_deadline = tokio::time::sleep(TELEMETRY_EXPORT_FLUSH_INTERVAL);
        tokio::pin!(flush_deadline);

        loop {
            tokio::select! {
                received = receiver.recv() => {
                    match received {
                        Some(record) => {
                            batch.push(record);
                            while batch.len() < TELEMETRY_EXPORT_BATCH_SIZE {
                                match receiver.try_recv() {
                                    Ok(record) => batch.push(record),
                                    Err(mpsc::error::TryRecvError::Empty) => break,
                                    Err(mpsc::error::TryRecvError::Disconnected) => break,
                                }
                            }

                            if batch.len() >= TELEMETRY_EXPORT_BATCH_SIZE {
                                flush_metering_batch(&state, &mut batch).await;
                                let next_flush = next_metering_flush_deadline();
                                flush_deadline.as_mut().reset(next_flush);
                            }
                        }
                        None => {
                            flush_metering_batch(&state, &mut batch).await;
                            break;
                        }
                    }
                }
                () = &mut flush_deadline => {
                    flush_metering_batch(&state, &mut batch).await;
                    let next_flush = next_metering_flush_deadline();
                    flush_deadline.as_mut().reset(next_flush);
                }
            }
        }
    });
}

fn next_metering_flush_deadline() -> tokio::time::Instant {
    tokio::time::Instant::now() + TELEMETRY_EXPORT_FLUSH_INTERVAL
}

async fn flush_metering_batch(state: &AppState, batch: &mut Vec<String>) {
    if batch.is_empty() {
        return;
    }

    let pending = std::mem::take(batch);

    // Durably stash each record in the metering outbox before attempting the
    // HTTP export. If the host crashes between here and the export, the records
    // are recoverable on the next boot. On successful export, the entries are
    // removed; on failure, they remain and a later sweep can retry.
    //
    // The exporter is the in-memory aggregation owner for system-faas-metering:
    // records are accumulated off the request path and flushed either when the
    // batch fills or when the periodic flush interval expires.
    let outbox_keys = persist_metering_batch(state, &pending);

    match export_metering_batch(state, pending).await {
        Ok(()) => {
            for key in outbox_keys {
                if let Err(error) = state
                    .core_store
                    .delete(store::CoreStoreBucket::MeteringOutbox, &key)
                {
                    tracing::warn!("metering outbox cleanup for `{key}` failed: {error:#}");
                }
            }
        }
        Err(error) => {
            tracing::warn!("telemetry metering export failed; outbox entries retained: {error}",);
        }
    }
}

pub(crate) fn persist_metering_batch(state: &AppState, batch: &[String]) -> Vec<String> {
    let mut keys = Vec::with_capacity(batch.len());
    for record in batch {
        match state
            .core_store
            .append_outbox(store::CoreStoreBucket::MeteringOutbox, record.as_bytes())
        {
            Ok(key) => keys.push(key),
            Err(error) => {
                tracing::warn!("metering outbox persist failed: {error:#}");
            }
        }
    }
    keys
}

pub(crate) fn spawn_metering_outbox_retry_sweeper(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(METERING_OUTBOX_RETRY_INTERVAL);
        loop {
            interval.tick().await;
            match drain_metering_outbox_once(&state, METERING_OUTBOX_RETRY_BATCH_LIMIT).await {
                Ok(0) => {}
                Ok(drained) => {
                    tracing::debug!("metering outbox retry sweeper exported {drained} record(s)");
                }
                Err(error) => {
                    tracing::warn!("metering outbox retry sweep failed: {error}");
                }
            }
        }
    });
}

pub(crate) async fn drain_metering_outbox_once(
    state: &AppState,
    limit: usize,
) -> std::result::Result<usize, String> {
    if limit == 0 {
        return Ok(0);
    }

    let core_store = Arc::clone(&state.core_store);
    let rows = tokio::task::spawn_blocking(move || {
        core_store.peek_outbox(store::CoreStoreBucket::MeteringOutbox, limit)
    })
    .await
    .map_err(|error| format!("metering outbox peek task failed: {error}"))?
    .map_err(|error| format!("failed to peek metering outbox: {error:#}"))?;

    if rows.is_empty() {
        return Ok(0);
    }

    let mut keys = Vec::with_capacity(rows.len());
    let mut batch = Vec::with_capacity(rows.len());
    for (key, payload) in rows {
        let record = String::from_utf8(payload)
            .map_err(|error| format!("metering outbox entry `{key}` is not UTF-8: {error}"))?;
        keys.push(key);
        batch.push(record);
    }

    export_metering_batch(state, batch).await?;

    let drained = keys.len();
    for key in keys {
        if let Err(error) = state
            .core_store
            .delete(store::CoreStoreBucket::MeteringOutbox, &key)
        {
            tracing::warn!("metering outbox retry cleanup for `{key}` failed: {error:#}");
        }
    }

    Ok(drained)
}

pub(crate) fn spawn_async_log_exporter(
    state: AppState,
    mut receiver: mpsc::Receiver<AsyncLogEntry>,
) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    handle.spawn(async move {
        while let Some(first_entry) = receiver.recv().await {
            let mut batch = vec![first_entry];
            while batch.len() < LOG_BATCH_SIZE {
                match tokio::time::timeout(LOG_BATCH_FLUSH_INTERVAL, receiver.recv()).await {
                    Ok(Some(entry)) => batch.push(entry),
                    Ok(None) | Err(_) => break,
                }
            }

            if let Err(error) = export_log_batch(&state, batch).await {
                tracing::warn!("async guest log export failed: {error}");
            }
        }
    });
}

pub(crate) async fn export_log_batch(
    state: &AppState,
    batch: Vec<AsyncLogEntry>,
) -> std::result::Result<(), String> {
    let runtime = state.runtime.load_full();
    let Some(route) = runtime.route_registry.sealed_route(SYSTEM_LOGGER_ROUTE) else {
        return Ok(());
    };

    let headers = HeaderMap::new();
    let method = Method::POST;
    let uri = Uri::from_static(SYSTEM_LOGGER_ROUTE);
    let body = serde_json::to_vec(&batch)
        .map_err(|error| format!("failed to serialize log batch: {error}"))?;
    let trailers = Vec::new();
    let result = execute_route_arc_with_middleware(
        state,
        &runtime,
        route,
        &headers,
        &method,
        &uri,
        &Bytes::from(body),
        &trailers,
        HopLimit(DEFAULT_HOP_LIMIT),
        None,
        false,
        None,
    )
    .await
    .map_err(|(status, message)| format!("logger route failed with {status}: {message}"))?;

    if result.response.status.is_success() {
        Ok(())
    } else {
        Err(format!(
            "logger route returned unexpected status {}",
            result.response.status
        ))
    }
}

pub(crate) async fn hop_limit_middleware(
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let hop_limit = match resolve_incoming_hop_limit(req.headers()) {
        Ok(hop_limit) => hop_limit,
        Err(()) => return loop_detected_response(),
    };

    req.extensions_mut().insert(hop_limit);
    req.headers_mut()
        .insert(HOP_LIMIT_HEADER, hop_limit.as_header_value());

    next.run(req).await
}

pub(crate) async fn custom_domain_routing_middleware(
    State(state): State<AppState>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    // HTTP/1.1 carries the target host in the `Host` header, but HTTP/2 and
    // HTTP/3 move it to the `:authority` pseudo-header, which hyper surfaces on
    // the request URI rather than as a `host` header. Fall back to the URI
    // authority so custom-domain routing works regardless of protocol version
    // (otherwise an h2/h3 client hitting a custom domain misses its route and
    // 404s on `/`).
    let host = request_host(req.headers())
        .map(str::to_owned)
        .or_else(|| req.uri().host().map(str::to_owned));
    let Some(host) = host else {
        return next.run(req).await;
    };
    let runtime = state.runtime.load_full();
    let Some(route) = runtime.route_registry.route_for_domain(&host) else {
        return next.run(req).await;
    };
    let path = route_domain_request_path(&route, req.uri());
    let mut builder = Uri::builder();
    if let Some(scheme) = req.uri().scheme_str() {
        builder = builder.scheme(scheme);
    }
    if let Some(authority) = req.uri().authority().cloned() {
        builder = builder.authority(authority);
    }
    if let Ok(uri) = builder.path_and_query(path).build() {
        *req.uri_mut() = uri;
    }

    next.run(req).await
}

pub(crate) fn route_domain_request_path(route: &IntegrityRoute, uri: &Uri) -> String {
    let original_path = normalize_route_path(uri.path());
    // A custom domain mounts its guest at the domain root: `/` maps to the
    // route path and sub-paths are prefixed with it. Skip prefixing when the
    // request already targets the route path (e.g. an h2/h3 client that sends
    // the absolute route path in `:path`) so we don't double it up
    // (`/api/x` -> `/api/x/api/x`).
    let already_targets_route =
        original_path == route.path || original_path.starts_with(&format!("{}/", route.path));
    let path = if original_path == "/" {
        route.path.clone()
    } else if already_targets_route {
        original_path
    } else {
        format!("{}{}", route.path, original_path)
    };

    match uri.query() {
        Some(query) => format!("{path}?{query}"),
        None => path,
    }
}

pub(crate) fn request_host(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("host")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(':').next().unwrap_or(value))
}

pub(crate) fn header_map_to_guest_fields(headers: &HeaderMap) -> GuestHttpFields {
    headers
        .iter()
        .map(|(name, value)| {
            let value = value
                .to_str()
                .map(str::to_owned)
                .unwrap_or_else(|_| String::from_utf8_lossy(value.as_bytes()).into_owned());
            (name.as_str().to_owned(), value)
        })
        .collect()
}

pub(crate) fn guest_fields_to_header_map(
    fields: &GuestHttpFields,
    label: &str,
) -> std::result::Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    insert_guest_fields(&mut headers, fields, label)?;
    Ok(headers)
}

pub(crate) fn insert_guest_fields(
    target: &mut HeaderMap,
    fields: &GuestHttpFields,
    label: &str,
) -> std::result::Result<(), String> {
    for (name, value) in fields {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| format!("guest returned an invalid {label} name `{name}`: {error}"))?;
        let header_value = HeaderValue::from_str(value).map_err(|error| {
            format!("guest returned an invalid {label} value for `{name}`: {error}")
        })?;
        target.append(header_name, header_value);
    }

    Ok(())
}

pub(crate) fn build_guest_response(
    response: GuestHttpResponse,
    completion_guard: Option<RouteResponseGuard>,
) -> std::result::Result<Response, String> {
    let mut response_headers = HeaderMap::new();
    insert_guest_fields(&mut response_headers, &response.headers, "response header")?;

    let trailer_map = if response.trailers.is_empty() {
        None
    } else {
        let mut trailers = HeaderMap::new();
        insert_guest_fields(&mut trailers, &response.trailers, "response trailer")?;
        Some(trailers)
    };

    let mut built = Response::builder()
        .status(response.status)
        .body(Body::new(GuestResponseBody::new(
            response.body,
            trailer_map,
            completion_guard,
        )))
        .map_err(|error| format!("failed to construct guest HTTP response: {error}"))?;
    built.headers_mut().extend(response_headers);
    Ok(built)
}

pub(crate) fn guest_response_into_response(result: RouteExecutionResult) -> Response {
    match build_guest_response(result.response, result.completion_guard) {
        Ok(response) => response,
        Err(message) => (StatusCode::INTERNAL_SERVER_ERROR, message).into_response(),
    }
}

pub(crate) fn clone_headers_with_original_route(
    headers: &HeaderMap,
    route: &IntegrityRoute,
) -> HeaderMap {
    let mut cloned = headers.clone();
    if !cloned.contains_key(TACHYON_ORIGINAL_ROUTE_HEADER) {
        if let Ok(value) = HeaderValue::from_str(&route.path) {
            cloned.insert(TACHYON_ORIGINAL_ROUTE_HEADER, value);
        }
    }
    cloned
}

pub(crate) async fn forward_request_to_override(
    http_client: &Client,
    destination: &str,
    headers: &HeaderMap,
    method: &Method,
    body: &Bytes,
    hop_limit: HopLimit,
) -> std::result::Result<Response, (StatusCode, String)> {
    let mut request = http_client.request(method.clone(), destination);
    for (name, value) in headers {
        if name == "host" || name == "content-length" || name == "connection" {
            continue;
        }
        request = request.header(name, value);
    }
    request = request.header(HOP_LIMIT_HEADER, hop_limit.decremented().to_string());
    let response = request.body(body.clone()).send().await.map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            format!("route override forward to `{destination}` failed: {error}"),
        )
    })?;
    let status = response.status();
    let response_headers = response.headers().clone();
    let response_body = response.bytes().await.map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            format!("failed to read override response body from `{destination}`: {error}"),
        )
    })?;
    let mut built = Response::builder()
        .status(status)
        .body(Body::from(response_body))
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to construct override response: {error}"),
            )
        })?;
    for (name, value) in &response_headers {
        if name == "content-length" || name == "connection" || name == "transfer-encoding" {
            continue;
        }
        built.headers_mut().append(name.clone(), value.clone());
    }
    Ok(built)
}

pub(crate) async fn forward_request_to_override_as_guest_response(
    http_client: &Client,
    destination: &str,
    headers: &HeaderMap,
    method: &Method,
    body: &Bytes,
    hop_limit: HopLimit,
) -> std::result::Result<GuestHttpResponse, (StatusCode, String)> {
    let mut request = http_client.request(method.clone(), destination);
    for (name, value) in headers {
        if name == "host" || name == "content-length" || name == "connection" {
            continue;
        }
        request = request.header(name, value);
    }
    request = request.header(HOP_LIMIT_HEADER, hop_limit.decremented().to_string());
    let response = request.body(body.clone()).send().await.map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            format!("mesh-overlay forward to `{destination}` failed: {error}"),
        )
    })?;
    let status = response.status();
    let headers = header_map_to_guest_fields(response.headers());
    let body = response.bytes().await.map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            format!("failed to read mesh-overlay response body from `{destination}`: {error}"),
        )
    })?;
    Ok(GuestHttpResponse {
        status,
        headers,
        body,
        trailers: Vec::new(),
    })
}

pub(crate) fn requested_model_alias(
    route: &IntegrityRoute,
    headers: &HeaderMap,
    body: &Bytes,
) -> Option<String> {
    let header_alias = ["x-tachyon-model", "x-model-alias", "model-alias"]
        .into_iter()
        .find_map(|name| headers.get(name))
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    if let Some(alias) = header_alias {
        return resolve_requested_model_alias(route, Some(alias));
    }

    let body_alias = if route.models.is_empty() {
        None
    } else {
        serde_json::from_slice::<Value>(body)
            .ok()
            .and_then(|payload| {
                ["model", "model_alias", "alias"]
                    .into_iter()
                    .find_map(|key| payload.get(key).and_then(Value::as_str))
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
            })
    };

    resolve_requested_model_alias(route, body_alias)
}

fn resolve_requested_model_alias(route: &IntegrityRoute, alias: Option<String>) -> Option<String> {
    alias
        .filter(|alias| {
            route.models.is_empty()
                || route
                    .models
                    .iter()
                    .any(|binding| binding.alias.eq_ignore_ascii_case(alias))
        })
        .or_else(|| {
            if route.models.len() == 1 {
                route.models.first().map(|binding| binding.alias.clone())
            } else {
                None
            }
        })
}

#[cfg(test)]
mod requested_model_alias_tests {
    use super::*;

    fn model_binding(alias: &str) -> IntegrityModelBinding {
        IntegrityModelBinding {
            alias: alias.to_owned(),
            path: String::new(),
            device: ModelDevice::Cpu,
            qos: RouteQos::Standard,
            dynamic: true,
            hardware_strategy: HardwareStrategy::default(),
        }
    }

    #[test]
    fn skips_body_alias_for_routes_without_model_bindings() {
        let route = IntegrityRoute::user("/plain");
        let headers = HeaderMap::new();
        let body = Bytes::from_static(br#"{"model":"llama3"}"#);

        assert_eq!(requested_model_alias(&route, &headers, &body), None);
    }

    #[test]
    fn keeps_header_alias_available_for_routes_without_model_bindings() {
        let route = IntegrityRoute::user("/plain");
        let mut headers = HeaderMap::new();
        headers.insert("x-tachyon-model", HeaderValue::from_static("llama3"));
        let body = Bytes::from_static(br#"{"model":"ignored"}"#);

        assert_eq!(
            requested_model_alias(&route, &headers, &body).as_deref(),
            Some("llama3")
        );
    }

    #[test]
    fn reads_body_alias_by_reference_for_model_routes() {
        let mut route = IntegrityRoute::user("/ai");
        route.models = vec![model_binding("llama3"), model_binding("mistral")];
        let headers = HeaderMap::new();
        let body = Bytes::from_static(br#"{"model":"mistral","messages":[{"role":"user"}]}"#);

        assert_eq!(
            requested_model_alias(&route, &headers, &body).as_deref(),
            Some("mistral")
        );
    }
}

#[cfg(feature = "ai-inference")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RouteMeshQosProfile {
    pub(crate) accelerator: ai_inference::AcceleratorKind,
    pub(crate) qos: RouteQos,
}

#[cfg(feature = "ai-inference")]
pub(crate) fn route_mesh_qos_profile(
    route: &IntegrityRoute,
    requested_model: Option<&str>,
) -> Option<RouteMeshQosProfile> {
    let binding = requested_model
        .and_then(|alias| {
            route
                .models
                .iter()
                .find(|binding| binding.alias.eq_ignore_ascii_case(alias))
        })
        .or_else(|| route.models.first())?;
    // The lane the work actually queues on, not the device the binding
    // declares. An `openai:` upstream runs on no local accelerator and is
    // scheduled on `Network`; reading its declared `cpu`/`cuda` here made mesh
    // admission watch an idle local queue while the network queue filled, so a
    // saturated upstream never redirected a request to a healthier peer.
    let accelerator = if binding
        .path
        .trim()
        .starts_with(ai_inference::UPSTREAM_SCHEME)
    {
        ai_inference::AcceleratorKind::Network
    } else {
        ai_inference::AcceleratorKind::from_model_device(&binding.device)
    };
    Some(RouteMeshQosProfile {
        accelerator,
        qos: binding.qos,
    })
}

#[cfg(feature = "ai-inference")]
pub(crate) fn should_consult_mesh_qos_override(
    profile: RouteMeshQosProfile,
    local_load: u32,
) -> bool {
    match profile.qos {
        RouteQos::RealTime => local_load > 0,
        RouteQos::Standard => local_load >= 4,
        RouteQos::Batch => local_load >= 1_000,
    }
}

#[cfg(not(feature = "resiliency"))]
mod resiliency {
    use super::{execute_route_with_middleware_inner, RouteExecutionResult, RouteInvocation};
    use axum::http::StatusCode;
    use sysinfo::System;

    pub(crate) async fn execute_route_with_resiliency(
        invocation: RouteInvocation,
    ) -> std::result::Result<RouteExecutionResult, (StatusCode, String)> {
        execute_route_with_middleware_inner(&invocation).await
    }

    pub(crate) fn available_system_ram_bytes() -> u64 {
        let mut system = System::new();
        system.refresh_memory();
        system.available_memory()
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_route_override(
    state: &AppState,
    runtime: &Arc<RuntimeState>,
    route: &IntegrityRoute,
    headers: &HeaderMap,
    method: &Method,
    uri: &Uri,
    body: &Bytes,
    trailer_fields: &GuestHttpFields,
    hop_limit: HopLimit,
    trace_id: &str,
    sampled_execution: bool,
    destination: &str,
) -> (Response, Option<u64>) {
    if destination.starts_with("http://") || destination.starts_with("https://") {
        match forward_request_to_override(
            &state.http_client,
            destination,
            headers,
            method,
            body,
            hop_limit,
        )
        .await
        {
            Ok(response) => (response, None),
            Err((status, message)) => ((status, message).into_response(), None),
        }
    } else {
        let override_path = normalize_route_path(destination);
        match runtime.route_registry.sealed_route(&override_path) {
            Some(override_route) => {
                let override_headers = clone_headers_with_original_route(headers, route);
                match execute_route_arc_with_middleware(
                    state,
                    runtime,
                    override_route,
                    &override_headers,
                    method,
                    uri,
                    body,
                    trailer_fields,
                    hop_limit,
                    Some(trace_id),
                    sampled_execution,
                    None,
                )
                .await
                {
                    Ok(result) => {
                        let fuel_consumed = result.fuel_consumed;
                        (guest_response_into_response(result), fuel_consumed)
                    }
                    Err((status, message)) => ((status, message).into_response(), None),
                }
            }
            None => (
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!(
                        "route override for `{}` points to missing route `{override_path}`",
                        route.path
                    ),
                )
                    .into_response(),
                None,
            ),
        }
    }
}

pub(crate) async fn faas_handler(
    State(state): State<AppState>,
    Extension(hop_limit): Extension<HopLimit>,
    #[cfg(feature = "websockets")] ws: Result<
        WebSocketUpgrade,
        axum::extract::ws::rejection::WebSocketUpgradeRejection,
    >,
    request: AxumRequest,
) -> Response {
    let (parts, body) = request.into_parts();
    let headers = parts.headers;
    let method = parts.method;
    let uri = parts.uri;
    let collected = match body.collect().await {
        Ok(collected) => collected,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("failed to read request body: {error}"),
            )
                .into_response();
        }
    };
    let trailers = collected.trailers().cloned().unwrap_or_default();
    let body = collected.to_bytes();
    let trailer_fields = header_map_to_guest_fields(&trailers);
    let _active_request = telemetry::begin_request(&state.telemetry);
    let runtime = state.runtime.load_full();
    let normalized_path = normalize_route_path(uri.path());
    if is_reserved_system_path(&normalized_path) {
        return (
            StatusCode::NOT_FOUND,
            format!("system route `{normalized_path}` is not registered in core-host"),
        )
            .into_response();
    }
    if method == Method::POST && normalized_path == "/api/v1/generate" {
        return enqueue_async_ai_inference_job(body);
    }
    if method == Method::GET {
        if let Some(job_id) = normalized_path.strip_prefix("/api/v1/jobs/") {
            return ai_inference_job_status_response(job_id);
        }
    }
    let trace_id = Uuid::new_v4().to_string();
    let sampled_execution = normalized_path != SYSTEM_METERING_ROUTE
        && should_sample_telemetry(runtime.config.telemetry_sample_rate);
    let traceparent = sampled_execution.then(generate_traceparent);
    telemetry::record_event(
        &state.telemetry,
        TelemetryEvent::RequestStart {
            trace_id: trace_id.clone(),
            path: normalized_path.clone(),
            sampled: sampled_execution,
            traceparent: traceparent.clone(),
            timestamp: Instant::now(),
        },
    );

    let (response, fuel_consumed): (Response, Option<u64>) = match runtime
        .route_registry
        .sealed_route(&normalized_path)
    {
        None => (
            (
                StatusCode::NOT_FOUND,
                format!("route `{normalized_path}` is not sealed in `integrity.lock`"),
            )
                .into_response(),
            None,
        ),
        Some(route) => match select_route_target(&route, &headers) {
            Err(error) => (
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!(
                        "failed to resolve route target for `{}`: {error}",
                        route.path
                    ),
                )
                    .into_response(),
                None,
            ),
            Ok(selected_target) => {
                let requested_model = requested_model_alias(&route, &headers, &body);
                let required_capabilities =
                    Capabilities::from_mask(selected_target.required_capability_mask);
                let local_supports_target = state.host_capabilities.supports(required_capabilities);
                #[cfg(feature = "ai-inference")]
                let mesh_qos_destination = route_mesh_qos_profile(
                    &route,
                    requested_model.as_deref(),
                )
                .and_then(|profile| {
                    let tier_snapshot = runtime.ai_runtime.queue_tier_snapshot(profile.accelerator);
                    let local_queue_depth = match profile.qos {
                        RouteQos::RealTime => tier_snapshot.realtime,
                        RouteQos::Standard => tier_snapshot.standard,
                        RouteQos::Batch => tier_snapshot.batch,
                    };
                    should_consult_mesh_qos_override(profile, local_queue_depth).then(|| {
                        control_plane_override_destination(
                            state.route_overrides.as_ref(),
                            &state.peer_capabilities,
                            &format!(
                                "{MESH_QOS_OVERRIDE_PREFIX}{}",
                                normalize_route_path(&route.path)
                            ),
                            &headers,
                            selected_target.required_capability_mask,
                            requested_model.as_deref(),
                        )
                    })?
                });

                #[cfg(not(feature = "ai-inference"))]
                let mesh_qos_destination: Option<String> = None;

                if let Some(destination) = mesh_qos_destination.or_else(|| {
                    control_plane_override_destination(
                        state.route_overrides.as_ref(),
                        &state.peer_capabilities,
                        &route.path,
                        &headers,
                        selected_target.required_capability_mask,
                        requested_model.as_deref(),
                    )
                }) {
                    execute_route_override(
                        &state,
                        &runtime,
                        &route,
                        &headers,
                        &method,
                        &uri,
                        &body,
                        &trailer_fields,
                        hop_limit,
                        &trace_id,
                        sampled_execution,
                        &destination,
                    )
                    .await
                } else if !local_supports_target {
                    let missing = state
                        .host_capabilities
                        .missing_names(required_capabilities)
                        .join(", ");
                    (
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        format!(
                            "Missing Capability: route `{}` requires [{}] but no capable mesh peer is available",
                            route.path, missing
                        ),
                    )
                        .into_response(),
                    None,
                )
                } else {
                    #[cfg(feature = "websockets")]
                    {
                        if selected_target.websocket {
                            let status = if is_websocket_upgrade_request(&headers) {
                                StatusCode::BAD_REQUEST
                            } else {
                                StatusCode::UPGRADE_REQUIRED
                            };
                            match ws {
                                Ok(upgrade) => {
                                    let upgrade: WebSocketUpgrade = upgrade;
                                    let websocket_state = state.clone();
                                    let websocket_route = route.as_ref().clone();
                                    let websocket_module = selected_target.module.clone();
                                    let websocket_target = selected_target.module.clone();
                                    (
                                        upgrade
                                            .on_upgrade(move |socket| async move {
                                                if let Err(error) = handle_websocket_connection(
                                                    websocket_state,
                                                    websocket_route,
                                                    websocket_module,
                                                    socket,
                                                )
                                                .await
                                                {
                                                    tracing::warn!(
                                                        target = %websocket_target,
                                                        "WebSocket session failed: {error:#}"
                                                    );
                                                }
                                            })
                                            .into_response(),
                                        None,
                                    )
                                }
                                Err(_) => (
                                    (
                                        status,
                                        format!(
                                            "route `{}` requires a valid WebSocket upgrade request",
                                            route.path
                                        ),
                                    )
                                        .into_response(),
                                    None,
                                ),
                            }
                        } else if is_websocket_upgrade_request(&headers) {
                            (
                                (
                                    StatusCode::BAD_REQUEST,
                                    format!(
                                        "route `{}` is not configured for WebSocket upgrades",
                                        route.path
                                    ),
                                )
                                    .into_response(),
                                None,
                            )
                        } else {
                            #[cfg(feature = "ai-inference")]
                            if is_streaming_accept_request(&headers) {
                                let request = GuestRequest {
                                    method: method.to_string(),
                                    uri: uri.to_string(),
                                    headers: header_map_to_guest_fields(&headers),
                                    body: body.clone(),
                                    trailers: trailer_fields.clone(),
                                };
                                return match network::handle_streaming_http_request(
                                    state.clone(),
                                    Arc::clone(&runtime),
                                    Arc::clone(&route),
                                    selected_target.module.clone(),
                                    request,
                                )
                                .await
                                {
                                    Ok(response) => response,
                                    Err((status, message)) => (status, message).into_response(),
                                };
                            }
                            match execute_route_arc_with_middleware(
                                &state,
                                &runtime,
                                Arc::clone(&route),
                                &headers,
                                &method,
                                &uri,
                                &body,
                                &trailer_fields,
                                hop_limit,
                                Some(&trace_id),
                                sampled_execution,
                                Some(selected_target.module.as_str()),
                            )
                            .await
                            {
                                Ok(result) => {
                                    let fuel_consumed = result.fuel_consumed;
                                    (guest_response_into_response(result), fuel_consumed)
                                }
                                Err((status, message)) => ((status, message).into_response(), None),
                            }
                        }
                    }

                    #[cfg(not(feature = "websockets"))]
                    {
                        if selected_target.websocket {
                            let status = if is_websocket_upgrade_request(&headers) {
                                StatusCode::NOT_IMPLEMENTED
                            } else {
                                StatusCode::UPGRADE_REQUIRED
                            };
                            (
                            (
                                status,
                                format!(
                                    "route `{}` requires the `websockets` host feature to accept upgraded traffic",
                                    route.path
                                ),
                            )
                                .into_response(),
                            None,
                        )
                        } else if is_websocket_upgrade_request(&headers) {
                            (
                                (
                                    StatusCode::BAD_REQUEST,
                                    format!(
                                        "route `{}` is not configured for WebSocket upgrades",
                                        route.path
                                    ),
                                )
                                    .into_response(),
                                None,
                            )
                        } else {
                            #[cfg(feature = "ai-inference")]
                            if is_streaming_accept_request(&headers) {
                                let request = GuestRequest {
                                    method: method.to_string(),
                                    uri: uri.to_string(),
                                    headers: header_map_to_guest_fields(&headers),
                                    body: body.clone(),
                                    trailers: trailer_fields.clone(),
                                };
                                return match network::handle_streaming_http_request(
                                    state.clone(),
                                    Arc::clone(&runtime),
                                    Arc::clone(&route),
                                    selected_target.module.clone(),
                                    request,
                                )
                                .await
                                {
                                    Ok(response) => response,
                                    Err((status, message)) => (status, message).into_response(),
                                };
                            }
                            match execute_route_arc_with_middleware(
                                &state,
                                &runtime,
                                Arc::clone(&route),
                                &headers,
                                &method,
                                &uri,
                                &body,
                                &trailer_fields,
                                hop_limit,
                                Some(&trace_id),
                                sampled_execution,
                                Some(selected_target.module.as_str()),
                            )
                            .await
                            {
                                Ok(result) => {
                                    let fuel_consumed = result.fuel_consumed;
                                    (guest_response_into_response(result), fuel_consumed)
                                }
                                Err((status, message)) => ((status, message).into_response(), None),
                            }
                        }
                    }
                }
            }
        },
    };

    telemetry::record_event(
        &state.telemetry,
        TelemetryEvent::RequestEnd {
            trace_id,
            status: response.status().as_u16(),
            fuel_consumed,
            timestamp: Instant::now(),
        },
    );

    response
}

fn is_reserved_system_path(path: &str) -> bool {
    path.starts_with("/auth/") || path.starts_with("/admin/")
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_route_with_middleware(
    state: &AppState,
    runtime: &Arc<RuntimeState>,
    route: &IntegrityRoute,
    headers: &HeaderMap,
    method: &Method,
    uri: &Uri,
    body: &Bytes,
    trailers: &GuestHttpFields,
    hop_limit: HopLimit,
    trace_id: Option<&str>,
    sampled_execution: bool,
    selected_module: Option<&str>,
) -> std::result::Result<RouteExecutionResult, (StatusCode, String)> {
    execute_route_arc_with_middleware(
        state,
        runtime,
        Arc::new(route.clone()),
        headers,
        method,
        uri,
        body,
        trailers,
        hop_limit,
        trace_id,
        sampled_execution,
        selected_module,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_route_arc_with_middleware(
    state: &AppState,
    runtime: &Arc<RuntimeState>,
    route: Arc<IntegrityRoute>,
    headers: &HeaderMap,
    method: &Method,
    uri: &Uri,
    body: &Bytes,
    trailers: &GuestHttpFields,
    hop_limit: HopLimit,
    trace_id: Option<&str>,
    sampled_execution: bool,
    selected_module: Option<&str>,
) -> std::result::Result<RouteExecutionResult, (StatusCode, String)> {
    let invocation = RouteInvocation {
        state: state.clone(),
        runtime: Arc::clone(runtime),
        route,
        headers: headers.clone(),
        method: method.clone(),
        uri: uri.clone(),
        body: body.clone(),
        trailers: trailers.clone(),
        hop_limit,
        trace_id: trace_id.map(str::to_owned),
        sampled_execution,
        selected_module: selected_module.map(str::to_owned),
    };

    resiliency::execute_route_with_resiliency(invocation).await
}

pub(crate) async fn execute_route_with_middleware_inner(
    invocation: &RouteInvocation,
) -> std::result::Result<RouteExecutionResult, (StatusCode, String)> {
    let state = &invocation.state;
    let runtime = &invocation.runtime;
    let route = &invocation.route;
    let headers = &invocation.headers;
    let method = &invocation.method;
    let uri = &invocation.uri;
    let body = &invocation.body;
    let trailers = &invocation.trailers;
    let hop_limit = invocation.hop_limit;
    let trace_id = invocation.trace_id.as_deref();
    let sampled_execution = invocation.sampled_execution;
    let selected_module = invocation.selected_module.as_deref();
    let mut accumulated_fuel = None;

    if let Some(middleware_name) = route.middleware.as_deref() {
        let middleware_resolved = runtime
            .route_registry
            .resolve_named_route(middleware_name)
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
        let middleware_route = runtime
            .route_registry
            .sealed_route(&middleware_resolved.path)
            .ok_or_else(|| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!(
                        "route middleware `{middleware_name}` resolved to missing path `{}`",
                        middleware_resolved.path
                    ),
                )
            })?;
        let middleware_response = execute_route_request(
            state,
            runtime,
            &middleware_route,
            headers,
            method,
            uri,
            body,
            trailers,
            hop_limit,
            trace_id,
            sampled_execution,
            None,
        )
        .await?;
        if middleware_response.response.status != StatusCode::OK {
            return Ok(middleware_response);
        }
        accumulated_fuel = merge_fuel_samples(accumulated_fuel, middleware_response.fuel_consumed);
    }

    let mut result = execute_route_request(
        state,
        runtime,
        route,
        headers,
        method,
        uri,
        body,
        trailers,
        hop_limit,
        trace_id,
        sampled_execution,
        selected_module,
    )
    .await?;
    result.fuel_consumed = merge_fuel_samples(accumulated_fuel, result.fuel_consumed);
    spawn_shadow_traffic_task(
        state,
        runtime,
        route,
        headers,
        method,
        uri,
        body,
        trailers,
        &result.response,
        trace_id,
    );
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_shadow_traffic_task(
    state: &AppState,
    runtime: &Arc<RuntimeState>,
    route: &IntegrityRoute,
    headers: &HeaderMap,
    method: &Method,
    uri: &Uri,
    body: &Bytes,
    trailers: &GuestHttpFields,
    primary_response: &GuestHttpResponse,
    trace_id: Option<&str>,
) {
    let Some(shadow_target) = route.shadow_target.clone() else {
        return;
    };
    if route.path == SYSTEM_SHADOW_PROXY_ROUTE {
        return;
    }
    let Some(shadow_route) = runtime
        .route_registry
        .sealed_route(SYSTEM_SHADOW_PROXY_ROUTE)
    else {
        tracing::warn!(
            route = %route.path,
            "shadow_target configured but system-faas-shadow-proxy route is not sealed"
        );
        return;
    };
    let Ok(event) = serde_json::to_vec(&serde_json::json!({
        "route": route.path,
        "shadow_target": shadow_target,
        "method": method.as_str(),
        "uri": uri.to_string(),
        "headers": header_map_to_guest_fields(headers),
        "trailers": trailers,
        "body_hex": hex::encode(body),
        "primary_status": primary_response.status.as_u16(),
        "primary_headers": primary_response.headers,
        "primary_body_sha256": sha256_hex(&primary_response.body),
        "trace_id": trace_id,
    })) else {
        tracing::warn!(route = %route.path, "failed to encode shadow traffic event");
        return;
    };

    let state = state.clone();
    let runtime = Arc::clone(runtime);
    tokio::spawn(async move {
        let headers = HeaderMap::new();
        let method = Method::POST;
        let uri = Uri::from_static(SYSTEM_SHADOW_PROXY_ROUTE);
        if let Err((status, message)) = execute_route_arc_with_middleware(
            &state,
            &runtime,
            shadow_route,
            &headers,
            &method,
            &uri,
            &Bytes::from(event),
            &Vec::new(),
            HopLimit(DEFAULT_HOP_LIMIT),
            None,
            false,
            None,
        )
        .await
        {
            tracing::warn!(
                status = %status,
                error = %message,
                "shadow traffic dispatch failed"
            );
        }
    });
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(bytes))
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_route_request(
    state: &AppState,
    runtime: &Arc<RuntimeState>,
    route: &IntegrityRoute,
    headers: &HeaderMap,
    method: &Method,
    uri: &Uri,
    body: &Bytes,
    trailers: &GuestHttpFields,
    hop_limit: HopLimit,
    trace_id: Option<&str>,
    sampled_execution: bool,
    selected_module: Option<&str>,
) -> std::result::Result<RouteExecutionResult, (StatusCode, String)> {
    if route.role == RouteRole::System && should_shed_system_route(&state.telemetry) {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            format!("system route `{}` shed under load", route.path),
        ));
    }
    if let Some(rejection) = enforce_distributed_rate_limit(state, runtime, route, headers).await {
        return Err(rejection);
    }
    let selected_module = selected_module
        .map(str::to_owned)
        .map(Ok)
        .unwrap_or_else(|| {
            select_route_module(route, headers)
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))
        })?;

    // Canary fractional routing: if an active rollout exists for this route,
    // override the selected module probabilistically.
    let (selected_module, canary_tracking) = {
        let registry = canary_rollouts()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(rollout) = registry.get(&route.path) {
            let is_stepping = rollout
                .phase
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .eq(&CanaryPhase::Stepping);
            if is_stepping {
                let weight = rollout.weight_pct.load(Ordering::Relaxed);
                if weight > 0 && rand::rng().random_range(0u32..100) < weight {
                    (rollout.next_version.clone(), Some(Arc::clone(rollout)))
                } else {
                    (selected_module, None)
                }
            } else {
                (selected_module, None)
            }
        } else {
            (selected_module, None)
        }
    };

    if let Some(rejection) =
        enforce_resource_admission(state, route, headers, method, body, hop_limit, runtime).await?
    {
        return Ok(rejection);
    }

    // VRAM-aware admission: AI inference routes are gated on accelerator headroom.
    if !route.models.is_empty() {
        if let Some(rejection) = enforce_vram_admission(state, route) {
            return Ok(rejection);
        }
    }

    let semaphore = runtime
        .concurrency_limits
        .get(&route.path)
        .cloned()
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("route `{}` is missing a concurrency limiter", route.path),
            )
        })?;
    match acquire_route_permit(Arc::clone(&semaphore)).await {
        Ok(permit) => {
            let result = execute_route_request_with_acquired_permit(
                state,
                runtime,
                route,
                headers.clone(),
                method.clone(),
                uri.clone(),
                body.clone(),
                trailers.clone(),
                hop_limit,
                trace_id.map(str::to_owned),
                sampled_execution,
                selected_module,
                semaphore,
                permit,
            )
            .await;
            // Update per-rollout error counters for canary requests.
            if let Some(ref rollout) = canary_tracking {
                rollout.next_req_count.fetch_add(1, Ordering::Relaxed);
                let is_error = result
                    .as_ref()
                    .map_or(true, |r| r.response.status.as_u16() >= 500);
                if is_error {
                    rollout.next_err_count.fetch_add(1, Ordering::Relaxed);
                }
            }
            result
        }
        Err(RoutePermitError::Closed) => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            format!("route `{}` is currently unavailable", route.path),
        )),
        Err(RoutePermitError::TimedOut) => {
            if route.allow_overflow {
                let requested_model = requested_model_alias(route, headers, body);
                if let Some(destination) = control_plane_override_destination(
                    state.route_overrides.as_ref(),
                    &state.peer_capabilities,
                    &route.path,
                    headers,
                    select_route_target(route, headers)
                        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?
                        .required_capability_mask,
                    requested_model.as_deref(),
                ) {
                    let response = forward_request_to_override_as_guest_response(
                        &state.http_client,
                        &destination,
                        headers,
                        method,
                        body,
                        hop_limit,
                    )
                    .await?;
                    return Ok(RouteExecutionResult {
                        response,
                        fuel_consumed: None,
                        completion_guard: None,
                    });
                }
            }

            if state.memory_governor.pressure() == memory_governor::MemoryPressure::Critical {
                return Err((
                    StatusCode::SERVICE_UNAVAILABLE,
                    format!(
                        "route `{}` is saturated and global memory pressure is critical",
                        route.path
                    ),
                ));
            }

            let (receiver, buffered_tier) = state
                .buffered_requests
                .enqueue(BufferedRouteRequest {
                    route_path: route.path.clone(),
                    selected_module,
                    method: method.to_string(),
                    uri: uri.to_string(),
                    headers: header_map_to_guest_fields(headers),
                    body: body.to_vec(),
                    trailers: trailers.clone(),
                    hop_limit: hop_limit.0,
                    trace_id: trace_id.map(str::to_owned),
                    sampled_execution,
                })
                .map_err(|error| {
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        format!(
                            "route `{}` is saturated and buffering failed: {error}",
                            route.path
                        ),
                    )
                })?;
            match tokio::time::timeout(BUFFER_RESPONSE_WAIT_TIMEOUT, receiver).await {
                Ok(Ok(Ok(mut result))) => {
                    result.response.headers.push((
                        "x-tachyon-buffered".to_owned(),
                        match buffered_tier {
                            BufferedRequestTier::Ram => "ram",
                            BufferedRequestTier::Disk => "disk",
                        }
                        .to_owned(),
                    ));
                    Ok(result)
                }
                Ok(Ok(Err(error))) => Err(error),
                Ok(Err(_)) => Err((
                    StatusCode::SERVICE_UNAVAILABLE,
                    format!("route `{}` buffered request was canceled", route.path),
                )),
                Err(_) => Err((
                    StatusCode::SERVICE_UNAVAILABLE,
                    format!("route `{}` buffered request timed out", route.path),
                )),
            }
        }
    }
}

pub(crate) async fn enforce_resource_admission(
    state: &AppState,
    route: &IntegrityRoute,
    headers: &HeaderMap,
    method: &Method,
    body: &Bytes,
    hop_limit: HopLimit,
    runtime: &Arc<RuntimeState>,
) -> std::result::Result<Option<RouteExecutionResult>, (StatusCode, String)> {
    let Some(policy) = route.resource_policy.as_ref() else {
        return Ok(None);
    };
    let required_ram_bytes = policy.required_ram_bytes();
    if required_ram_bytes == 0 {
        return Ok(None);
    }
    let available_ram = resiliency::available_system_ram_bytes();
    if available_ram >= required_ram_bytes {
        return Ok(None);
    }

    if policy.admission_strategy == AdmissionStrategy::MeshRetry {
        let requested_model = requested_model_alias(route, headers, body);
        let target = select_route_target(route, headers)
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
        if let Some(destination) = control_plane_override_destination(
            state.route_overrides.as_ref(),
            &state.peer_capabilities,
            &route.path,
            headers,
            target.required_capability_mask,
            requested_model.as_deref(),
        ) {
            let response = forward_request_to_override_as_guest_response(
                &state.http_client,
                &destination,
                headers,
                method,
                body,
                hop_limit,
            )
            .await?;
            return Ok(Some(RouteExecutionResult {
                response,
                fuel_consumed: None,
                completion_guard: None,
            }));
        }
    }

    let mut response = GuestHttpResponse::new(
        StatusCode::SERVICE_UNAVAILABLE,
        format!(
            "route `{}` requires {} bytes of available RAM but only {} bytes are available",
            route.path, required_ram_bytes, available_ram
        ),
    );
    response.headers.push((
        "x-tachyon-reason".to_owned(),
        "Insufficient-Cluster-Resources".to_owned(),
    ));
    let _ = runtime;
    Ok(Some(RouteExecutionResult {
        response,
        fuel_consumed: None,
        completion_guard: None,
    }))
}

/// Returns a rejection `RouteExecutionResult` when VRAM pressure is critical
/// for routes that drive AI inference, or `None` to allow the request through.
///
/// * **High pressure (>80%)** logs a routing warning; request proceeds
///   normally and may be forwarded to a peer with more headroom by the existing
///   `MeshRetry` admission strategy.
/// * **Critical pressure (>90%)** returns HTTP 503 with
///   `Retry-After: 5` and `x-tachyon-reason: vram-saturated`.  The caller
///   should not queue the request locally because the bounded buffering path
///   in the `TimedOut` permit branch already provides local queuing.
pub(crate) fn enforce_vram_admission(
    state: &AppState,
    route: &IntegrityRoute,
) -> Option<RouteExecutionResult> {
    match state.memory_governor.vram_pressure() {
        memory_governor::MemoryPressure::Critical => {
            tracing::warn!(
                route = %route.path,
                vram_pct = state.memory_governor.vram_utilization_pct(),
                "VRAM critical: queuing inference request"
            );
            let mut response = GuestHttpResponse::new(
                StatusCode::SERVICE_UNAVAILABLE,
                format!(
                    "route `{}` inference deferred: VRAM utilization {}% exceeds critical threshold",
                    route.path,
                    state.memory_governor.vram_utilization_pct(),
                ),
            );
            response
                .headers
                .push(("retry-after".to_owned(), "5".to_owned()));
            response
                .headers
                .push(("x-tachyon-reason".to_owned(), "vram-saturated".to_owned()));
            Some(RouteExecutionResult {
                response,
                fuel_consumed: None,
                completion_guard: None,
            })
        }
        memory_governor::MemoryPressure::High => {
            tracing::debug!(
                route = %route.path,
                vram_pct = state.memory_governor.vram_utilization_pct(),
                "VRAM high: applying routing penalty"
            );
            None
        }
        memory_governor::MemoryPressure::Normal => None,
    }
}

impl ResourcePolicy {
    pub(crate) fn required_ram_bytes(&self) -> u64 {
        let from_gb = self
            .min_ram_gb
            .unwrap_or(0)
            .saturating_mul(1024)
            .saturating_mul(1024)
            .saturating_mul(1024);
        let from_mb = self
            .min_ram_mb
            .unwrap_or(0)
            .saturating_mul(1024)
            .saturating_mul(1024);
        from_gb.max(from_mb)
    }
}

#[derive(Debug, Serialize)]
struct EnarxTeeInvocation {
    module: String,
    route_path: String,
    method: String,
    uri: String,
    headers: GuestHttpFields,
    body: Vec<u8>,
    trailers: GuestHttpFields,
    trace_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EnarxTeeResponse {
    status: u16,
    #[serde(default)]
    headers: GuestHttpFields,
    #[serde(default)]
    body: Vec<u8>,
    #[serde(default)]
    trailers: GuestHttpFields,
    #[serde(default)]
    fuel_consumed: Option<u64>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_route_request_with_acquired_permit(
    state: &AppState,
    runtime: &Arc<RuntimeState>,
    route: &IntegrityRoute,
    headers: HeaderMap,
    method: Method,
    uri: Uri,
    body: Bytes,
    trailers: GuestHttpFields,
    _hop_limit: HopLimit,
    trace_id: Option<String>,
    sampled_execution: bool,
    selected_module: String,
    semaphore: Arc<RouteExecutionControl>,
    permit: OwnedSemaphorePermit,
) -> BufferedRouteResult {
    let _volume_leases = state
        .volume_manager
        .acquire_route_volumes(route, Arc::clone(&state.storage_broker))
        .await
        .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error))?;
    prepare_encrypted_route_volumes(route).map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            error.into_response(&runtime.config).1,
        )
    })?;
    let _permit = permit;
    if let Some(FaaSRuntime::Microvm {
        image,
        vcpus,
        memory_mb,
    }) = route.runtime.as_ref()
    {
        let runner = system_faas_microvm_runner::MicroVmRunner::new(
            system_faas_microvm_runner::MicroVmConfig {
                image: PathBuf::from(image),
                vcpus: *vcpus,
                memory_mb: *memory_mb,
                keep_warm: false,
                tap_device: None,
                vsock_cid: None,
                serial_path: None,
                snapshot_path: None,
            },
        )
        .map_err(|error| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("microvm runner rejected route `{}`: {error}", route.path),
            )
        })?;
        let invocation = system_faas_microvm_runner::MicroVmInvocation {
            module_id: selected_module.clone(),
            payload: serde_json::json!({
                "routePath": route.path,
                "method": method.as_str(),
                "uri": uri.to_string(),
                "headers": header_map_to_guest_fields(&headers),
                "bodyUtf8": String::from_utf8_lossy(&body),
                "trailers": trailers,
                "traceId": trace_id,
            }),
        };
        semaphore.active_requests.fetch_add(1, Ordering::SeqCst);
        let microvm_result = match runner.invoke(invocation).await {
            Ok(result) => result,
            Err(error) => {
                semaphore.active_requests.fetch_sub(1, Ordering::SeqCst);
                return Err((
                    StatusCode::BAD_GATEWAY,
                    format!(
                        "microvm runner failed for route `{}`: {error:#}",
                        route.path
                    ),
                ));
            }
        };
        let status = StatusCode::from_u16(u16::try_from(microvm_result.status).unwrap_or(500))
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let response_body = if microvm_result.stdout.is_empty() {
            microvm_result.stderr.clone()
        } else {
            microvm_result.stdout.clone()
        };
        let mut response = GuestHttpResponse::new(status, response_body);
        response
            .headers
            .push(("x-tachyon-runtime".to_owned(), "microvm".to_owned()));
        if !microvm_result.stderr.is_empty() {
            response
                .headers
                .push(("x-tachyon-microvm-stderr".to_owned(), microvm_result.stderr));
        }
        return Ok(RouteExecutionResult {
            response,
            fuel_consumed: None,
            completion_guard: Some(RouteResponseGuard {
                control: Arc::clone(&semaphore),
            }),
        });
    }
    let active_request_guard = semaphore.begin_request();
    let propagated_headers = extract_propagated_headers(&headers);
    let engine = if sampled_execution {
        runtime.metered_engine.clone()
    } else {
        runtime.engine.clone()
    };
    let request_config = runtime.config.clone();
    let response_config = runtime.config.clone();
    let tee_backend_label = route
        .requires_tee
        .then(|| {
            runtime
                .config
                .tee_backend
                .as_ref()
                .map(tee_backend_header_value)
        })
        .flatten();
    let concurrency_limits = Arc::clone(&runtime.concurrency_limits);
    let storage_broker = Arc::clone(&state.storage_broker);
    let telemetry_context = trace_id.as_ref().map(|trace_id| GuestTelemetryContext {
        handle: state.telemetry.clone(),
        trace_id: trace_id.clone(),
    });
    let runtime_telemetry = state.telemetry.clone();
    let secret_access = SecretAccess::from_route(route, &state.secrets_vault);
    let task_route = route.clone();
    let task_function_name = selected_module.clone();
    let task_propagated_headers = propagated_headers.clone();
    let task_request_headers = headers.clone();
    let task_host_identity = Arc::clone(&state.host_identity);
    let task_route_overrides = Arc::clone(&state.route_overrides);
    let task_host_load = Arc::clone(&state.host_load);
    let task_bridge_manager = Arc::clone(&state.bridge_manager);
    let task_async_log_sender = state.async_log_sender.clone();
    let task_instance_pool = Arc::clone(&runtime.instance_pool);
    let task_component_cache = Arc::clone(&runtime.component_cache);
    let task_component_instance_pre_cache = Arc::clone(&runtime.component_instance_pre_cache);
    let task_legacy_instance_pre_cache = Arc::clone(&runtime.legacy_instance_pre_cache);
    let task_linker_cache = Arc::clone(&runtime.linker_cache);
    let task_local_mesh_dispatch = LocalMeshDispatchContext {
        state: state.clone(),
        runtime: Arc::clone(runtime),
        handle: tokio::runtime::Handle::current(),
    };
    let route_requires_tee = route.requires_tee;
    #[cfg(feature = "ai-inference")]
    let task_ai_runtime = Arc::clone(&runtime.ai_runtime);
    let guest_request = GuestRequest {
        method: method.to_string(),
        uri: uri.to_string(),
        headers: header_map_to_guest_fields(&headers),
        body: body.clone(),
        trailers: trailers.clone(),
    };
    let _host_load_guard = HostLoadGuard::new(
        Arc::clone(&state.host_load),
        guest_memory_page_count(request_config.guest_memory_limit_bytes),
    );
    // Concurrency admission: block / reject / pass through based on the route's
    // declared policy. The guard MUST stay alive for the duration of the
    // invocation; it is moved into the spawn_blocking closure below.
    let admission = concurrency_admission::check(state, route).await;
    let admission_guard = match admission {
        concurrency_admission::AdmissionOutcome::Pass(guard) => Some(guard),
        concurrency_admission::AdmissionOutcome::Rejected(rejection) => {
            return Err(translate_admission_rejection(&route.path, rejection));
        }
    };
    let result = if route_requires_tee {
        let backend = runtime.config.tee_backend.clone().ok_or_else(|| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                format!(
                    "route `{}` requires a TEE backend but none is configured",
                    route.path
                ),
            )
        })?;
        match backend {
            TeeBackendConfig::LocalEnclave => {
                tokio::task::spawn_blocking(move || {
                    let _guard = admission_guard; // drop on closure exit releases the slot
                    let mut outcome = execute_guest(
                        &engine,
                        &task_function_name,
                        guest_request,
                        &task_route,
                        GuestExecutionContext::builder(
                            request_config,
                            sampled_execution,
                            runtime_telemetry,
                            task_async_log_sender,
                            secret_access,
                            task_request_headers,
                            task_host_identity,
                            storage_broker,
                            task_bridge_manager,
                            concurrency_limits,
                            task_propagated_headers,
                            task_route_overrides,
                            task_host_load,
                            #[cfg(feature = "ai-inference")]
                            task_ai_runtime,
                        )
                        .telemetry(telemetry_context)
                        .local_mesh_dispatch(Some(task_local_mesh_dispatch.clone()))
                        .component_cache(Some(task_component_cache))
                        .component_instance_pre_cache(Some(task_component_instance_pre_cache))
                        .legacy_instance_pre_cache(Some(task_legacy_instance_pre_cache))
                        .linker_cache(Some(task_linker_cache))
                        .build(),
                    )?;
                    annotate_tee_outcome(&mut outcome, "local-enclave");
                    Ok(outcome)
                })
                .await
                .map_err(|error| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("TEE guest execution task failed: {error}"),
                    )
                })?
            }
            TeeBackendConfig::Enarx { keep_endpoint } => {
                let _guard = admission_guard;
                invoke_enarx_tee_backend(
                    &state.http_client,
                    &keep_endpoint,
                    EnarxTeeInvocation {
                        module: selected_module.clone(),
                        route_path: route.path.clone(),
                        method: method.to_string(),
                        uri: uri.to_string(),
                        headers: header_map_to_guest_fields(&headers),
                        body: body.to_vec(),
                        trailers: trailers.clone(),
                        trace_id: trace_id.clone(),
                    },
                )
                .await
            }
        }
    } else {
        tokio::task::spawn_blocking(move || {
            let _guard = admission_guard; // drop on closure exit releases the slot
            execute_guest(
                &engine,
                &task_function_name,
                guest_request,
                &task_route,
                GuestExecutionContext::builder(
                    request_config,
                    sampled_execution,
                    runtime_telemetry,
                    task_async_log_sender,
                    secret_access,
                    task_request_headers,
                    task_host_identity,
                    storage_broker,
                    task_bridge_manager,
                    concurrency_limits,
                    task_propagated_headers,
                    task_route_overrides,
                    task_host_load,
                    #[cfg(feature = "ai-inference")]
                    task_ai_runtime,
                )
                .telemetry(telemetry_context)
                .local_mesh_dispatch(Some(task_local_mesh_dispatch))
                .instance_pool(Some(task_instance_pool))
                .component_cache(Some(task_component_cache))
                .component_instance_pre_cache(Some(task_component_instance_pre_cache))
                .legacy_instance_pre_cache(Some(task_legacy_instance_pre_cache))
                .linker_cache(Some(task_linker_cache))
                .build(),
            )
        })
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("guest execution task failed: {error}"),
            )
        })?
    };
    seal_encrypted_route_volumes(route).map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            error.into_response(&runtime.config).1,
        )
    })?;

    let (mut response, fuel_consumed) = match result {
        Ok(outcome) => match outcome.output {
            GuestExecutionOutput::Http(response) => (response, outcome.fuel_consumed),
            GuestExecutionOutput::LegacyStdout(stdout) => (
                GuestHttpResponse::new(StatusCode::OK, stdout),
                outcome.fuel_consumed,
            ),
        },
        Err(error) => {
            error.log_if_needed(&selected_module);
            let (status, message) = error.into_response(&response_config);
            return Err((status, message));
        }
    };

    if let Some(backend) = tee_backend_label {
        annotate_tee_response(&mut response, backend);
    }

    Ok(RouteExecutionResult {
        response,
        fuel_consumed,
        completion_guard: Some(active_request_guard.into_response_guard()),
    })
}

fn annotate_tee_outcome(outcome: &mut GuestExecutionOutcome, backend: &'static str) {
    if let GuestExecutionOutput::Http(response) = &mut outcome.output {
        annotate_tee_response(response, backend);
    }
}

fn tee_backend_header_value(backend: &TeeBackendConfig) -> &'static str {
    match backend {
        TeeBackendConfig::LocalEnclave => "local-enclave",
        TeeBackendConfig::Enarx { .. } => "enarx",
    }
}

fn annotate_tee_response(response: &mut GuestHttpResponse, backend: &'static str) {
    response
        .headers
        .retain(|(name, _)| name != "x-tachyon-runtime" && name != "x-tachyon-tee-backend");
    response
        .headers
        .push(("x-tachyon-runtime".to_owned(), format!("tee-{backend}")));
    response
        .headers
        .push(("x-tachyon-tee-backend".to_owned(), backend.to_owned()));
}

async fn invoke_enarx_tee_backend(
    client: &Client,
    keep_endpoint: &str,
    invocation: EnarxTeeInvocation,
) -> std::result::Result<GuestExecutionOutcome, ExecutionError> {
    let payload = serde_json::to_vec(&invocation).map_err(|error| {
        ExecutionError::Internal(format!(
            "failed to encode Enarx TEE backend `{keep_endpoint}` invocation: {error}"
        ))
    })?;
    let response = client
        .post(keep_endpoint)
        .header("content-type", "application/json")
        .body(payload)
        .send()
        .await
        .map_err(|error| {
            ExecutionError::Internal(format!(
                "failed to invoke Enarx TEE backend `{keep_endpoint}`: {error}"
            ))
        })?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(ExecutionError::Internal(format!(
            "Enarx TEE backend `{keep_endpoint}` returned {status}: {body}"
        )));
    }

    let response_body = response.bytes().await.map_err(|error| {
        ExecutionError::Internal(format!(
            "failed to read Enarx TEE backend `{keep_endpoint}` response: {error}"
        ))
    })?;
    let tee_response =
        serde_json::from_slice::<EnarxTeeResponse>(&response_body).map_err(|error| {
            ExecutionError::Internal(format!(
                "failed to decode Enarx TEE backend `{keep_endpoint}` response: {error}"
            ))
        })?;
    let status = StatusCode::from_u16(tee_response.status).map_err(|error| {
        ExecutionError::Internal(format!(
            "Enarx TEE backend `{keep_endpoint}` returned invalid status `{}`: {error}",
            tee_response.status
        ))
    })?;
    let mut guest_response = GuestHttpResponse {
        status,
        headers: tee_response.headers,
        body: Bytes::from(tee_response.body),
        trailers: tee_response.trailers,
    };
    annotate_tee_response(&mut guest_response, "enarx");
    Ok(GuestExecutionOutcome {
        output: GuestExecutionOutput::Http(guest_response),
        fuel_consumed: tee_response.fuel_consumed,
    })
}

/// Map an `AdmissionRejection` to the `(StatusCode, String)` error shape
/// expected by `BufferedRouteResult`. Includes an `X-Tachyon-Leader` hint
/// when applicable so clients can retry against the elected leader.
fn translate_admission_rejection(
    route_path: &str,
    rejection: concurrency_admission::AdmissionRejection,
) -> (StatusCode, String) {
    use concurrency_admission::AdmissionRejection::*;
    match rejection {
        NotLeader { leader_node } => (
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "route `{route_path}` is mesh-leader scoped; this node is not the leader{}",
                leader_node
                    .map(|n| format!(" (try `{n}`)"))
                    .unwrap_or_default()
            ),
        ),
        Conflict { reason } => (StatusCode::CONFLICT, reason),
        AdmissionPaused => (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("route `{route_path}` admission is paused by an in-progress backup drain"),
        ),
        Dropped => (
            StatusCode::NO_CONTENT,
            format!("route `{route_path}` invocation dropped per `on_conflict = drop` policy"),
        ),
    }
}

pub(crate) async fn execute_buffered_route_request(
    state: &AppState,
    runtime: &Arc<RuntimeState>,
    route: &IntegrityRoute,
    semaphore: Arc<RouteExecutionControl>,
    permit: OwnedSemaphorePermit,
    request: BufferedRouteRequest,
) -> BufferedRouteResult {
    let method = Method::from_bytes(request.method.as_bytes()).map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to decode buffered request method: {error}"),
        )
    })?;
    let uri = request.uri.parse::<Uri>().map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to decode buffered request URI: {error}"),
        )
    })?;
    let headers = guest_fields_to_header_map(&request.headers, "buffered request headers")
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    execute_route_request_with_acquired_permit(
        state,
        runtime,
        route,
        headers,
        method,
        uri,
        Bytes::from(request.body),
        request.trailers,
        HopLimit(request.hop_limit),
        request.trace_id,
        request.sampled_execution,
        request.selected_module,
        semaphore,
        permit,
    )
    .await
}

pub(crate) async fn acquire_route_permit(
    control: Arc<RouteExecutionControl>,
) -> std::result::Result<OwnedSemaphorePermit, RoutePermitError> {
    match Arc::clone(&control.semaphore).try_acquire_owned() {
        Ok(permit) => Ok(permit),
        Err(TryAcquireError::Closed) => Err(RoutePermitError::Closed),
        Err(TryAcquireError::NoPermits) => {
            control.pending_waiters.fetch_add(1, Ordering::SeqCst);
            let result = tokio::time::timeout(
                ROUTE_CONCURRENCY_WAIT_TIMEOUT,
                Arc::clone(&control.semaphore).acquire_owned(),
            )
            .await;
            control.pending_waiters.fetch_sub(1, Ordering::SeqCst);

            match result {
                Ok(Ok(permit)) => Ok(permit),
                Ok(Err(_)) => Err(RoutePermitError::Closed),
                Err(_) => Err(RoutePermitError::TimedOut),
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn try_dispatch_local_mesh_request(
    state: &AppState,
    runtime: &Arc<RuntimeState>,
    resolved_url: &str,
    method: &Method,
    headers: &HeaderMap,
    body: Bytes,
    trailers: GuestHttpFields,
    hop_limit: HopLimit,
) -> std::result::Result<LocalMeshDispatchAttempt, String> {
    let started_at = Instant::now();
    if hop_limit.0 <= 1 {
        return Ok(LocalMeshDispatchAttempt::Handled(GuestHttpResponse::new(
            StatusCode::LOOP_DETECTED,
            "mesh hop limit exhausted",
        )));
    }
    if env_flag(FORCE_MESH_TRANSPORT_ENV) {
        record_mesh_dispatch(
            MeshDispatchMode::InProcess,
            MeshDispatchReason::Remote,
            started_at.elapsed(),
        );
        return Ok(LocalMeshDispatchAttempt::Fallback(
            MeshDispatchReason::Remote,
        ));
    }

    let url = reqwest::Url::parse(resolved_url)
        .map_err(|error| format!("internal mesh target `{resolved_url}` is invalid: {error}"))?;
    let normalized_path = normalize_route_path(url.path());
    let Some(route) = runtime.route_registry.sealed_route(&normalized_path) else {
        record_mesh_dispatch(
            MeshDispatchMode::InProcess,
            MeshDispatchReason::Remote,
            started_at.elapsed(),
        );
        return Ok(LocalMeshDispatchAttempt::Fallback(
            MeshDispatchReason::Remote,
        ));
    };
    let selected_target = select_route_target(&route, &HeaderMap::new()).map_err(|error| {
        format!(
            "failed to resolve local mesh target `{}`: {error}",
            route.path
        )
    })?;
    if !state.host_capabilities.supports(Capabilities::from_mask(
        selected_target.required_capability_mask,
    )) {
        record_mesh_dispatch(
            MeshDispatchMode::InProcess,
            MeshDispatchReason::Remote,
            started_at.elapsed(),
        );
        return Ok(LocalMeshDispatchAttempt::Fallback(
            MeshDispatchReason::Remote,
        ));
    }
    let Some(semaphore) = runtime.concurrency_limits.get(&route.path) else {
        record_mesh_dispatch(
            MeshDispatchMode::InProcess,
            MeshDispatchReason::Remote,
            started_at.elapsed(),
        );
        return Ok(LocalMeshDispatchAttempt::Fallback(
            MeshDispatchReason::Remote,
        ));
    };
    let saturation_reason =
        if state.memory_governor.pressure() == memory_governor::MemoryPressure::Critical {
            Some(MeshDispatchReason::Pressure)
        } else if semaphore.semaphore.available_permits() == 0 {
            Some(MeshDispatchReason::Saturated)
        } else {
            None
        };

    if let Some(reason) = saturation_reason {
        // `network = overflow`: a saturated local route with `allow_overflow`
        // routes the hop to the least-pressured eligible mesh peer (the same
        // `control_plane_override_destination` lookup and mTLS transport the
        // `RoutePermitError::TimedOut` branch of
        // `execute_route_request_with_acquired_permit` already uses) instead
        // of looping back into this same node's UDS/TCP fast path. The local
        // queue remains the last resort when no peer is eligible.
        if route.allow_overflow {
            if let Some(response) = try_peer_overflow_dispatch(
                state,
                &route,
                selected_target.required_capability_mask,
                headers,
                method,
                &body,
                hop_limit,
                reason,
                started_at,
            )
            .await?
            {
                return Ok(LocalMeshDispatchAttempt::Handled(response));
            }
        }
        record_mesh_dispatch(MeshDispatchMode::InProcess, reason, started_at.elapsed());
        return Ok(LocalMeshDispatchAttempt::Fallback(reason));
    }

    let uri = append_query(&normalized_path, url.query())
        .parse::<Uri>()
        .map_err(|error| {
            format!("failed to build in-process mesh URI for `{resolved_url}`: {error}")
        })?;
    let result = Box::pin(execute_route_request(
        state,
        runtime,
        &route,
        headers,
        method,
        &uri,
        &body,
        &trailers,
        HopLimit(hop_limit.decremented()),
        None,
        false,
        Some(selected_target.module.as_str()),
    ))
    .await
    .map_err(|(status, message)| {
        format!("in-process mesh fetch to `{resolved_url}` failed with {status}: {message}")
    })?;

    record_mesh_dispatch(
        MeshDispatchMode::InProcess,
        MeshDispatchReason::Ok,
        started_at.elapsed(),
    );
    Ok(LocalMeshDispatchAttempt::Handled(result.response))
}

/// Resolves an eligible mesh peer for a saturated/pressured local route and
/// forwards the request to it, mirroring the overflow branch that
/// `execute_route_request_with_acquired_permit` already runs on
/// `RoutePermitError::TimedOut`. Returns `Ok(None)` when no eligible peer is
/// known, so the caller falls through to the local UDS/TCP queue.
#[allow(clippy::too_many_arguments)]
async fn try_peer_overflow_dispatch(
    state: &AppState,
    route: &IntegrityRoute,
    required_capability_mask: u64,
    headers: &HeaderMap,
    method: &Method,
    body: &Bytes,
    hop_limit: HopLimit,
    reason: MeshDispatchReason,
    started_at: Instant,
) -> std::result::Result<Option<GuestHttpResponse>, String> {
    let requested_model = requested_model_alias(route, headers, body);
    let Some(destination) = control_plane_override_destination(
        state.route_overrides.as_ref(),
        &state.peer_capabilities,
        &route.path,
        headers,
        required_capability_mask,
        requested_model.as_deref(),
    ) else {
        return Ok(None);
    };

    let response = forward_request_to_override_as_guest_response(
        &state.http_client,
        &destination,
        headers,
        method,
        body,
        hop_limit,
    )
    .await
    .map_err(|(status, message)| {
        format!("mesh peer overflow forward to `{destination}` failed with {status}: {message}")
    })?;

    record_mesh_dispatch(MeshDispatchMode::Peer, reason, started_at.elapsed());
    Ok(Some(response))
}

pub(crate) enum LocalMeshDispatchAttempt {
    Handled(GuestHttpResponse),
    Fallback(MeshDispatchReason),
}

pub(crate) fn select_route_module(
    route: &IntegrityRoute,
    headers: &HeaderMap,
) -> std::result::Result<String, String> {
    select_route_target_with_roll(route, headers, None).map(|target| target.module)
}

pub(crate) fn select_stream_route_module(
    route: &IntegrityRoute,
) -> std::result::Result<String, String> {
    if route.targets.is_empty() {
        return Ok(route.name.clone());
    }

    select_route_target_with_roll(route, &HeaderMap::new(), None)
        .map(|target| target.module)
        .or_else(|_| Ok(route.name.clone()))
}

pub(crate) fn select_route_target(
    route: &IntegrityRoute,
    headers: &HeaderMap,
) -> std::result::Result<SelectedRouteTarget, String> {
    select_route_target_with_roll(route, headers, None)
}

pub(crate) fn select_route_target_with_roll(
    route: &IntegrityRoute,
    headers: &HeaderMap,
    random_roll: Option<u64>,
) -> std::result::Result<SelectedRouteTarget, String> {
    if route.targets.is_empty() {
        let required_capabilities = default_route_capabilities();
        return Ok(SelectedRouteTarget {
            module: route.name.clone(),
            websocket: false,
            required_capability_mask: Capabilities::from_requirement_list(&required_capabilities)
                .map_err(|error| error.to_string())?
                .mask,
            required_capabilities,
        });
    }

    for target in &route.targets {
        if target
            .match_header
            .as_ref()
            .is_some_and(|matcher| request_header_matches(headers, matcher))
        {
            let required_capabilities = if target.requires.is_empty() {
                default_route_capabilities()
            } else {
                target.requires.clone()
            };
            return Ok(SelectedRouteTarget {
                module: target.module.clone(),
                websocket: target.websocket,
                required_capability_mask: Capabilities::from_requirement_list(
                    &required_capabilities,
                )
                .map_err(|error| error.to_string())?
                .mask,
                required_capabilities,
            });
        }
    }

    let total_weight = route
        .targets
        .iter()
        .map(|target| u64::from(target.weight))
        .sum::<u64>();
    if total_weight > 0 {
        let draw = match random_roll {
            Some(roll) => roll % total_weight,
            None => rand::rng().random_range(0..total_weight),
        };
        let mut cumulative_weight = 0_u64;
        for target in &route.targets {
            if target.weight == 0 {
                continue;
            }
            cumulative_weight = cumulative_weight.saturating_add(u64::from(target.weight));
            if draw < cumulative_weight {
                let required_capabilities = if target.requires.is_empty() {
                    default_route_capabilities()
                } else {
                    target.requires.clone()
                };
                return Ok(SelectedRouteTarget {
                    module: target.module.clone(),
                    websocket: target.websocket,
                    required_capability_mask: Capabilities::from_requirement_list(
                        &required_capabilities,
                    )
                    .map_err(|error| error.to_string())?
                    .mask,
                    required_capabilities,
                });
            }
        }
    }

    resolve_function_name(&route.path)
        .map(|module| SelectedRouteTarget {
            module,
            websocket: false,
            required_capability_mask: Capabilities::CORE_WASI,
            required_capabilities: default_route_capabilities(),
        })
        .ok_or_else(|| {
            format!(
                "route `{}` does not define a routable guest target",
                route.path
            )
        })
}

pub(crate) fn request_header_matches(headers: &HeaderMap, matcher: &HeaderMatch) -> bool {
    headers
        .get(matcher.name.as_str())
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .is_some_and(|value| value == matcher.value)
}

/// Returns true when the client requested a streaming SSE response
/// (`Accept: text/event-stream`). Used to route ai-inference requests through
/// the incremental body-flush path instead of the buffered one.
#[cfg(feature = "ai-inference")]
pub(crate) fn is_streaming_accept_request(headers: &HeaderMap) -> bool {
    headers
        .get(hyper::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("text/event-stream"))
        .unwrap_or(false)
}

pub(crate) fn is_websocket_upgrade_request(headers: &HeaderMap) -> bool {
    let connection_upgrade = headers
        .get("connection")
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .any(|segment| segment.eq_ignore_ascii_case("upgrade"))
        })
        .unwrap_or(false);
    let websocket_upgrade = headers
        .get("upgrade")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);

    connection_upgrade && websocket_upgrade
}

pub(crate) fn extract_propagated_headers(headers: &HeaderMap) -> Vec<PropagatedHeader> {
    let Some(value) = headers
        .get(TACHYON_COHORT_HEADER)
        .or_else(|| headers.get(COHORT_HEADER))
        .and_then(|header| header.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Vec::new();
    };

    vec![
        PropagatedHeader {
            name: COHORT_HEADER.to_owned(),
            value: value.to_owned(),
        },
        PropagatedHeader {
            name: TACHYON_COHORT_HEADER.to_owned(),
            value: value.to_owned(),
        },
    ]
}

pub(crate) fn resolve_incoming_hop_limit(headers: &HeaderMap) -> std::result::Result<HopLimit, ()> {
    let hop_limit = headers
        .get(HOP_LIMIT_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(DEFAULT_HOP_LIMIT);

    if hop_limit == 0 {
        Err(())
    } else {
        Ok(HopLimit(hop_limit))
    }
}

#[cfg(test)]
pub(crate) fn resolve_mesh_fetch_target(
    config: &IntegrityConfig,
    route_registry: &RouteRegistry,
    caller_route: &IntegrityRoute,
    target: &str,
) -> std::result::Result<String, String> {
    resolve_outbound_http_target(
        config,
        route_registry,
        caller_route,
        &reqwest::Method::GET,
        target,
    )
    .map(|resolved| resolved.url)
}

pub(crate) fn is_internal_mesh_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("tachyon") || host.eq_ignore_ascii_case("mesh")
}

pub(crate) fn append_query(path: &str, query: Option<&str>) -> String {
    match query {
        Some(query) if !query.is_empty() => format!("{path}?{query}"),
        _ => path.to_owned(),
    }
}

impl OutboundTargetKind {
    pub(crate) fn is_internal(&self) -> bool {
        matches!(self, Self::Internal)
    }
}

pub(crate) fn resolve_outbound_http_target(
    config: &IntegrityConfig,
    route_registry: &RouteRegistry,
    caller_route: &IntegrityRoute,
    method: &reqwest::Method,
    target: &str,
) -> std::result::Result<ResolvedOutboundTarget, String> {
    if target.starts_with('/') {
        return Ok(ResolvedOutboundTarget {
            url: format!("{}{}", internal_mesh_base_url(config)?, target),
            kind: OutboundTargetKind::Internal,
        });
    }

    if !(target.starts_with("http://") || target.starts_with("https://")) {
        return Err(format!(
            "mesh fetch target `{target}` must be an absolute URL or an absolute route path"
        ));
    }

    let url = reqwest::Url::parse(target)
        .map_err(|error| format!("mesh fetch target `{target}` is not a valid URL: {error}"))?;
    if !url.host_str().is_some_and(is_internal_mesh_host) {
        return resolve_direct_external_target(caller_route, target);
    }

    let normalized_path = normalize_route_path(url.path());
    let base_url = internal_mesh_base_url(config)?;
    if route_registry.by_path.contains_key(&normalized_path) {
        return Ok(ResolvedOutboundTarget {
            url: format!("{base_url}{}", append_query(&normalized_path, url.query())),
            kind: OutboundTargetKind::Internal,
        });
    }

    let path_segments = url
        .path_segments()
        .into_iter()
        .flatten()
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let Some(first_segment) = path_segments.first().copied() else {
        return Err(format!(
            "internal mesh target `{target}` must identify a sealed route path, resource alias, or a single dependency name"
        ));
    };
    let suffix = url
        .path()
        .strip_prefix(&format!("/{first_segment}"))
        .unwrap_or_default();
    if let Some(resource) = config.resources.get(first_segment) {
        return resolve_resource_alias(
            config,
            route_registry,
            resource,
            first_segment,
            suffix,
            url.query(),
            method,
        );
    }

    if path_segments.len() != 1 {
        return Err(format!(
            "internal mesh target `{target}` must identify a sealed route path, resource alias, or a single dependency name"
        ));
    }
    let dependency_name = path_segments[0];
    let resolved_route =
        route_registry.resolve_dependency_route(&caller_route.path, dependency_name)?;
    Ok(ResolvedOutboundTarget {
        url: format!(
            "{base_url}{}",
            append_query(&resolved_route.path, url.query())
        ),
        kind: OutboundTargetKind::Internal,
    })
}

pub(crate) fn resolve_direct_external_target(
    caller_route: &IntegrityRoute,
    target: &str,
) -> std::result::Result<ResolvedOutboundTarget, String> {
    if caller_route.role == RouteRole::System {
        return Ok(ResolvedOutboundTarget {
            url: target.to_owned(),
            kind: OutboundTargetKind::External,
        });
    }

    Err(format!(
        "route `{}` is not allowed to call raw external URLs; seal an external resource alias in `integrity.lock` and use `http://mesh/<alias>` instead",
        caller_route.path
    ))
}

pub(crate) fn resolve_resource_alias(
    config: &IntegrityConfig,
    route_registry: &RouteRegistry,
    resource: &IntegrityResource,
    resource_name: &str,
    suffix: &str,
    query: Option<&str>,
    method: &reqwest::Method,
) -> std::result::Result<ResolvedOutboundTarget, String> {
    match resource {
        IntegrityResource::Internal {
            target,
            version_constraint,
        } => {
            let base_path = resolve_internal_resource_target(
                route_registry,
                target,
                version_constraint.as_deref(),
            )?;
            Ok(ResolvedOutboundTarget {
                url: format!(
                    "{}{}",
                    internal_mesh_base_url(config)?,
                    append_query(&join_resource_path(&base_path, suffix), query)
                ),
                kind: OutboundTargetKind::Internal,
            })
        }
        IntegrityResource::External {
            target,
            allowed_methods,
        } => {
            if !allowed_methods
                .iter()
                .any(|allowed| allowed == method.as_str())
            {
                return Err(format!(
                    "sealed external resource `{resource_name}` does not allow HTTP method `{}`",
                    method.as_str()
                ));
            }
            Ok(ResolvedOutboundTarget {
                url: join_external_resource_url(target, suffix, query)?,
                kind: OutboundTargetKind::External,
            })
        }
    }
}

pub(crate) fn resolve_internal_resource_target(
    route_registry: &RouteRegistry,
    target: &str,
    version_constraint: Option<&str>,
) -> std::result::Result<String, String> {
    if target.starts_with('/') {
        let normalized = normalize_route_path(target);
        let route = route_registry.by_path.get(&normalized).ok_or_else(|| {
            format!("sealed resource target `{normalized}` does not match any sealed route")
        })?;
        if let Some(requirement) = version_constraint {
            let parsed = VersionReq::parse(requirement).map_err(|error| {
                format!("sealed resource version constraint `{requirement}` is invalid: {error}")
            })?;
            if !parsed.matches(&route.version) {
                return Err(format!(
                    "sealed resource target `{normalized}` does not satisfy version constraint `{requirement}`"
                ));
            }
        }
        return Ok(normalized);
    }

    let route_name = normalize_service_name(target)
        .map_err(|error| format!("sealed resource target `{target}` is invalid: {error}"))?;
    let route = if let Some(requirement) = version_constraint {
        let parsed = VersionReq::parse(requirement).map_err(|error| {
            format!("sealed resource version constraint `{requirement}` is invalid: {error}")
        })?;
        route_registry.resolve_named_route_matching(&route_name, &parsed)?
    } else {
        route_registry.resolve_named_route(&route_name)?
    };
    Ok(route.path.clone())
}

pub(crate) fn join_resource_path(base_path: &str, suffix: &str) -> String {
    if suffix.is_empty() || suffix == "/" {
        return base_path.to_owned();
    }
    format!("{}{}", base_path.trim_end_matches('/'), suffix)
}

pub(crate) fn join_external_resource_url(
    base_url: &str,
    suffix: &str,
    query: Option<&str>,
) -> std::result::Result<String, String> {
    let mut url = reqwest::Url::parse(base_url).map_err(|error| {
        format!("sealed external resource target `{base_url}` is not a valid URL: {error}")
    })?;
    let merged_path = join_resource_path(url.path(), suffix);
    url.set_path(&merged_path);
    if let Some(query) = query {
        url.set_query(Some(query));
    }
    Ok(url.to_string())
}

pub(crate) fn internal_mesh_base_url(
    config: &IntegrityConfig,
) -> std::result::Result<String, String> {
    let host_address = config.host_address.trim();
    if host_address.is_empty() {
        return Err(
            "mesh fetch cannot resolve a relative route without a configured host address"
                .to_owned(),
        );
    }

    if let Ok(socket_addr) = host_address.parse::<SocketAddr>() {
        return Ok(format!(
            "http://{}:{}",
            client_connect_host(socket_addr.ip()),
            socket_addr.port()
        ));
    }

    Ok(format!("http://{}", host_address.trim_end_matches('/')))
}

impl RouteRegistry {
    pub(crate) fn build(config: &IntegrityConfig) -> Result<Self> {
        let mut registry = Self::default();
        let mut seen_versions = HashMap::<(String, String), String>::new();

        for route in &config.routes {
            let sealed_route = Arc::new(route.clone());
            registry
                .sealed_by_path
                .insert(route.path.clone(), Arc::clone(&sealed_route));
            for domain in &route.domains {
                registry
                    .sealed_by_domain
                    .insert(domain.clone(), Arc::clone(&sealed_route));
            }

            let version = Version::parse(route.version.trim()).with_context(|| {
                format!(
                    "Integrity Validation Failed: route `{}` has invalid semantic version `{}`",
                    route.path, route.version
                )
            })?;
            let dependencies = route
                .dependencies
                .iter()
                .map(|(name, requirement)| {
                    VersionReq::parse(requirement.trim())
                        .map(|parsed| (name.clone(), parsed))
                        .map_err(|error| {
                            anyhow!(
                                "Integrity Validation Failed: route `{}` has invalid dependency requirement `{}` for `{}`: {}",
                                route.path,
                                requirement,
                                name,
                                error
                            )
                        })
                })
                .collect::<Result<HashMap<_, _>>>()?;

            let resolved = ResolvedRoute {
                path: route.path.clone(),
                name: route.name.clone(),
                version,
                dependencies,
                requires_credentials: route.requires_credentials.iter().cloned().collect(),
            };
            let version_text = resolved.version.to_string();
            if let Some(existing_path) = seen_versions.insert(
                (resolved.name.clone(), version_text.clone()),
                resolved.path.clone(),
            ) {
                return Err(anyhow!(
                    "Integrity Validation Failed: routes `{}` and `{}` both declare `{}` version `{}`",
                    existing_path,
                    resolved.path,
                    resolved.name,
                    version_text
                ));
            }

            registry
                .by_name
                .entry(resolved.name.clone())
                .or_default()
                .push(resolved.clone());
            registry.by_path.insert(resolved.path.clone(), resolved);
        }

        for routes in registry.by_name.values_mut() {
            routes.sort_by(|left, right| {
                right
                    .version
                    .cmp(&left.version)
                    .then_with(|| left.path.cmp(&right.path))
            });
        }

        for route in registry.by_path.values() {
            registry
                .ensure_dependencies_satisfied(route)
                .map_err(anyhow::Error::msg)?;
        }

        for route in &config.routes {
            if let Some(middleware) = &route.middleware {
                let resolved_middleware = registry
                    .resolve_named_route(middleware)
                    .map_err(anyhow::Error::msg)?;
                if resolved_middleware.path == route.path {
                    return Err(anyhow!(
                        "Integrity Validation Failed: route `{}` cannot use itself (`{}`) as middleware",
                        route.path,
                        middleware
                    ));
                }
            }
        }

        Ok(registry)
    }

    pub(crate) fn sealed_route(&self, path: &str) -> Option<Arc<IntegrityRoute>> {
        let normalized = normalize_route_path(path);
        self.sealed_by_path.get(&normalized).cloned()
    }

    pub(crate) fn route_for_domain(&self, domain: &str) -> Option<Arc<IntegrityRoute>> {
        let normalized = tls_runtime::normalize_domain(domain).ok()?;
        self.sealed_by_domain.get(&normalized).cloned()
    }

    pub(crate) fn ensure_dependencies_satisfied(
        &self,
        route: &ResolvedRoute,
    ) -> std::result::Result<(), String> {
        for (dependency_name, requirement) in &route.dependencies {
            let dependency =
                self.resolve_dependency_candidate(route, dependency_name, requirement)?;
            let missing_credentials = dependency
                .requires_credentials
                .difference(&route.requires_credentials)
                .cloned()
                .collect::<Vec<_>>();

            if !missing_credentials.is_empty() {
                return Err(format!(
                    "Credential delegation failed: route {} ({}@{}) must also declare {:?} to satisfy dependency {} ({}@{})",
                    route.path,
                    route.name,
                    route.version,
                    missing_credentials,
                    dependency.path,
                    dependency.name,
                    dependency.version
                ));
            }
        }

        Ok(())
    }

    pub(crate) fn resolve_dependency_route(
        &self,
        caller_path: &str,
        dependency_name: &str,
    ) -> std::result::Result<&ResolvedRoute, String> {
        let caller = self.by_path.get(caller_path).ok_or_else(|| {
            format!(
                "mesh fetch caller route `{caller_path}` is missing from the sealed dependency registry"
            )
        })?;
        let requirement = caller.dependencies.get(dependency_name).ok_or_else(|| {
            format!(
                "route {} ({}@{}) does not declare `{}` in its sealed dependencies",
                caller.path, caller.name, caller.version, dependency_name
            )
        })?;

        self.resolve_dependency_candidate(caller, dependency_name, requirement)
    }

    pub(crate) fn resolve_named_route(
        &self,
        route_name: &str,
    ) -> std::result::Result<&ResolvedRoute, String> {
        self.by_name
            .get(route_name)
            .and_then(|routes| routes.first())
            .ok_or_else(|| {
                format!("route middleware `{route_name}` does not match any sealed route name")
            })
    }

    pub(crate) fn resolve_named_route_matching(
        &self,
        route_name: &str,
        requirement: &VersionReq,
    ) -> std::result::Result<&ResolvedRoute, String> {
        self.by_name
            .get(route_name)
            .into_iter()
            .flatten()
            .find(|candidate| requirement.matches(&candidate.version))
            .ok_or_else(|| {
                format!(
                    "sealed resource `{route_name}` requires a route matching `{requirement}`, but no compatible version was loaded"
                )
            })
    }

    pub(crate) fn resolve_dependency_candidate(
        &self,
        caller: &ResolvedRoute,
        dependency_name: &str,
        requirement: &VersionReq,
    ) -> std::result::Result<&ResolvedRoute, String> {
        self.by_name
            .get(dependency_name)
            .into_iter()
            .flatten()
            .find(|candidate| requirement.matches(&candidate.version))
            .ok_or_else(|| {
                format!(
                    "Dependency resolution failed: route {} ({}@{}) requires {} matching {}, but no compatible version was loaded",
                    caller.path,
                    caller.name,
                    caller.version,
                    dependency_name,
                    requirement
                )
            })
    }
}

impl BatchTargetRegistry {
    pub(crate) fn build(config: &IntegrityConfig) -> Result<Self> {
        let mut registry = Self::default();
        for target in &config.batch_targets {
            if registry
                .by_name
                .insert(target.name.clone(), target.clone())
                .is_some()
            {
                return Err(anyhow!(
                    "Integrity Validation Failed: batch target `{}` is defined more than once",
                    target.name
                ));
            }
        }

        Ok(registry)
    }

    pub(crate) fn get(&self, name: &str) -> Option<&IntegrityBatchTarget> {
        self.by_name.get(name)
    }
}

pub(crate) fn client_connect_host(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(ip) if ip.is_unspecified() => Ipv4Addr::LOCALHOST.to_string(),
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) if ip.is_unspecified() => format!("[{}]", Ipv6Addr::LOCALHOST),
        IpAddr::V6(ip) => format!("[{ip}]"),
    }
}

#[cfg(unix)]
pub(crate) fn discovery_publish_ip(config: &IntegrityConfig) -> Result<String> {
    let host_address = config.host_address.trim();
    if host_address.is_empty() {
        return Err(anyhow!(
            "cannot publish a UDS fast-path endpoint without a configured host address"
        ));
    }

    if let Ok(socket_addr) = host_address.parse::<SocketAddr>() {
        return Ok(match socket_addr.ip() {
            IpAddr::V4(ip) if ip.is_unspecified() => Ipv4Addr::LOCALHOST.to_string(),
            IpAddr::V4(ip) => ip.to_string(),
            IpAddr::V6(ip) if ip.is_unspecified() => Ipv6Addr::LOCALHOST.to_string(),
            IpAddr::V6(ip) => ip.to_string(),
        });
    }

    let host = host_address
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/')
        .split('/')
        .next()
        .unwrap_or(host_address)
        .split(':')
        .next()
        .unwrap_or(host_address)
        .trim_matches('[')
        .trim_matches(']');
    if host.is_empty() {
        return Err(anyhow!(
            "cannot derive a publishable IP from host address `{host_address}`"
        ));
    }

    Ok(host.to_owned())
}

pub(crate) fn loop_detected_response() -> Response {
    (
        StatusCode::LOOP_DETECTED,
        "Tachyon Mesh: Routing loop detected (Hop limit exceeded)",
    )
        .into_response()
}

impl HopLimit {
    pub(crate) fn as_header_value(self) -> HeaderValue {
        HeaderValue::from_str(&self.0.to_string())
            .expect("hop limit should always produce a valid header value")
    }

    pub(crate) fn decremented(self) -> u32 {
        self.0.saturating_sub(1)
    }
}

#[cfg(test)]
mod tee_dispatch_tests {
    use super::*;

    #[test]
    fn tee_annotation_marks_http_outcomes_with_backend_headers() {
        let mut outcome = GuestExecutionOutcome {
            output: GuestExecutionOutput::Http(GuestHttpResponse::new(StatusCode::OK, "ok")),
            fuel_consumed: Some(7),
        };

        annotate_tee_outcome(&mut outcome, "local-enclave");

        let GuestExecutionOutput::Http(response) = outcome.output else {
            panic!("TEE annotation should preserve HTTP outcomes");
        };
        assert!(response
            .headers
            .iter()
            .any(|(name, value)| { name == "x-tachyon-runtime" && value == "tee-local-enclave" }));
        assert!(response
            .headers
            .iter()
            .any(|(name, value)| { name == "x-tachyon-tee-backend" && value == "local-enclave" }));
    }
}
