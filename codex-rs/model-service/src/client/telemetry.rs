use super::*;

struct Auth401FeedbackSnapshot<'a> {
    request_id: &'a str,
    cf_ray: &'a str,
    error: &'a str,
    error_code: &'a str,
}

impl<'a> Auth401FeedbackSnapshot<'a> {
    fn from_optional_fields(
        request_id: Option<&'a str>,
        cf_ray: Option<&'a str>,
        error: Option<&'a str>,
        error_code: Option<&'a str>,
    ) -> Self {
        Self {
            request_id: request_id.unwrap_or(""),
            cf_ray: cf_ray.unwrap_or(""),
            error: error.unwrap_or(""),
            error_code: error_code.unwrap_or(""),
        }
    }
}

fn emit_feedback_auth_recovery_tags(
    auth_recovery_mode: &str,
    auth_recovery_phase: &str,
    auth_recovery_outcome: &str,
    auth_request_id: Option<&str>,
    auth_cf_ray: Option<&str>,
    auth_error: Option<&str>,
    auth_error_code: Option<&str>,
) {
    let auth_401 = Auth401FeedbackSnapshot::from_optional_fields(
        auth_request_id,
        auth_cf_ray,
        auth_error,
        auth_error_code,
    );
    feedback_tags!(
        auth_recovery_mode = auth_recovery_mode,
        auth_recovery_phase = auth_recovery_phase,
        auth_recovery_outcome = auth_recovery_outcome,
        auth_401_request_id = auth_401.request_id,
        auth_401_cf_ray = auth_401.cf_ray,
        auth_401_error = auth_401.error,
        auth_401_error_code = auth_401.error_code
    );
}

/// Handles a 401 response by optionally refreshing ChatGPT tokens once.
///
/// When refresh succeeds, the caller should retry the API call; otherwise
/// the mapped `CodexErr` is returned to the caller.
#[derive(Clone, Copy, Debug)]
pub(super) struct UnauthorizedRecoveryExecution {
    pub(super) mode: &'static str,
    pub(super) phase: &'static str,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct PendingUnauthorizedRetry {
    pub(super) retry_after_unauthorized: bool,
    pub(super) recovery_mode: Option<&'static str>,
    pub(super) recovery_phase: Option<&'static str>,
}

impl PendingUnauthorizedRetry {
    pub(super) fn from_recovery(recovery: UnauthorizedRecoveryExecution) -> Self {
        Self {
            retry_after_unauthorized: true,
            recovery_mode: Some(recovery.mode),
            recovery_phase: Some(recovery.phase),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct AuthRequestTelemetryContext {
    pub(super) auth_mode: Option<&'static str>,
    pub(super) auth_header_attached: bool,
    pub(super) auth_header_name: Option<&'static str>,
    pub(super) retry_after_unauthorized: bool,
    pub(super) recovery_mode: Option<&'static str>,
    pub(super) recovery_phase: Option<&'static str>,
}

impl AuthRequestTelemetryContext {
    pub(super) fn new(
        auth_mode: Option<AuthMode>,
        api_auth: &dyn AuthProvider,
        retry: PendingUnauthorizedRetry,
    ) -> Self {
        let auth_telemetry = auth_header_telemetry(api_auth);
        Self {
            auth_mode: auth_mode.map(|mode| match mode {
                AuthMode::ApiKey => "ApiKey",
                AuthMode::Chatgpt | AuthMode::ChatgptAuthTokens | AuthMode::AgentIdentity => {
                    "Chatgpt"
                }
            }),
            auth_header_attached: auth_telemetry.attached,
            auth_header_name: auth_telemetry.name,
            retry_after_unauthorized: retry.retry_after_unauthorized,
            recovery_mode: retry.recovery_mode,
            recovery_phase: retry.recovery_phase,
        }
    }
}

pub(super) async fn handle_unauthorized(
    transport: TransportError,
    auth_recovery: &mut Option<Box<dyn ModelProviderUnauthorizedRecovery>>,
    session_telemetry: &SharedSessionTelemetry,
) -> Result<UnauthorizedRecoveryExecution> {
    let debug = extract_response_debug_context(&transport);
    if let Some(recovery) = auth_recovery
        && recovery.has_next()
    {
        let mode = recovery.mode_name();
        let phase = recovery.step_name();
        return match recovery.next().await {
            Ok(step_result) => {
                session_telemetry.record_auth_recovery(
                    mode,
                    phase,
                    "recovery_succeeded",
                    debug.request_id.as_deref(),
                    debug.cf_ray.as_deref(),
                    debug.auth_error.as_deref(),
                    debug.auth_error_code.as_deref(),
                    /*recovery_reason*/ None,
                    step_result.auth_state_changed(),
                );
                emit_feedback_auth_recovery_tags(
                    mode,
                    phase,
                    "recovery_succeeded",
                    debug.request_id.as_deref(),
                    debug.cf_ray.as_deref(),
                    debug.auth_error.as_deref(),
                    debug.auth_error_code.as_deref(),
                );
                Ok(UnauthorizedRecoveryExecution { mode, phase })
            }
            Err(ModelProviderAuthRecoveryError::Permanent(failed)) => {
                session_telemetry.record_auth_recovery(
                    mode,
                    phase,
                    "recovery_failed_permanent",
                    debug.request_id.as_deref(),
                    debug.cf_ray.as_deref(),
                    debug.auth_error.as_deref(),
                    debug.auth_error_code.as_deref(),
                    /*recovery_reason*/ None,
                    /*auth_state_changed*/ None,
                );
                emit_feedback_auth_recovery_tags(
                    mode,
                    phase,
                    "recovery_failed_permanent",
                    debug.request_id.as_deref(),
                    debug.cf_ray.as_deref(),
                    debug.auth_error.as_deref(),
                    debug.auth_error_code.as_deref(),
                );
                Err(CodexErr::RefreshTokenFailed(failed))
            }
            Err(ModelProviderAuthRecoveryError::Transient(other)) => {
                session_telemetry.record_auth_recovery(
                    mode,
                    phase,
                    "recovery_failed_transient",
                    debug.request_id.as_deref(),
                    debug.cf_ray.as_deref(),
                    debug.auth_error.as_deref(),
                    debug.auth_error_code.as_deref(),
                    /*recovery_reason*/ None,
                    /*auth_state_changed*/ None,
                );
                emit_feedback_auth_recovery_tags(
                    mode,
                    phase,
                    "recovery_failed_transient",
                    debug.request_id.as_deref(),
                    debug.cf_ray.as_deref(),
                    debug.auth_error.as_deref(),
                    debug.auth_error_code.as_deref(),
                );
                Err(CodexErr::Io(other))
            }
        };
    }

    let (mode, phase, recovery_reason) = match auth_recovery.as_ref() {
        Some(recovery) => (
            recovery.mode_name(),
            recovery.step_name(),
            Some(recovery.unavailable_reason()),
        ),
        None => ("none", "none", Some("auth_manager_missing")),
    };
    session_telemetry.record_auth_recovery(
        mode,
        phase,
        "recovery_not_run",
        debug.request_id.as_deref(),
        debug.cf_ray.as_deref(),
        debug.auth_error.as_deref(),
        debug.auth_error_code.as_deref(),
        recovery_reason,
        /*auth_state_changed*/ None,
    );
    emit_feedback_auth_recovery_tags(
        mode,
        phase,
        "recovery_not_run",
        debug.request_id.as_deref(),
        debug.cf_ray.as_deref(),
        debug.auth_error.as_deref(),
        debug.auth_error_code.as_deref(),
    );

    Err(map_api_error(ApiError::Transport(transport)))
}

pub(super) fn api_error_http_status(error: &ApiError) -> Option<u16> {
    match error {
        ApiError::Transport(TransportError::Http { status, .. }) => Some(status.as_u16()),
        _ => None,
    }
}

pub(super) struct ApiTelemetry {
    session_telemetry: SharedSessionTelemetry,
    auth_context: AuthRequestTelemetryContext,
    request_route_telemetry: RequestRouteTelemetry,
    auth_env_telemetry: AuthEnvTelemetry,
}

fn session_sse_event_telemetry(event: &SseEventTelemetry) -> SessionSseEventTelemetry {
    SessionSseEventTelemetry {
        kind: event.kind.clone(),
        success: event.success,
        error_message: event.error_message.clone(),
    }
}

fn session_websocket_event_telemetry(
    event: &WebsocketEventTelemetry,
) -> SessionWebsocketEventTelemetry {
    SessionWebsocketEventTelemetry {
        kind: event.kind.clone(),
        success: event.success,
        error_message: event.error_message.clone(),
        payload: event.payload.clone(),
    }
}

impl ApiTelemetry {
    pub(super) fn new(
        session_telemetry: SharedSessionTelemetry,
        auth_context: AuthRequestTelemetryContext,
        request_route_telemetry: RequestRouteTelemetry,
        auth_env_telemetry: AuthEnvTelemetry,
    ) -> Self {
        Self {
            session_telemetry,
            auth_context,
            request_route_telemetry,
            auth_env_telemetry,
        }
    }
}

impl RequestTelemetry for ApiTelemetry {
    fn on_request(
        &self,
        attempt: u64,
        status: Option<HttpStatusCode>,
        error: Option<&TransportError>,
        duration: Duration,
    ) {
        let error_message = error.map(telemetry_transport_error_message);
        let status = status.map(|s| s.as_u16());
        let debug = error
            .map(extract_response_debug_context)
            .unwrap_or_default();
        self.session_telemetry.record_api_request(
            attempt,
            status,
            error_message.as_deref(),
            duration,
            self.auth_context.auth_header_attached,
            self.auth_context.auth_header_name,
            self.auth_context.retry_after_unauthorized,
            self.auth_context.recovery_mode,
            self.auth_context.recovery_phase,
            self.request_route_telemetry.endpoint,
            debug.request_id.as_deref(),
            debug.cf_ray.as_deref(),
            debug.auth_error.as_deref(),
            debug.auth_error_code.as_deref(),
        );
        emit_feedback_request_tags_with_auth_env(
            &FeedbackRequestTags {
                endpoint: self.request_route_telemetry.endpoint,
                auth_header_attached: self.auth_context.auth_header_attached,
                auth_header_name: self.auth_context.auth_header_name,
                auth_mode: self.auth_context.auth_mode,
                auth_retry_after_unauthorized: Some(self.auth_context.retry_after_unauthorized),
                auth_recovery_mode: self.auth_context.recovery_mode,
                auth_recovery_phase: self.auth_context.recovery_phase,
                auth_connection_reused: None,
                auth_request_id: debug.request_id.as_deref(),
                auth_cf_ray: debug.cf_ray.as_deref(),
                auth_error: debug.auth_error.as_deref(),
                auth_error_code: debug.auth_error_code.as_deref(),
                auth_recovery_followup_success: self
                    .auth_context
                    .retry_after_unauthorized
                    .then_some(error.is_none()),
                auth_recovery_followup_status: self
                    .auth_context
                    .retry_after_unauthorized
                    .then_some(status)
                    .flatten(),
            },
            &self.auth_env_telemetry.to_otel_metadata(),
        );
    }
}

impl SseTelemetry for ApiTelemetry {
    fn on_sse_poll(&self, event: Option<&SseEventTelemetry>, duration: Duration) {
        let telemetry = event.map(session_sse_event_telemetry);
        self.session_telemetry
            .log_sse_event(telemetry.as_ref(), duration);
    }
}

impl WebsocketTelemetry for ApiTelemetry {
    fn on_ws_request(&self, duration: Duration, error: Option<&ApiError>, connection_reused: bool) {
        let error_message = error.map(telemetry_api_error_message);
        let status = error.and_then(api_error_http_status);
        let debug = error
            .map(extract_response_debug_context_from_api_error)
            .unwrap_or_default();
        self.session_telemetry.record_websocket_request(
            duration,
            error_message.as_deref(),
            connection_reused,
        );
        emit_feedback_request_tags_with_auth_env(
            &FeedbackRequestTags {
                endpoint: self.request_route_telemetry.endpoint,
                auth_header_attached: self.auth_context.auth_header_attached,
                auth_header_name: self.auth_context.auth_header_name,
                auth_mode: self.auth_context.auth_mode,
                auth_retry_after_unauthorized: Some(self.auth_context.retry_after_unauthorized),
                auth_recovery_mode: self.auth_context.recovery_mode,
                auth_recovery_phase: self.auth_context.recovery_phase,
                auth_connection_reused: Some(connection_reused),
                auth_request_id: debug.request_id.as_deref(),
                auth_cf_ray: debug.cf_ray.as_deref(),
                auth_error: debug.auth_error.as_deref(),
                auth_error_code: debug.auth_error_code.as_deref(),
                auth_recovery_followup_success: self
                    .auth_context
                    .retry_after_unauthorized
                    .then_some(error.is_none()),
                auth_recovery_followup_status: self
                    .auth_context
                    .retry_after_unauthorized
                    .then_some(status)
                    .flatten(),
            },
            &self.auth_env_telemetry.to_otel_metadata(),
        );
    }

    fn on_ws_event(&self, event: Option<&WebsocketEventTelemetry>, duration: Duration) {
        let telemetry = event.map(session_websocket_event_telemetry);
        self.session_telemetry
            .record_websocket_event(telemetry.as_ref(), duration);
    }
}
