use super::*;
use model_service_api::CompactModelRequest;
use model_service_api::MemorySummarizeModelRequest;
use model_service_api::ModelCompactResult;
use model_service_api::ModelFuture;
use model_service_api::ModelServiceError;
use model_service_api::TurnModelRequest;

#[derive(Debug, Clone)]
pub(crate) struct LegacyModelClientAdapter {
    pub(crate) descriptor: ModelClientDescriptor,
    pub(crate) client: ModelClient,
    pub(crate) models_manager: SharedModelsManager,
    pub(crate) model_metadata_overrides: Vec<ModelMetadataOverride>,
    pub(crate) session_telemetry: SharedSessionTelemetry,
}

pub(crate) struct LegacyTurnModelClientAdapter {
    pub(crate) session: ModelClientSession,
    pub(crate) provider: Option<String>,
}

impl ModelClientApi for LegacyModelClientAdapter {
    fn descriptor(&self) -> &ModelClientDescriptor {
        &self.descriptor
    }

    fn responses_websocket_enabled(&self) -> bool {
        self.client.responses_websocket_enabled()
    }

    fn set_window_generation(&self, window_generation: u64) {
        self.client.set_window_generation(window_generation);
    }

    fn advance_window_generation(&self) {
        self.client.advance_window_generation();
    }

    fn create_turn_client(
        &self,
    ) -> ModelFuture<'_, Result<OwnedModelTurnClientApi, ModelServiceError>> {
        let client = self.client.clone();
        let provider = self.descriptor.provider.clone();
        Box::pin(async move {
            let session = client.new_session();
            Ok(Box::new(LegacyTurnModelClientAdapter { session, provider })
                as OwnedModelTurnClientApi)
        })
    }

    fn prepare_realtime_transport(
        &self,
        request: PrepareRealtimeTransportRequest,
    ) -> ModelFuture<'_, Result<PreparedRealtimeTransport, ModelRequestError>> {
        let client = self.client.clone();
        Box::pin(async move {
            let provider_info = client.provider_info().clone();
            let auth_manager = client.auth_manager();
            let auth = match auth_manager {
                Some(manager) => manager.auth().await,
                None => None,
            };
            let api_provider = build_realtime_api_provider(
                &provider_info,
                auth.as_ref(),
                request.websocket_base_url.as_deref(),
            )
            .map_err(|err| ModelRequestError::new(err.to_string()))?;
            let extra_headers = if request.include_api_key_header {
                let realtime_api_key = realtime_api_key(auth.as_ref(), &provider_info)
                    .map_err(|err| ModelRequestError::new(err.to_string()))?;
                realtime_request_headers(
                    request.requested_realtime_session_id.as_deref(),
                    Some(realtime_api_key.as_str()),
                )
            } else {
                realtime_request_headers(request.requested_realtime_session_id.as_deref(), None)
            }
            .map_err(|err| ModelRequestError::new(err.to_string()))?;
            Ok(PreparedRealtimeTransport {
                api_provider,
                extra_headers,
            })
        })
    }

    fn realtime_websocket_client(
        &self,
        api_provider: model_service_api::Provider,
    ) -> Box<dyn model_service_api::RealtimeWebsocketClientRuntime> {
        self.client.realtime_websocket_client(api_provider)
    }

    fn create_realtime_call_with_transport(
        &self,
        request: RealtimeWebrtcCallRequest,
    ) -> ModelFuture<'_, Result<RealtimeWebrtcCallHandle, ModelRequestError>> {
        let client = self.client.clone();
        Box::pin(async move {
            let result = client
                .create_realtime_call_with_headers(
                    request.sdp,
                    request.api_provider,
                    request.session_config,
                    request.extra_headers,
                )
                .await
                .map_err(|err| ModelRequestError::new(err.to_string()))?;
            Ok(RealtimeWebrtcCallHandle {
                sdp: result.sdp,
                call_id: result.call_id,
                sideband_headers: result.sideband_headers,
            })
        })
    }

    fn stream_responses(
        &self,
        request: ResponsesModelRequest,
    ) -> ModelFuture<'_, Result<ModelResponseStream, ModelRequestError>> {
        let client = self.client.clone();
        let models_manager = self.models_manager.clone();
        let config = ModelsManagerConfig {
            model_metadata_overrides: self.model_metadata_overrides.clone(),
            ..Default::default()
        };
        let session_telemetry = self.session_telemetry.clone();
        let descriptor_model = self.descriptor.model.clone();
        Box::pin(async move {
            let model = request
                .model
                .clone()
                .or_else(|| (!descriptor_model.is_empty()).then_some(descriptor_model.clone()))
                .ok_or_else(|| ModelRequestError::new("responses request is missing a model"))?;
            let model_info = models_manager.get_model_info(&model, &config).await;
            let prompt = Prompt {
                input: request.input,
                tools: request.tools,
                parallel_tool_calls: request.parallel_tool_calls,
                base_instructions: request.base_instructions,
                personality: request.personality,
                output_schema: request.output_schema,
                output_schema_strict: request.output_schema_strict,
            };
            let mut session = client.new_session();
            let stream = session
                .stream(
                    &prompt,
                    &model_info,
                    &session_telemetry,
                    request.reasoning_effort,
                    request.reasoning_summary,
                    request.service_tier.map(|tier| tier.to_string()),
                    request.turn_metadata_header.as_deref(),
                    &InferenceTraceContext::disabled(),
                )
                .await
                .map_err(|err| ModelRequestError::new(err.to_string()))?;
            Ok(Box::pin(stream.map(|event| {
                event
                    .map(map_legacy_response_event)
                    .map_err(|err| ModelRequestError::new(err.to_string()))
            })) as ModelResponseStream)
        })
    }

    fn create_realtime_call(
        &self,
        request: RealtimeModelRequest,
    ) -> ModelFuture<'_, Result<RealtimeCallHandle, ModelRequestError>> {
        let client = self.client.clone();
        let provider_info = client.provider_info().clone();
        let auth_manager = client.auth_manager();
        let descriptor_model = self.descriptor.model.clone();
        Box::pin(async move {
            let auth = match auth_manager {
                Some(manager) => manager.auth().await,
                None => None,
            };
            let api_provider = build_realtime_api_provider(&provider_info, auth.as_ref(), None)
                .map_err(|err| ModelRequestError::new(err.to_string()))?;
            let api_key = realtime_api_key(auth.as_ref(), &provider_info)
                .map_err(|err| ModelRequestError::new(err.to_string()))?;
            let extra_headers =
                realtime_request_headers(request.realtime_session_id.as_deref(), Some(&api_key))
                    .map_err(|err| ModelRequestError::new(err.to_string()))?
                    .unwrap_or_default();
            let output_modality = request
                .output_modality
                .unwrap_or(RealtimeOutputModality::Audio);
            let voice = request.voice.unwrap_or_else(|| {
                default_realtime_voice(protocol::protocol::RealtimeConversationVersion::V2)
            });
            let model = request
                .model
                .clone()
                .or_else(|| (!descriptor_model.is_empty()).then_some(descriptor_model.clone()));
            let session_config = RealtimeSessionConfig {
                instructions: request.prompt.unwrap_or_default(),
                model,
                session_id: request.realtime_session_id,
                event_parser: RealtimeEventParser::RealtimeV2,
                session_mode: RealtimeSessionMode::Conversational,
                output_modality,
                voice,
            };
            let result = client
                .create_realtime_call_with_headers(
                    request.sdp,
                    api_provider,
                    session_config,
                    extra_headers,
                )
                .await
                .map_err(|err| ModelRequestError::new(err.to_string()))?;
            Ok(RealtimeCallHandle {
                sdp: Some(result.sdp),
                call_id: result.call_id,
            })
        })
    }

    fn compact(
        &self,
        request: CompactModelRequest,
    ) -> ModelFuture<'_, Result<ModelCompactResult, ModelRequestError>> {
        let client = self.client.clone();
        let models_manager = self.models_manager.clone();
        let config = ModelsManagerConfig {
            model_metadata_overrides: self.model_metadata_overrides.clone(),
            ..Default::default()
        };
        let session_telemetry = self.session_telemetry.clone();
        let descriptor_model = self.descriptor.model.clone();
        Box::pin(async move {
            let model = request
                .model
                .clone()
                .or_else(|| (!descriptor_model.is_empty()).then_some(descriptor_model.clone()))
                .ok_or_else(|| ModelRequestError::new("compact request is missing a model"))?;
            let model_info = models_manager.get_model_info(&model, &config).await;
            let prompt = Prompt {
                input: request.input,
                tools: Vec::new(),
                parallel_tool_calls: false,
                base_instructions: request.base_instructions,
                personality: None,
                output_schema: request.output_schema,
                output_schema_strict: request.output_schema_strict,
            };
            let items = client
                .compact_conversation_history(
                    &prompt,
                    &model_info,
                    CompactConversationRequestSettings {
                        effort: request.reasoning_effort,
                        summary: request.reasoning_summary,
                        service_tier: request.service_tier.map(|tier| tier.to_string()),
                    },
                    &session_telemetry,
                    &CompactionTraceContext::disabled(),
                )
                .await
                .map_err(|err| ModelRequestError::new(err.to_string()))?;
            Ok(ModelCompactResult { items })
        })
    }

    fn summarize_memories(
        &self,
        request: MemorySummarizeModelRequest,
    ) -> ModelFuture<'_, Result<Vec<ModelMemorySummary>, ModelRequestError>> {
        let client = self.client.clone();
        let models_manager = self.models_manager.clone();
        let config = ModelsManagerConfig {
            model_metadata_overrides: self.model_metadata_overrides.clone(),
            ..Default::default()
        };
        let session_telemetry = self.session_telemetry.clone();
        let descriptor_model = self.descriptor.model.clone();
        Box::pin(async move {
            let model = request
                .model
                .clone()
                .or_else(|| (!descriptor_model.is_empty()).then_some(descriptor_model.clone()))
                .ok_or_else(|| {
                    ModelRequestError::new("memory summarize request is missing a model")
                })?;
            let model_info = models_manager.get_model_info(&model, &config).await;
            let raw_memories = request
                .raw_memories
                .into_iter()
                .map(|memory| RawMemory {
                    id: memory.id,
                    metadata: RawMemoryMetadata {
                        source_path: memory.source_path,
                    },
                    items: memory.items,
                })
                .collect();
            let summaries = client
                .summarize_memories(
                    raw_memories,
                    &model_info,
                    request.reasoning_effort,
                    &session_telemetry,
                )
                .await
                .map_err(|err| ModelRequestError::new(err.to_string()))?;
            Ok(summaries
                .into_iter()
                .map(|summary| ModelMemorySummary {
                    raw_memory: summary.raw_memory,
                    memory_summary: summary.memory_summary,
                })
                .collect())
        })
    }
}

impl ModelTurnClientApi for LegacyTurnModelClientAdapter {
    fn provider(&self) -> Option<&str> {
        self.provider.as_deref()
    }

    fn reset_websocket_session(&mut self) {
        self.session.reset_websocket_session();
    }

    fn send_response_processed<'a>(&'a self, response_id: &'a str) -> ModelFuture<'a, ()> {
        Box::pin(async move {
            self.session.send_response_processed(response_id).await;
        })
    }

    fn prewarm_websocket(
        &mut self,
        request: TurnModelRequest,
    ) -> ModelFuture<'_, Result<(), ModelRequestError>> {
        Box::pin(async move {
            let prompt = Prompt {
                input: request.request.input,
                tools: request.request.tools,
                parallel_tool_calls: request.request.parallel_tool_calls,
                base_instructions: request.request.base_instructions,
                personality: request.request.personality,
                output_schema: request.request.output_schema,
                output_schema_strict: request.request.output_schema_strict,
            };
            self.session
                .prewarm_websocket(
                    &prompt,
                    &request.model_info,
                    &request.session_telemetry,
                    request.request.reasoning_effort,
                    request.request.reasoning_summary,
                    request
                        .request
                        .service_tier
                        .map(|service_tier| service_tier.to_string()),
                    request.turn_metadata_header.as_deref(),
                )
                .await
                .map_err(ModelRequestError::from_codex_err)
        })
    }

    fn stream_responses(
        &mut self,
        request: TurnModelRequest,
    ) -> ModelFuture<'_, Result<ModelResponseStream, ModelRequestError>> {
        Box::pin(async move {
            let prompt = Prompt {
                input: request.request.input,
                tools: request.request.tools,
                parallel_tool_calls: request.request.parallel_tool_calls,
                base_instructions: request.request.base_instructions,
                personality: request.request.personality,
                output_schema: request.request.output_schema,
                output_schema_strict: request.request.output_schema_strict,
            };
            let stream = self
                .session
                .stream(
                    &prompt,
                    &request.model_info,
                    &request.session_telemetry,
                    request.request.reasoning_effort,
                    request.request.reasoning_summary,
                    request
                        .request
                        .service_tier
                        .map(|service_tier| service_tier.to_string()),
                    request.turn_metadata_header.as_deref(),
                    &request.inference_trace,
                )
                .await
                .map_err(ModelRequestError::from_codex_err)?;
            Ok(Box::pin(stream.map(|event| {
                event
                    .map(map_legacy_response_event)
                    .map_err(ModelRequestError::from_codex_err)
            })) as ModelResponseStream)
        })
    }

    fn try_switch_fallback_transport(
        &mut self,
        session_telemetry: SharedSessionTelemetry,
        model_info: ModelInfo,
    ) -> bool {
        self.session
            .try_switch_fallback_transport(&session_telemetry, &model_info)
    }
}

pub(crate) fn map_legacy_response_event(event: LegacyResponseEvent) -> ModelResponseEvent {
    match event {
        LegacyResponseEvent::Created => ModelResponseEvent::Created,
        LegacyResponseEvent::OutputItemDone(item) => ModelResponseEvent::ItemDone { item },
        LegacyResponseEvent::OutputItemAdded(item) => ModelResponseEvent::ItemAdded { item },
        LegacyResponseEvent::ServerModel(model) => ModelResponseEvent::ServerModel { model },
        LegacyResponseEvent::ModelVerifications(verifications) => {
            ModelResponseEvent::ModelVerifications { verifications }
        }
        LegacyResponseEvent::ServerReasoningIncluded(included) => {
            ModelResponseEvent::ServerReasoningIncluded { included }
        }
        LegacyResponseEvent::Completed {
            response_id,
            token_usage,
            end_turn,
        } => ModelResponseEvent::Completed {
            response_id,
            token_usage,
            end_turn,
        },
        LegacyResponseEvent::OutputTextDelta(delta) => {
            ModelResponseEvent::OutputTextDelta { delta }
        }
        LegacyResponseEvent::ToolCallInputDelta {
            item_id,
            call_id,
            delta,
        } => ModelResponseEvent::ToolCallInputDelta {
            item_id,
            call_id,
            delta,
        },
        LegacyResponseEvent::ReasoningSummaryDelta {
            delta,
            summary_index,
        } => ModelResponseEvent::ReasoningSummaryDelta {
            delta,
            summary_index,
        },
        LegacyResponseEvent::ReasoningContentDelta {
            delta,
            content_index,
        } => ModelResponseEvent::ReasoningContentDelta {
            delta,
            content_index,
        },
        LegacyResponseEvent::ReasoningSummaryPartAdded { summary_index } => {
            ModelResponseEvent::ReasoningSummaryPartAdded { summary_index }
        }
        LegacyResponseEvent::RateLimits(snapshot) => ModelResponseEvent::RateLimits { snapshot },
        LegacyResponseEvent::ModelsEtag(etag) => ModelResponseEvent::ModelsEtag { etag },
    }
}
