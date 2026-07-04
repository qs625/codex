use super::*;

impl ModelClient {
    pub async fn create_realtime_call_with_headers(
        &self,
        sdp: String,
        api_provider: ApiProvider,
        session_config: ApiRealtimeSessionConfig,
        mut extra_headers: ApiHeaderMap,
    ) -> Result<RealtimeWebrtcCallStart> {
        let client_setup = self.current_client_setup().await?;
        if let Some(header_value) = self.generate_attestation_header_for().await {
            extra_headers.insert(X_OAI_ATTESTATION_HEADER, header_value);
        }
        let mut sideband_headers = extra_headers.clone();
        sideband_headers.extend(sideband_websocket_auth_headers(
            client_setup.api_auth.as_ref(),
        ));
        let response = self
            .state
            .api_runtime_factory
            .realtime_call_client(api_provider, client_setup.api_auth)
            .create(RealtimeCallRuntimeRequest {
                sdp,
                session_config,
                extra_headers,
                request_telemetry: None,
            })
            .await
            .map_err(map_api_error)?;
        Ok(RealtimeWebrtcCallStart {
            sdp: response.sdp,
            call_id: response.call_id,
            sideband_headers,
        })
    }

    pub fn realtime_websocket_client(
        &self,
        api_provider: ApiProvider,
    ) -> Box<dyn RealtimeWebsocketClientRuntime> {
        self.state
            .api_runtime_factory
            .realtime_websocket_client(api_provider)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn connect_websocket(
        &self,
        session_telemetry: &SharedSessionTelemetry,
        api_provider: ApiProvider,
        api_auth: SharedAuthProvider,
        turn_state: Option<Arc<OnceLock<String>>>,
        turn_metadata_header: Option<&str>,
        auth_context: AuthRequestTelemetryContext,
        request_route_telemetry: RequestRouteTelemetry,
    ) -> std::result::Result<ApiWebSocketConnection, ApiError> {
        let headers = self
            .build_websocket_headers(turn_state.as_ref(), turn_metadata_header)
            .await;
        let websocket_telemetry = ModelClientSession::build_websocket_telemetry(
            session_telemetry,
            auth_context,
            request_route_telemetry,
            self.state.auth_env_telemetry.clone(),
        );
        let websocket_connect_timeout = self.state.provider.info().websocket_connect_timeout();
        let start = Instant::now();
        let connector = self
            .state
            .api_runtime_factory
            .responses_websocket_connector(api_provider, api_auth);
        let request = ResponsesWebsocketConnectRequest {
            extra_headers: headers,
            default_headers: transport_client_identity::default_identity_headers(),
            turn_state,
            telemetry: Some(websocket_telemetry),
        };
        let result = match tokio::time::timeout(
            websocket_connect_timeout,
            ResponsesWebsocketConnectorRuntime::connect(connector.as_ref(), request),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(ApiError::Transport(TransportError::Timeout)),
        };
        let error_message = result.as_ref().err().map(telemetry_api_error_message);
        let response_debug = result
            .as_ref()
            .err()
            .map(extract_response_debug_context_from_api_error)
            .unwrap_or_default();
        let status = result.as_ref().err().and_then(api_error_http_status);
        session_telemetry.record_websocket_connect(
            start.elapsed(),
            status,
            error_message.as_deref(),
            auth_context.auth_header_attached,
            auth_context.auth_header_name,
            auth_context.retry_after_unauthorized,
            auth_context.recovery_mode,
            auth_context.recovery_phase,
            request_route_telemetry.endpoint,
            /*connection_reused*/ false,
            response_debug.request_id.as_deref(),
            response_debug.cf_ray.as_deref(),
            response_debug.auth_error.as_deref(),
            response_debug.auth_error_code.as_deref(),
        );
        emit_feedback_request_tags_with_auth_env(
            &FeedbackRequestTags {
                endpoint: request_route_telemetry.endpoint,
                auth_header_attached: auth_context.auth_header_attached,
                auth_header_name: auth_context.auth_header_name,
                auth_mode: auth_context.auth_mode,
                auth_retry_after_unauthorized: Some(auth_context.retry_after_unauthorized),
                auth_recovery_mode: auth_context.recovery_mode,
                auth_recovery_phase: auth_context.recovery_phase,
                auth_connection_reused: Some(false),
                auth_request_id: response_debug.request_id.as_deref(),
                auth_cf_ray: response_debug.cf_ray.as_deref(),
                auth_error: response_debug.auth_error.as_deref(),
                auth_error_code: response_debug.auth_error_code.as_deref(),
                auth_recovery_followup_success: auth_context
                    .retry_after_unauthorized
                    .then_some(result.is_ok()),
                auth_recovery_followup_status: auth_context
                    .retry_after_unauthorized
                    .then_some(status)
                    .flatten(),
            },
            &self.state.auth_env_telemetry.to_otel_metadata(),
        );
        result
    }

    async fn build_websocket_headers(
        &self,
        turn_state: Option<&Arc<OnceLock<String>>>,
        turn_metadata_header: Option<&str>,
    ) -> ApiHeaderMap {
        let turn_metadata_header = parse_turn_metadata_header(turn_metadata_header);
        let session_id = self.state.session_id.to_string();
        let thread_id = self.state.thread_id.to_string();
        let mut headers = build_responses_headers(
            self.state.beta_features_header.as_deref(),
            turn_state,
            turn_metadata_header.as_ref(),
        );
        if let Ok(header_value) = HeaderValue::from_str(&thread_id) {
            headers.insert("x-client-request-id", header_value);
        }
        headers.extend(build_session_headers(Some(session_id), Some(thread_id)));
        headers.extend(self.build_responses_identity_headers());
        if let Some(header_value) = self.generate_attestation_header_for().await {
            headers.insert(X_OAI_ATTESTATION_HEADER, header_value);
        }
        headers.insert(
            OPENAI_BETA_HEADER,
            HeaderValue::from_static(RESPONSES_WEBSOCKETS_V2_BETA_HEADER_VALUE),
        );
        if self.state.include_timing_metrics {
            headers.insert(
                X_RESPONSESAPI_INCLUDE_TIMING_METRICS_HEADER,
                HeaderValue::from_static("true"),
            );
        }
        headers
    }
}
