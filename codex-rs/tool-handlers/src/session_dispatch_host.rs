use std::sync::Arc;
use std::sync::Weak;

use codex_protocol::ThreadId;
use codex_protocol::protocol::SessionSource;
use codex_thread_api::ToolSessionCapability;
use codex_thread_api::ToolSessionDispatchTrace;
use codex_thread_api::ToolTurnCapability;
use codex_session_telemetry_api::DisabledSessionTelemetryFactory;
use codex_session_telemetry_api::SessionTelemetryCreateParams;
use codex_session_telemetry_api::SessionTelemetryFactory;
use codex_session_telemetry_api::SharedSessionTelemetry;
use codex_tool_runtime::ToolDispatchHost;
use codex_tool_runtime::ToolDispatchTraceHandle;
use codex_tool_runtime::ToolInvocation;
use codex_tool_runtime::ToolTelemetryTags;
use codex_tool_runtime_api::PostToolUseHookOutcome;
use codex_tool_runtime_api::PostToolUsePayload;
use codex_tool_runtime_api::PreToolUseHookOutcome;
use codex_tool_runtime_api::PreToolUsePayload;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolName;
use codex_tool_types::ToolOutput;
use codex_tool_types::ToolPayload;
use tracing::warn;

/// Tool-service dispatch host backed by a session capability.
///
/// This type belongs to the tool owner boundary: it implements the generic
/// tool runtime dispatch hooks and calls only `codex-thread-api` capability
/// methods. It must not depend on concrete thread-runtime types.
pub struct SessionToolDispatchHost {
    session: Weak<dyn ToolSessionCapability>,
    fallback_telemetry: SharedSessionTelemetry,
}

impl SessionToolDispatchHost {
    pub fn new(session: Weak<dyn ToolSessionCapability>) -> Self {
        Self {
            session,
            fallback_telemetry: disabled_tool_dispatch_telemetry(),
        }
    }
}

fn disabled_tool_dispatch_telemetry() -> SharedSessionTelemetry {
    DisabledSessionTelemetryFactory.create(SessionTelemetryCreateParams {
        conversation_id: ThreadId::new(),
        model: "disabled".to_string(),
        slug: "disabled".to_string(),
        account_id: None,
        account_email: None,
        auth_mode: None,
        auth_env: Default::default(),
        originator: "tool-dispatch-fallback".to_string(),
        log_user_prompts: false,
        terminal_type: "unknown".to_string(),
        session_source: SessionSource::Unknown,
        metrics_service_name: None,
    })
}

fn missing_dispatch_session_message() -> String {
    "tool dispatch session capability is no longer available".to_string()
}

fn upgrade_dispatch_session_capability(
    session: &Weak<dyn ToolSessionCapability>,
) -> Result<Arc<dyn ToolSessionCapability>, String> {
    session
        .upgrade()
        .ok_or_else(missing_dispatch_session_message)
}

pub struct SessionToolDispatchTrace {
    inner: Box<dyn ToolSessionDispatchTrace>,
}

impl<Session, Turn, Tracker> ToolDispatchTraceHandle<ToolInvocation<Session, Turn, Tracker>>
    for SessionToolDispatchTrace
{
    fn record_completed(
        &self,
        _invocation: &ToolInvocation<Session, Turn, Tracker>,
        call_id: &str,
        payload: &ToolPayload,
        result: &dyn ToolOutput,
    ) {
        self.inner.record_completed(call_id, payload, result);
    }

    fn record_failed(&self, error: &FunctionCallError) {
        self.inner.record_failed(error);
    }
}

impl<Session, Turn, Tracker> ToolDispatchHost<ToolInvocation<Session, Turn, Tracker>>
    for SessionToolDispatchHost
where
    Turn: ToolTurnCapability,
{
    type Trace = SessionToolDispatchTrace;

    fn telemetry(
        &self,
        invocation: &ToolInvocation<Session, Turn, Tracker>,
    ) -> SharedSessionTelemetry {
        upgrade_dispatch_session_capability(&self.session)
            .map(|session| session.tool_dispatch_telemetry(&invocation.turn))
            .unwrap_or_else(|_| Arc::clone(&self.fallback_telemetry))
    }

    fn base_tool_result_tags(
        &self,
        invocation: &ToolInvocation<Session, Turn, Tracker>,
    ) -> ToolTelemetryTags {
        upgrade_dispatch_session_capability(&self.session)
            .map(|session| session.base_tool_result_tags(&invocation.turn))
            .unwrap_or_default()
    }

    fn record_tool_call_started<'a>(
        &'a self,
        invocation: &'a ToolInvocation<Session, Turn, Tracker>,
    ) -> impl std::future::Future<Output = ()> + Send + 'a {
        let turn = &invocation.turn;
        async move {
            let Ok(session) = upgrade_dispatch_session_capability(&self.session) else {
                warn!("{}", missing_dispatch_session_message());
                return;
            };
            session.record_tool_call_started(turn).await;
        }
    }

    fn start_trace(&self, invocation: &ToolInvocation<Session, Turn, Tracker>) -> Self::Trace {
        let inner = upgrade_dispatch_session_capability(&self.session)
            .map(|session| {
                session.start_tool_dispatch_trace(
                    &invocation.turn,
                    &invocation.call_id,
                    &invocation.tool_name,
                    &invocation.source,
                    &invocation.payload,
                )
            })
            .unwrap_or_else(|_| Box::new(NoopToolDispatchTrace));
        SessionToolDispatchTrace { inner }
    }

    fn run_pre_tool_use_hooks<'a>(
        &'a self,
        invocation: &'a ToolInvocation<Session, Turn, Tracker>,
        payload: PreToolUsePayload,
    ) -> impl std::future::Future<Output = PreToolUseHookOutcome> + Send + 'a {
        let turn = &invocation.turn;
        let call_id = invocation.call_id.clone();
        async move {
            let Ok(session) = upgrade_dispatch_session_capability(&self.session) else {
                return PreToolUseHookOutcome::Blocked(missing_dispatch_session_message());
            };
            session
                .run_pre_tool_use_hooks_for_tool(turn, call_id, payload)
                .await
        }
    }

    fn run_post_tool_use_hooks<'a>(
        &'a self,
        invocation: &'a ToolInvocation<Session, Turn, Tracker>,
        payload: PostToolUsePayload,
    ) -> impl std::future::Future<Output = PostToolUseHookOutcome> + Send + 'a {
        let turn = &invocation.turn;
        async move {
            let Ok(session) = upgrade_dispatch_session_capability(&self.session) else {
                return PostToolUseHookOutcome {
                    replacement_text: Some(missing_dispatch_session_message()),
                };
            };
            session
                .run_post_tool_use_hooks_for_tool(turn, payload)
                .await
        }
    }

    fn emit_tool_read_metric<'a>(
        &'a self,
        invocation: &'a ToolInvocation<Session, Turn, Tracker>,
        success: bool,
    ) -> impl std::future::Future<Output = ()> + Send + 'a {
        let turn = &invocation.turn;
        let tool_name = invocation.tool_name.clone();
        let payload = invocation.payload.clone();
        async move {
            let Ok(session) = upgrade_dispatch_session_capability(&self.session) else {
                warn!("{}", missing_dispatch_session_message());
                return;
            };
            session
                .emit_tool_read_metric(turn, &tool_name, &payload, success)
                .await;
        }
    }

    fn account_goal_tool_completed<'a>(
        &'a self,
        invocation: &'a ToolInvocation<Session, Turn, Tracker>,
        tool_name: &'a ToolName,
    ) -> impl std::future::Future<Output = Result<(), String>> + Send + 'a {
        let turn = &invocation.turn;
        let tool_name = tool_name.clone();
        async move {
            let session = upgrade_dispatch_session_capability(&self.session)?;
            session.account_goal_tool_completed(turn, &tool_name).await
        }
    }
}

struct NoopToolDispatchTrace;

impl ToolSessionDispatchTrace for NoopToolDispatchTrace {
    fn record_completed(&self, _call_id: &str, _payload: &ToolPayload, _result: &dyn ToolOutput) {}

    fn record_failed(&self, _error: &FunctionCallError) {}
}
