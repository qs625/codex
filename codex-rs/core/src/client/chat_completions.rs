use super::*;

impl ModelClientSession {
    #[allow(clippy::too_many_arguments)]
    #[instrument(
        name = "model_client.stream_chat_completions_api",
        level = "info",
        skip_all,
        fields(
            model = %model_info.slug,
            wire_api = %self.client.state.provider.info().wire_api,
            transport = "chat_completions_http",
            http.method = "POST",
            api.path = CHAT_COMPLETIONS_ENDPOINT,
            turn.has_metadata_header = turn_metadata_header.is_some()
        )
    )]
    pub(super) async fn stream_chat_completions_api(
        &self,
        prompt: &Prompt,
        model_info: &ModelInfo,
        session_telemetry: &SessionTelemetry,
        effort: Option<ReasoningEffortConfig>,
        summary: ReasoningSummaryConfig,
        service_tier: Option<String>,
        turn_metadata_header: Option<&str>,
        inference_trace: &InferenceTraceContext,
        path: ApiChatCompletionsPath,
    ) -> Result<ResponseStream> {
        let auth_manager = self.client.state.provider.auth_manager();
        let mut auth_recovery = auth_manager
            .as_ref()
            .map(AuthManager::unauthorized_recovery);
        let mut pending_retry = PendingUnauthorizedRetry::default();
        loop {
            let client_setup = self.client.current_client_setup().await?;
            let transport = ReqwestTransport::new(build_reqwest_client());
            let request_auth_context = AuthRequestTelemetryContext::new(
                client_setup.auth.as_ref().map(CodexAuth::auth_mode),
                client_setup.api_auth.as_ref(),
                pending_retry,
            );
            let request_telemetry = ModelClient::build_request_telemetry(
                session_telemetry,
                request_auth_context,
                RequestRouteTelemetry::for_endpoint(CHAT_COMPLETIONS_ENDPOINT),
                self.client.state.auth_env_telemetry.clone(),
            );
            let mut extra_headers = self
                .build_chat_completions_headers(turn_metadata_header)
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
            inference_trace_attempt.add_request_headers(&mut extra_headers);
            inference_trace_attempt.record_started(&request);
            let client = ApiChatCompletionsClient::new(
                transport,
                client_setup.api_provider,
                client_setup.api_auth,
                path,
            )
            .with_telemetry(Some(request_telemetry));
            let stream_result = client.create(request, extra_headers).await;

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

    async fn build_chat_completions_headers(
        &self,
        turn_metadata_header: Option<&str>,
    ) -> ApiHeaderMap {
        let turn_metadata_header = parse_turn_metadata_header(turn_metadata_header);
        let session_id = self.client.state.session_id.to_string();
        let thread_id = self.client.state.thread_id.to_string();
        let mut headers = build_responses_headers(
            self.client.state.beta_features_header.as_deref(),
            Some(&self.turn_state),
            turn_metadata_header.as_ref(),
        );
        headers.extend(self.client.build_responses_identity_headers());
        headers.extend(build_session_headers(Some(session_id), Some(thread_id)));
        if let Some(header_value) = self.client.generate_attestation_header_for().await {
            headers.insert(X_OAI_ATTESTATION_HEADER, header_value);
        }
        headers
    }
}
