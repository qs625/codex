use super::*;
use crate::responses_websocket_client::is_websocket_request_send_closed_error;

impl Drop for ModelClientSession {
    fn drop(&mut self) {
        let websocket_session = std::mem::take(&mut self.websocket_session);
        self.client
            .store_cached_websocket_session(websocket_session);
    }
}

impl ModelClientSession {
    pub fn reset_websocket_session(&mut self) {
        self.websocket_session.connection = None;
        self.websocket_session.last_request = None;
        self.websocket_session.last_response_rx = None;
        self.websocket_session
            .set_connection_reused(/*connection_reused*/ false);
    }

    pub async fn send_response_processed(&self, response_id: &str) {
        let Some(connection) = self.websocket_session.connection.as_ref() else {
            return;
        };
        if let Err(err) = connection
            .send_response_processed(response_id.to_string())
            .await
        {
            debug!("failed to send response.processed websocket request: {err}");
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn build_responses_options(
        &self,
        turn_metadata_header: Option<&str>,
        compression: Compression,
    ) -> ApiResponsesOptions {
        let turn_metadata_header = parse_turn_metadata_header(turn_metadata_header);
        let session_id = self.client.state.session_id.to_string();
        let thread_id = self.client.state.thread_id.to_string();
        ApiResponsesOptions {
            session_id: Some(session_id),
            thread_id: Some(thread_id),
            session_source: Some(self.client.state.session_source.clone()),
            extra_headers: {
                let mut headers = build_responses_headers(
                    self.client.state.beta_features_header.as_deref(),
                    Some(&self.turn_state),
                    turn_metadata_header.as_ref(),
                );
                headers.extend(self.client.build_responses_identity_headers());
                if let Some(header_value) = self.client.generate_attestation_header_for().await {
                    headers.insert(X_OAI_ATTESTATION_HEADER, header_value);
                }
                headers
            },
            compression,
            turn_state: Some(Arc::clone(&self.turn_state)),
        }
    }

    fn responses_request_compression(
        &self,
        auth: Option<&codex_auth_types::RequestAuthSnapshot>,
    ) -> Compression {
        if self.client.state.enable_request_compression
            && auth.is_some_and(codex_auth_types::RequestAuthSnapshot::uses_codex_backend)
            && self.client.state.provider.info().is_openai()
        {
            Compression::Zstd
        } else {
            Compression::None
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[instrument(
        name = "model_client.stream_responses_api",
        level = "info",
        skip_all,
        fields(
            model = %model_info.slug,
            wire_api = %self.client.state.provider.info().wire_api,
            transport = "responses_http",
            http.method = "POST",
            api.path = "responses",
            turn.has_metadata_header = turn_metadata_header.is_some()
        )
    )]
    async fn stream_responses_api(
        &self,
        prompt: &Prompt,
        model_info: &ModelInfo,
        session_telemetry: &SharedSessionTelemetry,
        effort: Option<ReasoningEffortConfig>,
        summary: ReasoningSummaryConfig,
        service_tier: Option<String>,
        turn_metadata_header: Option<&str>,
        inference_trace: &InferenceTraceContext,
    ) -> Result<ResponseStream> {
        let auth_manager = self.client.state.provider.auth_manager();
        let mut auth_recovery = auth_manager
            .as_ref()
            .and_then(|auth_manager| auth_manager.unauthorized_recovery());
        let mut pending_retry = PendingUnauthorizedRetry::default();
        loop {
            let client_setup = self.client.current_client_setup().await?;
            let request_auth_context = AuthRequestTelemetryContext::new(
                client_setup
                    .auth
                    .as_ref()
                    .map(codex_auth_types::RequestAuthSnapshot::auth_mode),
                client_setup.api_auth.as_ref(),
                pending_retry,
            );
            let (request_telemetry, sse_telemetry) = Self::build_streaming_telemetry(
                session_telemetry,
                request_auth_context,
                RequestRouteTelemetry::for_endpoint(RESPONSES_ENDPOINT),
                self.client.state.auth_env_telemetry.clone(),
            );
            let compression = self.responses_request_compression(client_setup.auth.as_ref());
            let mut options = self
                .build_responses_options(turn_metadata_header, compression)
                .await;

            let request = self.client.build_responses_request(
                &client_setup.api_provider,
                prompt,
                model_info,
                effort,
                summary,
                service_tier.clone(),
            )?;
            let inference_trace_attempt = inference_trace.start_attempt();
            inference_trace_attempt.add_request_headers(&mut options.extra_headers);
            inference_trace_attempt.record_started(&request);
            let client = self
                .client
                .state
                .api_runtime_factory
                .responses_client(client_setup.api_provider, client_setup.api_auth);
            let stream_result = client
                .stream_request(ResponsesStreamRuntimeRequest {
                    request,
                    options,
                    request_telemetry: Some(request_telemetry),
                    sse_telemetry: Some(sse_telemetry),
                })
                .await;

            match stream_result {
                Ok(stream) => {
                    let (stream, _) = map_response_stream(
                        stream,
                        session_telemetry.clone(),
                        inference_trace_attempt,
                    );
                    return Ok(stream);
                }
                Err(ApiError::Transport(
                    unauthorized_transport @ TransportError::Http { status, .. },
                )) if status == HttpStatusCode::UNAUTHORIZED => {
                    let response_debug_context =
                        extract_response_debug_context(&unauthorized_transport);
                    inference_trace_attempt.record_failed(
                        &unauthorized_transport,
                        response_debug_context.request_id.as_deref(),
                        /*output_items*/ &[],
                    );
                    pending_retry = PendingUnauthorizedRetry::from_recovery(
                        handle_unauthorized(
                            unauthorized_transport,
                            &mut auth_recovery,
                            session_telemetry,
                        )
                        .await?,
                    );
                    continue;
                }
                Err(err) => {
                    let response_debug_context =
                        extract_response_debug_context_from_api_error(&err);
                    let err = map_api_error(err);
                    inference_trace_attempt.record_failed(
                        &err,
                        response_debug_context.request_id.as_deref(),
                        /*output_items*/ &[],
                    );
                    return Err(err);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[instrument(
        name = "model_client.stream_responses_websocket",
        level = "info",
        skip_all,
        fields(
            model = %model_info.slug,
            wire_api = %self.client.state.provider.info().wire_api,
            transport = "responses_websocket",
            api.path = "responses",
            turn.has_metadata_header = turn_metadata_header.is_some(),
            websocket.warmup = warmup
        )
    )]
    async fn stream_responses_websocket(
        &mut self,
        prompt: &Prompt,
        model_info: &ModelInfo,
        session_telemetry: &SharedSessionTelemetry,
        effort: Option<ReasoningEffortConfig>,
        summary: ReasoningSummaryConfig,
        service_tier: Option<String>,
        turn_metadata_header: Option<&str>,
        warmup: bool,
        request_trace: Option<W3cTraceContext>,
        inference_trace: &InferenceTraceContext,
    ) -> Result<WebsocketStreamOutcome> {
        let auth_manager = self.client.state.provider.auth_manager();

        let mut auth_recovery = auth_manager
            .as_ref()
            .and_then(|auth_manager| auth_manager.unauthorized_recovery());
        let mut pending_retry = PendingUnauthorizedRetry::default();
        let mut stale_reused_send_retry_used = false;
        loop {
            let client_setup = self.client.current_client_setup().await?;
            let request_auth_context = AuthRequestTelemetryContext::new(
                client_setup
                    .auth
                    .as_ref()
                    .map(codex_auth_types::RequestAuthSnapshot::auth_mode),
                client_setup.api_auth.as_ref(),
                pending_retry,
            );
            let compression = self.responses_request_compression(client_setup.auth.as_ref());

            let options = self
                .build_responses_options(turn_metadata_header, compression)
                .await;
            let request = self.client.build_responses_request(
                &client_setup.api_provider,
                prompt,
                model_info,
                effort,
                summary,
                service_tier.clone(),
            )?;
            let mut ws_payload = ResponseCreateWsRequest {
                client_metadata: response_create_client_metadata(
                    Some(self.client.build_ws_client_metadata(turn_metadata_header)),
                    request_trace.as_ref(),
                ),
                ..ResponseCreateWsRequest::from(&request)
            };
            if warmup {
                ws_payload.generate = Some(false);
            }

            match self
                .websocket_connection(WebsocketConnectParams {
                    session_telemetry,
                    api_provider: client_setup.api_provider,
                    api_auth: client_setup.api_auth,
                    turn_metadata_header,
                    options: &options,
                    auth_context: request_auth_context,
                    request_route_telemetry: RequestRouteTelemetry::for_endpoint(
                        RESPONSES_ENDPOINT,
                    ),
                })
                .await
            {
                Ok(_) => {}
                Err(ApiError::Transport(TransportError::Http { status, .. }))
                    if status == HttpStatusCode::UPGRADE_REQUIRED =>
                {
                    return Ok(WebsocketStreamOutcome::FallbackToHttp);
                }
                Err(ApiError::Transport(
                    unauthorized_transport @ TransportError::Http { status, .. },
                )) if status == HttpStatusCode::UNAUTHORIZED => {
                    pending_retry = PendingUnauthorizedRetry::from_recovery(
                        handle_unauthorized(
                            unauthorized_transport,
                            &mut auth_recovery,
                            session_telemetry,
                        )
                        .await?,
                    );
                    continue;
                }
                Err(err) => return Err(map_api_error(err)),
            }

            let mut ws_request = self.prepare_websocket_request(ws_payload, &request);
            self.websocket_session.last_request = Some(request);
            let inference_trace_attempt = if warmup {
                InferenceTraceAttempt::disabled()
            } else {
                inference_trace.start_attempt()
            };
            stamp_ws_stream_request_start_ms(&mut ws_request);
            inference_trace_attempt.record_started(&ws_request);
            let websocket_connection =
                self.websocket_session.connection.as_ref().ok_or_else(|| {
                    map_api_error(ApiError::Stream(
                        "websocket connection is unavailable".to_string(),
                    ))
                })?;
            let connection_reused = self.websocket_session.connection_reused();
            let stream_result = websocket_connection
                .stream_request(ws_request, connection_reused)
                .await;
            let stream_result = match stream_result {
                Ok(stream) => stream,
                Err(err)
                    if connection_reused
                        && !stale_reused_send_retry_used
                        && is_websocket_request_send_closed_error(&err) =>
                {
                    stale_reused_send_retry_used = true;
                    self.reset_websocket_session();
                    debug!(
                        "discarded stale reused responses websocket after request send close; reconnecting once"
                    );
                    continue;
                }
                Err(err) => {
                    let response_debug_context =
                        extract_response_debug_context_from_api_error(&err);
                    let err = map_api_error(err);
                    inference_trace_attempt.record_failed(
                        &err,
                        response_debug_context.request_id.as_deref(),
                        /*output_items*/ &[],
                    );
                    return Err(err);
                }
            };
            let (stream, last_request_rx) = map_response_stream(
                stream_result,
                session_telemetry.clone(),
                inference_trace_attempt,
            );
            self.websocket_session.last_response_rx = Some(last_request_rx);
            return Ok(WebsocketStreamOutcome::Stream(stream));
        }
    }

    fn build_streaming_telemetry(
        session_telemetry: &SharedSessionTelemetry,
        auth_context: AuthRequestTelemetryContext,
        request_route_telemetry: RequestRouteTelemetry,
        auth_env_telemetry: AuthEnvTelemetry,
    ) -> (Arc<dyn RequestTelemetry>, Arc<dyn SseTelemetry>) {
        let telemetry = Arc::new(ApiTelemetry::new(
            session_telemetry.clone(),
            auth_context,
            request_route_telemetry,
            auth_env_telemetry,
        ));
        let request_telemetry: Arc<dyn RequestTelemetry> = telemetry.clone();
        let sse_telemetry: Arc<dyn SseTelemetry> = telemetry;
        (request_telemetry, sse_telemetry)
    }

    pub(super) fn build_websocket_telemetry(
        session_telemetry: &SharedSessionTelemetry,
        auth_context: AuthRequestTelemetryContext,
        request_route_telemetry: RequestRouteTelemetry,
        auth_env_telemetry: AuthEnvTelemetry,
    ) -> Arc<dyn WebsocketTelemetry> {
        let telemetry = Arc::new(ApiTelemetry::new(
            session_telemetry.clone(),
            auth_context,
            request_route_telemetry,
            auth_env_telemetry,
        ));
        let websocket_telemetry: Arc<dyn WebsocketTelemetry> = telemetry;
        websocket_telemetry
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn prewarm_websocket(
        &mut self,
        prompt: &Prompt,
        model_info: &ModelInfo,
        session_telemetry: &SharedSessionTelemetry,
        effort: Option<ReasoningEffortConfig>,
        summary: ReasoningSummaryConfig,
        service_tier: Option<String>,
        turn_metadata_header: Option<&str>,
    ) -> Result<()> {
        if !self.client.responses_websocket_enabled() {
            return Ok(());
        }
        if self.websocket_session.last_request.is_some() {
            return Ok(());
        }

        let disabled_trace = InferenceTraceContext::disabled();
        match self
            .stream_responses_websocket(
                prompt,
                model_info,
                session_telemetry,
                effort,
                summary,
                service_tier,
                turn_metadata_header,
                /*warmup*/ true,
                current_span_w3c_trace_context(),
                &disabled_trace,
            )
            .await
        {
            Ok(WebsocketStreamOutcome::Stream(mut stream)) => {
                while let Some(event) = stream.next().await {
                    match event {
                        Ok(ResponseEvent::Completed { .. }) => break,
                        Err(err) => return Err(err),
                        _ => {}
                    }
                }
                Ok(())
            }
            Ok(WebsocketStreamOutcome::FallbackToHttp) => {
                self.try_switch_fallback_transport(session_telemetry, model_info);
                Ok(())
            }
            Err(err) => Err(err),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn stream(
        &mut self,
        prompt: &Prompt,
        model_info: &ModelInfo,
        session_telemetry: &SharedSessionTelemetry,
        effort: Option<ReasoningEffortConfig>,
        summary: ReasoningSummaryConfig,
        service_tier: Option<String>,
        turn_metadata_header: Option<&str>,
        inference_trace: &InferenceTraceContext,
    ) -> Result<ResponseStream> {
        let wire_api = self.client.state.provider.info().wire_api;
        match wire_api {
            WireApi::Responses => {
                if self.client.responses_websocket_enabled() {
                    let request_trace = current_span_w3c_trace_context();
                    match self
                        .stream_responses_websocket(
                            prompt,
                            model_info,
                            session_telemetry,
                            effort,
                            summary,
                            service_tier.clone(),
                            turn_metadata_header,
                            /*warmup*/ false,
                            request_trace,
                            inference_trace,
                        )
                        .await?
                    {
                        WebsocketStreamOutcome::Stream(stream) => return Ok(stream),
                        WebsocketStreamOutcome::FallbackToHttp => {
                            self.try_switch_fallback_transport(session_telemetry, model_info);
                        }
                    }
                }

                self.stream_responses_api(
                    prompt,
                    model_info,
                    session_telemetry,
                    effort,
                    summary,
                    service_tier,
                    turn_metadata_header,
                    inference_trace,
                )
                .await
            }
            WireApi::ChatCompletions => {
                self.stream_chat_completions_api(
                    prompt,
                    model_info,
                    session_telemetry,
                    effort,
                    summary,
                    service_tier,
                    turn_metadata_header,
                    inference_trace,
                    ApiChatCompletionsPath::AppendChatCompletions,
                )
                .await
            }
            WireApi::AzureChatCompletions => {
                self.stream_chat_completions_api(
                    prompt,
                    model_info,
                    session_telemetry,
                    effort,
                    summary,
                    service_tier,
                    turn_metadata_header,
                    inference_trace,
                    ApiChatCompletionsPath::FullEndpoint,
                )
                .await
            }
        }
    }

    pub fn try_switch_fallback_transport(
        &mut self,
        session_telemetry: &SharedSessionTelemetry,
        model_info: &ModelInfo,
    ) -> bool {
        let activated = self
            .client
            .force_http_fallback(session_telemetry, model_info);
        self.websocket_session = WebsocketSession::default();
        activated
    }
}
