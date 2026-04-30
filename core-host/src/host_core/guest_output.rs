fn guest_execution_error(error: wasmtime::Error, context: impl Into<String>) -> ExecutionError {
    let error = error.context(context.into());

    if let Some(kind) = classify_resource_limit(&error) {
        return ExecutionError::ResourceLimitExceeded {
            kind,
            detail: format!("{error:#}"),
        };
    }

    ExecutionError::Internal(format!("{error:#}"))
}

fn classify_resource_limit(error: &wasmtime::Error) -> Option<ResourceLimitKind> {
    if let Some(limit) = error.downcast_ref::<ResourceLimitTrap>() {
        return Some(limit.kind);
    }

    error.downcast_ref::<Trap>().and_then(|trap| match trap {
        Trap::OutOfFuel => Some(ResourceLimitKind::Fuel),
        Trap::AllocationTooLarge => Some(ResourceLimitKind::Memory),
        _ => None,
    })
}

#[cfg(test)]
fn split_guest_stdout(function_name: &str, stdout: Bytes) -> Bytes {
    let output = String::from_utf8_lossy(&stdout);
    let mut response = String::new();

    for segment in output.split_inclusive('\n') {
        let line = trim_line_endings(segment);

        if let Some(record) = parse_guest_log_line(line) {
            forward_guest_log(function_name, record);
            continue;
        }

        response.push_str(segment);
    }

    Bytes::from(response)
}

fn trim_line_endings(segment: &str) -> &str {
    let trimmed = segment.strip_suffix('\n').unwrap_or(segment);
    trimmed.strip_suffix('\r').unwrap_or(trimmed)
}

fn parse_guest_log_line(line: &str) -> Option<GuestLogRecord> {
    serde_json::from_str::<GuestLogRecord>(line).ok()
}

fn flush_async_guest_output(state: &mut AsyncGuestOutputState) {
    if state.pending.is_empty() {
        return;
    }

    let segment = std::mem::take(&mut state.pending);
    handle_async_guest_segment(state, &segment);
}

fn handle_async_guest_segment(state: &mut AsyncGuestOutputState, segment: &[u8]) {
    let text = String::from_utf8_lossy(segment);
    let line = trim_line_endings(&text);
    if line.is_empty() {
        if state.capture_response {
            append_async_guest_response(state, segment);
        }
        return;
    }

    if let Some(record) = parse_guest_log_line(line) {
        enqueue_structured_guest_log(state, record);
        return;
    }

    if state.capture_response {
        append_async_guest_response(state, segment);
    } else {
        enqueue_raw_guest_log(state, line.to_owned());
    }
}

fn append_async_guest_response(state: &mut AsyncGuestOutputState, segment: &[u8]) {
    if state.response_overflowed {
        return;
    }

    state.response.extend_from_slice(segment);
    if state.response.len() > state.max_response_bytes {
        state.response_overflowed = true;
    }
}

fn enqueue_structured_guest_log(state: &AsyncGuestOutputState, record: GuestLogRecord) {
    let message = record
        .fields
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("guest emitted a structured log")
        .to_owned();
    enqueue_async_guest_log(
        state,
        record.level,
        message,
        record.target,
        Some(Value::Object(record.fields)),
    );
}

fn enqueue_raw_guest_log(state: &AsyncGuestOutputState, message: String) {
    let level = match state.stream_type {
        Some(GuestLogStreamType::Stderr) => "error",
        _ => "info",
    };
    enqueue_async_guest_log(state, level.to_owned(), message, None, None);
}

fn enqueue_async_guest_log(
    state: &AsyncGuestOutputState,
    level: String,
    message: String,
    guest_target: Option<String>,
    structured_fields: Option<Value>,
) {
    let Some(sender) = &state.sender else {
        return;
    };
    let Some(stream_type) = state.stream_type else {
        return;
    };
    let timestamp_unix_ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default();

    let _ = sender.try_send(AsyncLogEntry {
        target_name: state.function_name.clone(),
        timestamp_unix_ms,
        stream_type,
        level,
        message,
        guest_target,
        structured_fields,
    });
}

#[cfg(test)]
fn forward_guest_log(function_name: &str, record: GuestLogRecord) {
    let level = record.level.to_ascii_uppercase();
    let target = record.target.unwrap_or_else(|| "guest".to_owned());
    let message = record
        .fields
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("guest emitted a structured log")
        .to_owned();
    let fields = Value::Object(record.fields).to_string();

    match level.as_str() {
        "TRACE" => tracing::trace!(
            guest_function = function_name,
            guest_target = %target,
            guest_fields = %fields,
            "{message}"
        ),
        "DEBUG" => tracing::debug!(
            guest_function = function_name,
            guest_target = %target,
            guest_fields = %fields,
            "{message}"
        ),
        "WARN" => tracing::warn!(
            guest_function = function_name,
            guest_target = %target,
            guest_fields = %fields,
            "{message}"
        ),
        "ERROR" => tracing::error!(
            guest_function = function_name,
            guest_target = %target,
            guest_fields = %fields,
            "{message}"
        ),
        _ => tracing::info!(
            guest_function = function_name,
            guest_target = %target,
            guest_fields = %fields,
            "{message}"
        ),
    }
}
