use super::*;

/// Reuses the API-auth material that created the WebRTC call for the sideband WebSocket join.
///
/// API-key sessions send that API bearer. ChatGPT-auth sessions send their bearer plus account id;
/// transceiver is responsible for accepting that same call-create identity on the direct
/// `api.openai.com` sideband path.
pub(super) fn sideband_websocket_auth_headers(api_auth: &dyn AuthProvider) -> ApiHeaderMap {
    let mut headers = ApiHeaderMap::new();
    api_auth.add_auth_headers(&mut headers);
    headers
}

/// Parses per-turn metadata into an HTTP header value.
///
/// Invalid values are treated as absent so callers can compare and propagate
/// metadata with the same sanitization path used when constructing headers.
pub(super) fn parse_turn_metadata_header(
    turn_metadata_header: Option<&str>,
) -> Option<HeaderValue> {
    turn_metadata_header.and_then(|value| HeaderValue::from_str(value).ok())
}

/// Stamp a ResponsesWsRequest with the current time.
///
/// Meant to be called just before sending the request over the socket, to capture realistic
/// transport timing.
pub(super) fn stamp_ws_stream_request_start_ms(request: &mut ResponsesWsRequest) {
    let ResponsesWsRequest::ResponseCreate(payload) = request else {
        return;
    };
    payload
        .client_metadata
        .get_or_insert_with(HashMap::new)
        .insert(
            X_CODEX_WS_STREAM_REQUEST_START_MS_CLIENT_METADATA_KEY.to_string(),
            now_unix_timestamp_ms().to_string(),
        );
}

fn now_unix_timestamp_ms() -> i64 {
    use std::time::SystemTime;
    use std::time::UNIX_EPOCH;

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Builds the extra headers attached to Responses API requests.
///
/// These headers implement Codex-specific conventions:
///
/// - `x-codex-beta-features`: comma-separated beta feature keys enabled for the session.
/// - `x-codex-turn-state`: sticky routing token captured earlier in the turn.
/// - `x-codex-turn-metadata`: optional per-turn metadata for observability.
pub(super) fn build_responses_headers(
    beta_features_header: Option<&str>,
    turn_state: Option<&Arc<OnceLock<String>>>,
    turn_metadata_header: Option<&HeaderValue>,
) -> ApiHeaderMap {
    let mut headers = ApiHeaderMap::new();
    if let Some(value) = beta_features_header
        && !value.is_empty()
        && let Ok(header_value) = HeaderValue::from_str(value)
    {
        headers.insert("x-codex-beta-features", header_value);
    }
    if let Some(turn_state) = turn_state
        && let Some(state) = turn_state.get()
        && let Ok(header_value) = HeaderValue::from_str(state)
    {
        headers.insert(X_CODEX_TURN_STATE_HEADER, header_value);
    }
    if let Some(header_value) = turn_metadata_header {
        headers.insert(X_CODEX_TURN_METADATA_HEADER, header_value.clone());
    }
    headers
}

pub(super) fn subagent_header_value(session_source: &SessionSource) -> Option<String> {
    match session_source {
        SessionSource::SubAgent(subagent_source) => match subagent_source {
            SubAgentSource::Review => Some("review".to_string()),
            SubAgentSource::Compact => Some("compact".to_string()),
            SubAgentSource::MemoryConsolidation => Some("memory_consolidation".to_string()),
            SubAgentSource::ThreadSpawn { .. } => Some("collab_spawn".to_string()),
            SubAgentSource::Other(label) => Some(label.clone()),
        },
        SessionSource::Internal(InternalSessionSource::MemoryConsolidation) => {
            Some("memory_consolidation".to_string())
        }
        SessionSource::Cli
        | SessionSource::VSCode
        | SessionSource::Exec
        | SessionSource::Mcp
        | SessionSource::Custom(_)
        | SessionSource::Unknown => None,
    }
}

pub(super) fn parent_thread_id_header_value(session_source: &SessionSource) -> Option<String> {
    match session_source {
        SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id, ..
        }) => Some(parent_thread_id.to_string()),
        SessionSource::Cli
        | SessionSource::VSCode
        | SessionSource::Exec
        | SessionSource::Mcp
        | SessionSource::Custom(_)
        | SessionSource::Internal(_)
        | SessionSource::SubAgent(_)
        | SessionSource::Unknown => None,
    }
}
