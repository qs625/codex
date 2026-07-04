use super::*;

impl ModelClient {
    #[allow(clippy::too_many_arguments)]
    /// Creates a new session-scoped `ModelClient`.
    ///
    /// All arguments are expected to be stable for the lifetime of a Codex session. Per-turn values
    /// are passed to [`ModelClientSession::stream`] (and other turn-scoped methods) explicitly.
    pub fn new(
        provider_auth_manager: Option<SharedModelProviderAuthManager>,
        session_id: SessionId,
        thread_id: ThreadId,
        installation_id: String,
        api_runtime_factory: SharedApiRuntimeFactory,
        model_provider_factory: SharedModelProviderFactory,
        provider_info: ModelProviderInfo,
        session_source: SessionSource,
        model_verbosity: Option<VerbosityConfig>,
        chat_completions_max_tokens_by_model: HashMap<String, u64>,
        enable_request_compression: bool,
        include_timing_metrics: bool,
        beta_features_header: Option<String>,
        attestation_provider: Option<Arc<dyn AttestationProvider>>,
    ) -> Self {
        let model_provider =
            model_provider_factory.create_model_provider(provider_info, provider_auth_manager);
        let codex_api_key_env_enabled = model_provider
            .auth_manager()
            .as_ref()
            .is_some_and(|manager| manager.codex_api_key_env_enabled());
        let auth_env_telemetry = collect_auth_env_telemetry(AuthEnvTelemetryInput {
            provider_env_key: model_provider.info().env_key.as_deref(),
            codex_api_key_env_enabled,
        });
        let include_attestation = model_provider.supports_attestation();
        Self {
            state: Arc::new(ModelClientState {
                session_id,
                thread_id,
                window_generation: AtomicU64::new(0),
                installation_id,
                api_runtime_factory,
                model_provider_factory,
                provider: model_provider,
                auth_env_telemetry,
                session_source,
                model_verbosity,
                chat_completions_max_tokens_by_model,
                enable_request_compression,
                include_timing_metrics,
                beta_features_header,
                include_attestation,
                attestation_provider,
                disable_websockets: AtomicBool::new(false),
                cached_websocket_session: StdMutex::new(WebsocketSession::default()),
            }),
        }
    }

    /// Creates a fresh turn-scoped streaming session.
    ///
    /// This constructor does not perform network I/O itself; the session opens a websocket lazily
    /// when the first stream request is issued.
    pub fn new_session(&self) -> ModelClientSession {
        ModelClientSession {
            client: self.clone(),
            websocket_session: self.take_cached_websocket_session(),
            turn_state: Arc::new(OnceLock::new()),
        }
    }

    pub fn new_session_for_provider(
        &self,
        provider_auth_manager: Option<SharedModelProviderAuthManager>,
        provider_info: ModelProviderInfo,
    ) -> ModelClientSession {
        if self.state.provider.info() == &provider_info {
            return self.new_session();
        }

        self.clone_with_provider(provider_auth_manager, provider_info)
            .new_session()
    }

    fn clone_with_provider(
        &self,
        provider_auth_manager: Option<SharedModelProviderAuthManager>,
        provider_info: ModelProviderInfo,
    ) -> Self {
        let model_provider = self
            .state
            .model_provider_factory
            .create_model_provider(provider_info, provider_auth_manager);
        let codex_api_key_env_enabled = model_provider
            .auth_manager()
            .as_ref()
            .is_some_and(|manager| manager.codex_api_key_env_enabled());
        let auth_env_telemetry = collect_auth_env_telemetry(AuthEnvTelemetryInput {
            provider_env_key: model_provider.info().env_key.as_deref(),
            codex_api_key_env_enabled,
        });
        let include_attestation = model_provider.supports_attestation();
        Self {
            state: Arc::new(ModelClientState {
                session_id: self.state.session_id,
                thread_id: self.state.thread_id,
                window_generation: AtomicU64::new(
                    self.state.window_generation.load(Ordering::Relaxed),
                ),
                installation_id: self.state.installation_id.clone(),
                api_runtime_factory: Arc::clone(&self.state.api_runtime_factory),
                model_provider_factory: Arc::clone(&self.state.model_provider_factory),
                provider: model_provider,
                auth_env_telemetry,
                session_source: self.state.session_source.clone(),
                model_verbosity: self.state.model_verbosity,
                chat_completions_max_tokens_by_model: self
                    .state
                    .chat_completions_max_tokens_by_model
                    .clone(),
                enable_request_compression: self.state.enable_request_compression,
                include_timing_metrics: self.state.include_timing_metrics,
                beta_features_header: self.state.beta_features_header.clone(),
                include_attestation,
                attestation_provider: self.state.attestation_provider.clone(),
                disable_websockets: AtomicBool::new(
                    self.state.disable_websockets.load(Ordering::Relaxed),
                ),
                cached_websocket_session: StdMutex::new(WebsocketSession::default()),
            }),
        }
    }

    pub fn auth_manager(&self) -> Option<SharedModelProviderAuthManager> {
        self.state.provider.auth_manager()
    }

    pub fn provider_info(&self) -> &ModelProviderInfo {
        self.state.provider.info()
    }

    pub fn set_window_generation(&self, window_generation: u64) {
        self.state
            .window_generation
            .store(window_generation, Ordering::Relaxed);
        self.store_cached_websocket_session(WebsocketSession::default());
    }

    pub fn advance_window_generation(&self) {
        self.state.window_generation.fetch_add(1, Ordering::Relaxed);
        self.store_cached_websocket_session(WebsocketSession::default());
    }

    pub(super) fn current_window_id(&self) -> String {
        let thread_id = self.state.thread_id;
        let window_generation = self.state.window_generation.load(Ordering::Relaxed);
        format!("{thread_id}:{window_generation}")
    }

    fn take_cached_websocket_session(&self) -> WebsocketSession {
        let mut cached_websocket_session = self
            .state
            .cached_websocket_session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::take(&mut *cached_websocket_session)
    }

    pub(super) fn store_cached_websocket_session(&self, websocket_session: WebsocketSession) {
        *self
            .state
            .cached_websocket_session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = websocket_session;
    }

    pub(crate) fn force_http_fallback(
        &self,
        session_telemetry: &SharedSessionTelemetry,
        _model_info: &ModelInfo,
    ) -> bool {
        let websocket_enabled = self.responses_websocket_enabled();
        let activated =
            websocket_enabled && !self.state.disable_websockets.swap(true, Ordering::Relaxed);
        if activated {
            warn!("falling back to HTTP");
            session_telemetry.counter(
                "codex.transport.fallback_to_http",
                /*inc*/ 1,
                &[("from_wire_api", "responses_websocket")],
            );
        }

        self.store_cached_websocket_session(WebsocketSession::default());
        activated
    }

    /// Compacts the current conversation history using the Compact endpoint.
    ///
    /// This is a unary call (no streaming) that returns a new list of
    /// `ResponseItem`s representing the compacted transcript.
    ///
    /// The model selection and telemetry context are passed explicitly to keep `ModelClient`
    /// session-scoped.
    pub async fn compact_conversation_history(
        &self,
        prompt: &Prompt,
        model_info: &ModelInfo,
        settings: CompactConversationRequestSettings,
        session_telemetry: &SharedSessionTelemetry,
        compaction_trace: &CompactionTraceContext,
    ) -> Result<Vec<ResponseItem>> {
        if prompt.input.is_empty() {
            return Ok(Vec::new());
        }
        let client_setup = self.current_client_setup().await?;
        let request_telemetry = Self::build_request_telemetry(
            session_telemetry,
            AuthRequestTelemetryContext::new(
                client_setup
                    .auth
                    .as_ref()
                    .map(codex_auth_types::RequestAuthSnapshot::auth_mode),
                client_setup.api_auth.as_ref(),
                PendingUnauthorizedRetry::default(),
            ),
            RequestRouteTelemetry::for_endpoint(RESPONSES_COMPACT_ENDPOINT),
            self.state.auth_env_telemetry.clone(),
        );
        let request = self.build_responses_request(
            &client_setup.api_provider,
            prompt,
            model_info,
            settings.effort,
            settings.summary,
            settings.service_tier,
        )?;
        let ResponsesApiRequest {
            model,
            instructions,
            input,
            tools,
            parallel_tool_calls,
            reasoning,
            service_tier,
            prompt_cache_key,
            text,
            ..
        } = request;
        let payload = ApiCompactionInput {
            model: &model,
            input: &input,
            instructions: &instructions,
            tools,
            parallel_tool_calls,
            reasoning,
            service_tier: service_tier.as_deref(),
            prompt_cache_key: prompt_cache_key.as_deref(),
            text,
        };

        let mut extra_headers = ApiHeaderMap::new();
        if let Ok(header_value) = HeaderValue::from_str(&self.state.installation_id) {
            extra_headers.insert(X_CODEX_INSTALLATION_ID_HEADER, header_value);
        }
        extra_headers.extend(build_responses_headers(
            self.state.beta_features_header.as_deref(),
            /*turn_state*/ None,
            /*turn_metadata_header*/ None,
        ));
        extra_headers.extend(self.build_responses_identity_headers());
        extra_headers.extend(build_session_headers(
            Some(self.state.session_id.to_string()),
            Some(self.state.thread_id.to_string()),
        ));
        if let Some(header_value) = self.generate_attestation_header_for().await {
            extra_headers.insert(X_OAI_ATTESTATION_HEADER, header_value);
        }
        let client = self
            .state
            .api_runtime_factory
            .compact_client(client_setup.api_provider, client_setup.api_auth);
        let trace_attempt = compaction_trace.start_attempt(&payload);
        let result = client
            .compact_input(CompactInputRuntimeRequest {
                input: &payload,
                extra_headers,
                request_telemetry: Some(request_telemetry),
            })
            .await
            .map_err(map_api_error);
        trace_attempt.record_result(result.as_deref());
        result
    }

    pub async fn summarize_memories(
        &self,
        raw_memories: Vec<ApiRawMemory>,
        model_info: &ModelInfo,
        effort: Option<ReasoningEffortConfig>,
        session_telemetry: &SharedSessionTelemetry,
    ) -> Result<Vec<ApiMemorySummarizeOutput>> {
        if raw_memories.is_empty() {
            return Ok(Vec::new());
        }

        let client_setup = self.current_client_setup().await?;
        let request_telemetry = Self::build_request_telemetry(
            session_telemetry,
            AuthRequestTelemetryContext::new(
                client_setup
                    .auth
                    .as_ref()
                    .map(codex_auth_types::RequestAuthSnapshot::auth_mode),
                client_setup.api_auth.as_ref(),
                PendingUnauthorizedRetry::default(),
            ),
            RequestRouteTelemetry::for_endpoint(MEMORIES_SUMMARIZE_ENDPOINT),
            self.state.auth_env_telemetry.clone(),
        );
        let client = self
            .state
            .api_runtime_factory
            .memories_client(client_setup.api_provider, client_setup.api_auth);

        let payload = ApiMemorySummarizeInput {
            model: model_info.slug.clone(),
            raw_memories,
            reasoning: effort.map(|effort| Reasoning {
                effort: Some(effort),
                summary: None,
            }),
        };

        client
            .summarize_input(MemorySummarizeRuntimeRequest {
                input: &payload,
                extra_headers: self.build_subagent_headers(),
                request_telemetry: Some(request_telemetry),
            })
            .await
            .map_err(map_api_error)
    }

    pub(super) fn build_request_telemetry(
        session_telemetry: &SharedSessionTelemetry,
        auth_context: AuthRequestTelemetryContext,
        request_route_telemetry: RequestRouteTelemetry,
        auth_env_telemetry: AuthEnvTelemetry,
    ) -> Arc<dyn RequestTelemetry> {
        let telemetry = Arc::new(ApiTelemetry::new(
            session_telemetry.clone(),
            auth_context,
            request_route_telemetry,
            auth_env_telemetry,
        ));
        let request_telemetry: Arc<dyn RequestTelemetry> = telemetry;
        request_telemetry
    }

    fn build_reasoning(
        model_info: &ModelInfo,
        effort: Option<ReasoningEffortConfig>,
        summary: ReasoningSummaryConfig,
    ) -> Option<Reasoning> {
        if model_info.supports_reasoning_summaries {
            Some(Reasoning {
                effort: effort.or(model_info.default_reasoning_level),
                summary: if summary == ReasoningSummaryConfig::None {
                    None
                } else {
                    Some(summary)
                },
            })
        } else {
            None
        }
    }

    pub(super) fn build_responses_request(
        &self,
        provider: &ApiProvider,
        prompt: &Prompt,
        model_info: &ModelInfo,
        effort: Option<ReasoningEffortConfig>,
        summary: ReasoningSummaryConfig,
        service_tier: Option<String>,
    ) -> Result<ResponsesApiRequest> {
        let instructions = &prompt.base_instructions.text;
        let input = prompt.get_formatted_input();
        let tools = create_tools_json_for_responses_api(&prompt.tools)?;
        let reasoning = Self::build_reasoning(model_info, effort, summary);
        let include = if reasoning.is_some() {
            vec!["reasoning.encrypted_content".to_string()]
        } else {
            Vec::new()
        };
        let verbosity = if model_info.support_verbosity {
            self.state.model_verbosity.or(model_info.default_verbosity)
        } else {
            if self.state.model_verbosity.is_some() {
                warn!(
                    "model_verbosity is set but ignored as the model does not support verbosity: {}",
                    model_info.slug
                );
            }
            None
        };
        let text = create_text_param_for_request(
            verbosity,
            &prompt.output_schema,
            prompt.output_schema_strict,
        );
        let prompt_cache_key = Some(self.state.thread_id.to_string());
        let service_tier =
            service_tier.filter(|service_tier| model_info.supports_service_tier(service_tier));
        let request = ResponsesApiRequest {
            model: model_info.slug.clone(),
            instructions: instructions.clone(),
            input,
            tools,
            tool_choice: "auto".to_string(),
            parallel_tool_calls: prompt.parallel_tool_calls,
            reasoning,
            store: provider.is_azure_responses_endpoint(),
            stream: true,
            include,
            service_tier,
            prompt_cache_key,
            text,
            client_metadata: Some(HashMap::from([(
                X_CODEX_INSTALLATION_ID_HEADER.to_string(),
                self.state.installation_id.clone(),
            )])),
            chat_completions_max_tokens: self
                .state
                .chat_completions_max_tokens_by_model
                .get(&model_info.slug)
                .copied(),
        };
        Ok(request)
    }

    pub fn responses_websocket_enabled(&self) -> bool {
        if !self.state.provider.info().supports_websockets
            || self.state.disable_websockets.load(Ordering::Relaxed)
        {
            return false;
        }

        true
    }

    pub(super) async fn current_client_setup(&self) -> Result<CurrentClientSetup> {
        let auth = self.state.provider.auth().await;
        let api_provider = self.state.provider.api_provider().await?;
        let api_auth = self.state.provider.api_auth().await?;
        Ok(CurrentClientSetup {
            auth,
            api_provider,
            api_auth,
        })
    }

}
