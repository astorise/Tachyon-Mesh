use super::*;

pub(crate) fn integrity_manifest_path() -> PathBuf {
    std::env::var_os(INTEGRITY_MANIFEST_PATH_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("integrity.lock"))
}

pub(crate) fn build_app(state: AppState) -> Router {
    let admin_routes = Router::new()
        .route("/admin/status", get(auth::admin_status_handler))
        .route("/admin/manifest", post(admin_manifest_update_handler))
        .route(
            "/admin/enrollment/start",
            post(admin_enrollment_start_handler),
        )
        .route(
            "/admin/enrollment/approve",
            post(admin_enrollment_approve_handler),
        )
        .route(
            "/admin/enrollment/poll/{session_id}",
            get(admin_enrollment_poll_handler),
        )
        .route(
            "/admin/security/recovery-codes",
            post(auth::generate_recovery_codes_handler),
        )
        .route(
            "/admin/security/2fa/regenerate",
            post(auth::regenerate_account_security_handler),
        )
        .route("/admin/security/pats", post(auth::issue_pat_handler))
        .route("/admin/assets", post(system_storage::upload_asset_handler))
        .route(
            "/admin/models/init",
            post(system_storage::init_upload_handler),
        )
        .route(
            "/admin/models/upload/{upload_id}",
            put(system_storage::upload_chunk_handler),
        )
        .route(
            "/admin/models/commit/{upload_id}",
            post(system_storage::commit_upload_handler),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::admin_auth_middleware,
        ));

    let app = Router::new()
        .merge(admin_routes)
        .route(
            "/auth/signup/validate-token",
            post(auth::validate_registration_token_handler),
        )
        .route("/auth/signup/stage", post(auth::stage_signup_handler))
        .route(
            "/auth/signup/finalize",
            post(auth::finalize_enrollment_handler),
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
    let Some(route) = runtime.config.sealed_route(SYSTEM_METERING_ROUTE).cloned() else {
        return Ok(());
    };

    let headers = HeaderMap::new();
    let method = Method::POST;
    let uri = Uri::from_static(SYSTEM_METERING_ROUTE);
    let body = encode_metering_batch(batch);
    let trailers = Vec::new();
    let result = execute_route_with_middleware(
        state,
        &runtime,
        &route,
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
        while let Some(first_record) = receiver.recv().await {
            let mut batch = vec![first_record];
            while batch.len() < TELEMETRY_EXPORT_BATCH_SIZE {
                match receiver.try_recv() {
                    Ok(record) => batch.push(record),
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => break,
                }
            }

            // Durably stash each record in the metering outbox before attempting the
            // HTTP export. If the host crashes between here and the export, the records
            // are recoverable on the next boot. On successful export, the entries are
            // removed; on failure, they remain and a later sweep can retry.
            //
            // This is the implementation of the `async-zero-blocking-metering` change:
            // the request critical path remains untouched (the original `mpsc` emit was
            // already async), but the durability story is now an explicit outbox table
            // rather than an in-memory channel that vanishes on a crash.
            let outbox_keys = persist_metering_batch(&state, &batch);

            match export_metering_batch(&state, batch).await {
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
                    tracing::warn!(
                        "telemetry metering export failed; outbox entries retained: {error}",
                    );
                }
            }
        }
    });
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
    let Some(route) = runtime.config.sealed_route(SYSTEM_LOGGER_ROUTE).cloned() else {
        return Ok(());
    };

    let headers = HeaderMap::new();
    let method = Method::POST;
    let uri = Uri::from_static(SYSTEM_LOGGER_ROUTE);
    let body = serde_json::to_vec(&batch)
        .map_err(|error| format!("failed to serialize log batch: {error}"))?;
    let trailers = Vec::new();
    let result = execute_route_with_middleware(
        state,
        &runtime,
        &route,
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
    let Some(host) = request_host(req.headers()) else {
        return next.run(req).await;
    };
    let runtime = state.runtime.load_full();
    let Some(route) = runtime.config.route_for_domain(host) else {
        return next.run(req).await;
    };
    let path = route_domain_request_path(route, req.uri());
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
    let path = if original_path == "/" {
        route.path.clone()
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

    let body_alias = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|payload| payload.as_object().cloned())
        .and_then(|payload| {
            ["model", "model_alias", "alias"]
                .into_iter()
                .find_map(|key| payload.get(key).and_then(Value::as_str))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        });

    header_alias
        .or(body_alias)
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
    Some(RouteMeshQosProfile {
        accelerator: ai_inference::AcceleratorKind::from_model_device(&binding.device),
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
        match runtime.config.sealed_route(&override_path).cloned() {
            Some(override_route) => {
                let override_headers = clone_headers_with_original_route(headers, route);
                match execute_route_with_middleware(
                    state,
                    runtime,
                    &override_route,
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
        .config
        .sealed_route(&normalized_path)
        .cloned()
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
                                    let websocket_route = route.clone();
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
                            match execute_route_with_middleware(
                                &state,
                                &runtime,
                                &route,
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
                            match execute_route_with_middleware(
                                &state,
                                &runtime,
                                &route,
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
    let invocation = RouteInvocation {
        state: state.clone(),
        runtime: Arc::clone(runtime),
        route: route.clone(),
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
            .config
            .sealed_route(&middleware_resolved.path)
            .cloned()
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
        .config
        .sealed_route(SYSTEM_SHADOW_PROXY_ROUTE)
        .cloned()
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
        if let Err((status, message)) = execute_route_with_middleware(
            &state,
            &runtime,
            &shadow_route,
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

    if let Some(rejection) =
        enforce_resource_admission(state, route, headers, method, body, hop_limit, runtime).await?
    {
        return Ok(rejection);
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
            execute_route_request_with_acquired_permit(
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
            .await
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
    hop_limit: HopLimit,
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
    let route_registry = Arc::clone(&runtime.route_registry);
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
    let result = tokio::task::spawn_blocking(move || {
        execute_guest(
            &engine,
            &task_function_name,
            guest_request,
            &task_route,
            GuestExecutionContext {
                config: request_config,
                sampled_execution,
                runtime_telemetry,
                async_log_sender: task_async_log_sender,
                secret_access,
                request_headers: task_request_headers,
                host_identity: task_host_identity,
                storage_broker,
                bridge_manager: task_bridge_manager,
                telemetry: telemetry_context,
                concurrency_limits,
                propagated_headers: task_propagated_headers,
                route_overrides: task_route_overrides,
                host_load: task_host_load,
                #[cfg(feature = "ai-inference")]
                ai_runtime: task_ai_runtime,
                instance_pool: if route_requires_tee {
                    None
                } else {
                    Some(task_instance_pool)
                },
            },
        )
    })
    .await
    .map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("guest execution task failed: {error}"),
        )
    })?;
    seal_encrypted_route_volumes(route).map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            error.into_response(&runtime.config).1,
        )
    })?;

    let (response, fuel_consumed) = match result {
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

    let response = resolve_mesh_response(
        &state.http_client,
        &response_config,
        &route_registry,
        route,
        &state.host_identity,
        &state.uds_fast_path,
        hop_limit,
        &propagated_headers,
        response,
    )
    .await
    .map_err(|error| (StatusCode::BAD_GATEWAY, error))?;

    Ok(RouteExecutionResult {
        response,
        fuel_consumed,
        completion_guard: Some(active_request_guard.into_response_guard()),
    })
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
pub(crate) async fn resolve_mesh_response(
    http_client: &Client,
    config: &IntegrityConfig,
    route_registry: &RouteRegistry,
    caller_route: &IntegrityRoute,
    host_identity: &HostIdentity,
    uds_fast_path: &UdsFastPathRegistry,
    hop_limit: HopLimit,
    propagated_headers: &[PropagatedHeader],
    response: GuestHttpResponse,
) -> std::result::Result<GuestHttpResponse, String> {
    let Some(target) = extract_mesh_fetch_url(&response.body) else {
        return Ok(response);
    };
    let resolved_target = resolve_outbound_http_target(
        config,
        route_registry,
        caller_route,
        &reqwest::Method::GET,
        target,
    )?;
    let url = rewrite_outbound_http_url(&resolved_target.url, config);
    let inject_identity = resolved_target.kind.is_internal();
    let identity_token = if inject_identity {
        Some(
            host_identity
                .sign_route(caller_route)
                .map_err(|error| format!("failed to sign mesh caller identity: {error:#}"))?,
        )
    } else {
        None
    };
    let response = send_mesh_fetch_request(
        http_client,
        uds_fast_path,
        &url,
        &resolved_target.kind,
        hop_limit,
        propagated_headers,
        identity_token.as_deref(),
    )
    .await?;

    let status = response.status();
    let headers = header_map_to_guest_fields(response.headers());
    let body = response.bytes().await.map_err(|error| {
        format!("failed to read mesh fetch response body from `{url}`: {error}")
    })?;

    if status == StatusCode::LOOP_DETECTED || status.is_success() {
        Ok(GuestHttpResponse {
            status,
            headers,
            body,
            trailers: Vec::new(),
        })
    } else {
        Err(format!(
            "mesh fetch to `{url}` returned an error status: {status}"
        ))
    }
}

pub(crate) fn apply_mesh_fetch_headers(
    mut request: reqwest::RequestBuilder,
    target_kind: &OutboundTargetKind,
    hop_limit: HopLimit,
    propagated_headers: &[PropagatedHeader],
    identity_token: Option<&str>,
) -> reqwest::RequestBuilder {
    if !target_kind.is_internal() {
        return request;
    }

    request = request.header(HOP_LIMIT_HEADER, hop_limit.decremented().to_string());
    for header in propagated_headers
        .iter()
        .filter(|header| !header.name.eq_ignore_ascii_case(TACHYON_IDENTITY_HEADER))
    {
        request = request.header(&header.name, &header.value);
    }
    if let Some(identity_token) = identity_token {
        request = request.header(TACHYON_IDENTITY_HEADER, format!("Bearer {identity_token}"));
    }

    request
}

pub(crate) async fn send_mesh_fetch_request(
    http_client: &Client,
    _uds_fast_path: &UdsFastPathRegistry,
    url: &str,
    target_kind: &OutboundTargetKind,
    hop_limit: HopLimit,
    propagated_headers: &[PropagatedHeader],
    identity_token: Option<&str>,
) -> std::result::Result<reqwest::Response, String> {
    #[cfg(unix)]
    if let Some(peer) = _uds_fast_path.discover_peer_for_url(url) {
        let uds_client = Client::builder()
            .unix_socket(peer.socket_path.as_path())
            .build()
            .map_err(|error| {
                format!(
                    "failed to build UDS mesh client for `{}`: {error}",
                    peer.socket_path.display()
                )
            })?;
        let request = apply_mesh_fetch_headers(
            uds_client.get(url),
            target_kind,
            hop_limit,
            propagated_headers,
            identity_token,
        );
        match request.send().await {
            Ok(response) => return Ok(response),
            Err(error) => {
                _uds_fast_path.note_connect_failure(&peer);
                tracing::debug!(
                    socket = %peer.socket_path.display(),
                    url = %url,
                    "UDS fast-path unavailable, falling back to TCP: {error}"
                );
            }
        }
    }

    apply_mesh_fetch_headers(
        http_client.get(url),
        target_kind,
        hop_limit,
        propagated_headers,
        identity_token,
    )
    .send()
    .await
    .map_err(|error| format!("mesh fetch to `{url}` failed: {error}"))
}

pub(crate) fn extract_mesh_fetch_url(stdout: &Bytes) -> Option<&str> {
    std::str::from_utf8(stdout)
        .ok()?
        .trim()
        .strip_prefix("MESH_FETCH:")
        .map(str::trim)
        .filter(|url| !url.is_empty())
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
