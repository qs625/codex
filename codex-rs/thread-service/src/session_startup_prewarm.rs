use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use codex_approval_service_api::is_guardian_reviewer_source;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::info;
use tracing::warn;

use crate::client_common::PromptBuildParams;
use crate::client_common::build_prompt;
use crate::session::INITIAL_SUBMIT_ID;
use crate::session::session::Session;
use crate::session::turn::built_tools;
use metrics_api::STARTUP_PREWARM_AGE_AT_FIRST_TURN_METRIC;
use metrics_api::STARTUP_PREWARM_DURATION_METRIC;
use model_service_api::OwnedModelTurnClientApi;
use model_service_api::TurnModelRequest;
use protocol::error::Result as CodexResult;
use protocol::models::BaseInstructions;
use session_telemetry_api::SharedSessionTelemetry;

pub(crate) struct SessionStartupPrewarmHandle {
    task: JoinHandle<CodexResult<OwnedModelTurnClientApi>>,
    started_at: Instant,
    timeout: Duration,
}

pub(crate) enum SessionStartupPrewarmResolution {
    Cancelled,
    Ready(OwnedModelTurnClientApi),
    Unavailable {
        status: &'static str,
        prewarm_duration: Option<Duration>,
    },
}

impl SessionStartupPrewarmHandle {
    pub(crate) fn new(
        task: JoinHandle<CodexResult<OwnedModelTurnClientApi>>,
        started_at: Instant,
        timeout: Duration,
    ) -> Self {
        Self {
            task,
            started_at,
            timeout,
        }
    }

    async fn resolve(
        self,
        session_telemetry: &SharedSessionTelemetry,
        cancellation_token: &CancellationToken,
    ) -> SessionStartupPrewarmResolution {
        let resolve_started_at = Instant::now();
        let Self {
            mut task,
            started_at,
            timeout,
        } = self;
        let age_at_first_turn = started_at.elapsed();
        let remaining = timeout.saturating_sub(age_at_first_turn);

        let resolution = if task.is_finished() {
            Self::resolution_from_join_result(task.await, started_at)
        } else {
            match tokio::select! {
                _ = cancellation_token.cancelled() => None,
                result = tokio::time::timeout(remaining, &mut task) => Some(result),
            } {
                Some(Ok(result)) => Self::resolution_from_join_result(result, started_at),
                Some(Err(_elapsed)) => {
                    task.abort();
                    info!("startup websocket prewarm timed out before the first turn could use it");
                    SessionStartupPrewarmResolution::Unavailable {
                        status: "timed_out",
                        prewarm_duration: Some(started_at.elapsed()),
                    }
                }
                None => {
                    task.abort();
                    session_telemetry.record_startup_phase(
                        "startup_prewarm_resolve",
                        resolve_started_at.elapsed(),
                        Some("cancelled"),
                    );
                    session_telemetry.record_duration(
                        STARTUP_PREWARM_AGE_AT_FIRST_TURN_METRIC,
                        age_at_first_turn,
                        &[("status", "cancelled")],
                    );
                    session_telemetry.record_duration(
                        STARTUP_PREWARM_DURATION_METRIC,
                        started_at.elapsed(),
                        &[("status", "cancelled")],
                    );
                    return SessionStartupPrewarmResolution::Cancelled;
                }
            }
        };
        let status = match &resolution {
            SessionStartupPrewarmResolution::Cancelled => "cancelled",
            SessionStartupPrewarmResolution::Ready(_) => "ready",
            SessionStartupPrewarmResolution::Unavailable { status, .. } => status,
        };
        session_telemetry.record_startup_phase(
            "startup_prewarm_resolve",
            resolve_started_at.elapsed(),
            Some(status),
        );

        match resolution {
            SessionStartupPrewarmResolution::Cancelled => {
                SessionStartupPrewarmResolution::Cancelled
            }
            SessionStartupPrewarmResolution::Ready(prewarmed_session) => {
                session_telemetry.record_duration(
                    STARTUP_PREWARM_AGE_AT_FIRST_TURN_METRIC,
                    age_at_first_turn,
                    &[("status", "consumed")],
                );
                SessionStartupPrewarmResolution::Ready(prewarmed_session)
            }
            SessionStartupPrewarmResolution::Unavailable {
                status,
                prewarm_duration,
            } => {
                session_telemetry.record_duration(
                    STARTUP_PREWARM_AGE_AT_FIRST_TURN_METRIC,
                    age_at_first_turn,
                    &[("status", status)],
                );
                if let Some(prewarm_duration) = prewarm_duration {
                    session_telemetry.record_duration(
                        STARTUP_PREWARM_DURATION_METRIC,
                        prewarm_duration,
                        &[("status", status)],
                    );
                }
                SessionStartupPrewarmResolution::Unavailable {
                    status,
                    prewarm_duration,
                }
            }
        }
    }

    fn resolution_from_join_result(
        result: std::result::Result<CodexResult<OwnedModelTurnClientApi>, tokio::task::JoinError>,
        started_at: Instant,
    ) -> SessionStartupPrewarmResolution {
        match result {
            Ok(Ok(prewarmed_session)) => SessionStartupPrewarmResolution::Ready(prewarmed_session),
            Ok(Err(err)) => {
                warn!("startup websocket prewarm setup failed: {err:#}");
                SessionStartupPrewarmResolution::Unavailable {
                    status: "failed",
                    prewarm_duration: None,
                }
            }
            Err(err) => {
                warn!("startup websocket prewarm setup join failed: {err}");
                SessionStartupPrewarmResolution::Unavailable {
                    status: "join_failed",
                    prewarm_duration: Some(started_at.elapsed()),
                }
            }
        }
    }
}

impl Session {
    pub(crate) async fn schedule_startup_prewarm(self: &Arc<Self>, base_instructions: String) {
        let session_telemetry = self.services.session_telemetry.clone();
        let websocket_connect_timeout = self.provider().await.websocket_connect_timeout();
        let started_at = Instant::now();
        let startup_prewarm_session = Arc::clone(self);
        let startup_prewarm = tokio::spawn(async move {
            let result =
                schedule_startup_prewarm_inner(startup_prewarm_session, base_instructions).await;
            let status = if result.is_ok() { "ready" } else { "failed" };
            session_telemetry.record_startup_phase(
                "startup_prewarm_total",
                started_at.elapsed(),
                Some(status),
            );
            session_telemetry.record_duration(
                STARTUP_PREWARM_DURATION_METRIC,
                started_at.elapsed(),
                &[("status", status)],
            );
            result
        });
        self.set_session_startup_prewarm(SessionStartupPrewarmHandle::new(
            startup_prewarm,
            started_at,
            websocket_connect_timeout,
        ))
        .await;
    }

    pub(crate) async fn consume_startup_prewarm_for_regular_turn(
        &self,
        cancellation_token: &CancellationToken,
    ) -> SessionStartupPrewarmResolution {
        let Some(startup_prewarm) = self.take_session_startup_prewarm().await else {
            return SessionStartupPrewarmResolution::Unavailable {
                status: "not_scheduled",
                prewarm_duration: None,
            };
        };
        startup_prewarm
            .resolve(&self.services.session_telemetry, cancellation_token)
            .await
    }
}

async fn schedule_startup_prewarm_inner(
    session: Arc<Session>,
    base_instructions: String,
) -> CodexResult<OwnedModelTurnClientApi> {
    let prewarm_started_at = Instant::now();
    let startup_turn_context = session
        .new_default_turn_with_sub_id(INITIAL_SUBMIT_ID.to_owned())
        .await;
    startup_turn_context.session_telemetry.record_startup_phase(
        "startup_prewarm_create_turn_context",
        prewarm_started_at.elapsed(),
        /*status*/ None,
    );
    let startup_cancellation_token = CancellationToken::new();
    let built_tools_started_at = Instant::now();
    let session_capability: Arc<dyn thread_service_api::ThreadSessionCapability> =
        Arc::clone(&session) as Arc<dyn thread_service_api::ThreadSessionCapability>;
    let startup_router = built_tools(
        Arc::clone(&session),
        Arc::clone(&startup_turn_context),
        Arc::downgrade(&session_capability),
        &[],
        &HashSet::new(),
        /*skills_outcome*/ None,
        &startup_cancellation_token,
    )
    .await?;
    startup_turn_context.session_telemetry.record_startup_phase(
        "startup_prewarm_build_tools",
        built_tools_started_at.elapsed(),
        /*status*/ None,
    );
    let build_prompt_started_at = Instant::now();
    let startup_prompt = build_prompt(PromptBuildParams {
        input: Vec::new(),
        tools: session.services.tool_service.model_visible_specs(
            crate::session::turn::tool_service_request(
                &session,
                &startup_turn_context,
                &startup_router,
            ),
        ),
        parallel_tool_calls: startup_turn_context.model_info.supports_parallel_tool_calls,
        base_instructions: BaseInstructions {
            text: base_instructions,
        },
        personality: startup_turn_context.personality,
        output_schema: startup_turn_context.final_output_json_schema.clone(),
        output_schema_strict: !is_guardian_reviewer_source(&startup_turn_context.session_source),
    });
    startup_turn_context.session_telemetry.record_startup_phase(
        "startup_prewarm_build_prompt",
        build_prompt_started_at.elapsed(),
        /*status*/ None,
    );
    let startup_turn_metadata_header = startup_turn_context
        .turn_metadata_state
        .current_header_value();
    let model_client_api = crate::session::turn::model_client_api_for_turn(
        session.as_ref(),
        startup_turn_context.as_ref(),
    )
    .await
    .map_err(|err| {
        protocol::error::CodexErr::Fatal(format!(
            "failed to resolve startup prewarm model client api: {err}"
        ))
    })?;
    let mut client_session = model_client_api.create_turn_client().await.map_err(|err| {
        protocol::error::CodexErr::Fatal(format!(
            "failed to create startup prewarm model client: {err}"
        ))
    })?;
    let websocket_warmup_started_at = Instant::now();
    client_session
        .prewarm_websocket(TurnModelRequest {
            request: model_service_api::ResponsesModelRequest {
                input: startup_prompt.input.clone(),
                tools: startup_prompt.tools.clone(),
                parallel_tool_calls: startup_prompt.parallel_tool_calls,
                base_instructions: startup_prompt.base_instructions.clone(),
                personality: startup_prompt.personality,
                output_schema: startup_prompt.output_schema.clone(),
                output_schema_strict: startup_prompt.output_schema_strict,
                model: Some(startup_turn_context.model_info.slug.clone()),
                reasoning_effort: startup_turn_context.reasoning_effort,
                reasoning_summary: startup_turn_context.reasoning_summary,
                service_tier: crate::session::turn::model_service_tier(
                    startup_turn_context.config.service_tier.as_deref(),
                ),
                verbosity: None,
                turn_metadata_header: startup_turn_metadata_header.clone(),
            },
            model_info: startup_turn_context.model_info.clone(),
            session_telemetry: startup_turn_context.session_telemetry.clone(),
            turn_metadata_header: startup_turn_metadata_header,
            inference_trace: rollout_trace_api::InferenceTraceContext::disabled(),
        })
        .await
        .map_err(|err| protocol::error::CodexErr::Stream(err.to_string(), None))?;
    startup_turn_context.session_telemetry.record_startup_phase(
        "startup_prewarm_websocket_warmup",
        websocket_warmup_started_at.elapsed(),
        /*status*/ None,
    );

    Ok(client_session)
}
