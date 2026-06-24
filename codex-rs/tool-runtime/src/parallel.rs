use crate::AbortedToolOutput;
use crate::AnyToolResult;
use crate::ToolArgumentDiffConsumer;
use codex_protocol::error::CodexErr;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseItem;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolPayload;
use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tokio_util::either::Either;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;
use tracing::Instrument;
use tracing::instrument;
use tracing::trace_span;

pub use codex_tool_planning::ToolCall;
pub use codex_tool_planning::ToolCallSource;
pub use codex_tool_planning::ToolName;

/// Host-side bridge used by [`ToolCallRuntime`] to dispatch parsed tool calls.
///
/// Implementations own the concrete handler registry and any session-specific
/// hook/telemetry accounting. The generic runtime only coordinates
/// cancellation, per-tool parallelism, and conversion of dispatch errors into
/// model-visible responses.
pub trait ToolCallRuntimeRouter<Session, Turn, Tracker, DiffContext>:
    Send + Sync + 'static
{
    fn create_diff_consumer(
        &self,
        tool_name: &ToolName,
    ) -> Option<Box<dyn ToolArgumentDiffConsumer<DiffContext>>>;

    fn tool_supports_parallel(&self, call: &ToolCall) -> bool;

    fn dispatch_tool_call_with_code_mode_result(
        &self,
        session: Session,
        turn: Turn,
        cancellation_token: CancellationToken,
        tracker: Tracker,
        call: ToolCall,
        source: ToolCallSource,
    ) -> impl Future<Output = Result<AnyToolResult, FunctionCallError>> + Send;
}

pub struct ToolCallRuntime<Router, Session, Turn, Tracker, DiffContext> {
    router: Arc<Router>,
    session: Session,
    turn_context: Turn,
    tracker: Tracker,
    parallel_execution: Arc<RwLock<()>>,
    _marker: PhantomData<fn(DiffContext)>,
}

impl<Router, Session, Turn, Tracker, DiffContext> Clone
    for ToolCallRuntime<Router, Session, Turn, Tracker, DiffContext>
where
    Session: Clone,
    Turn: Clone,
    Tracker: Clone,
{
    fn clone(&self) -> Self {
        Self {
            router: Arc::clone(&self.router),
            session: self.session.clone(),
            turn_context: self.turn_context.clone(),
            tracker: self.tracker.clone(),
            parallel_execution: Arc::clone(&self.parallel_execution),
            _marker: PhantomData,
        }
    }
}

impl<Router, Session, Turn, Tracker, DiffContext>
    ToolCallRuntime<Router, Session, Turn, Tracker, DiffContext>
where
    Router: ToolCallRuntimeRouter<Session, Turn, Tracker, DiffContext>,
    Session: Clone + Send + Sync + 'static,
    Turn: Clone + Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
    DiffContext: 'static,
{
    pub fn new(
        router: Arc<Router>,
        session: Session,
        turn_context: Turn,
        tracker: Tracker,
    ) -> Self {
        Self {
            router,
            session,
            turn_context,
            tracker,
            parallel_execution: Arc::new(RwLock::new(())),
            _marker: PhantomData,
        }
    }

    pub fn create_diff_consumer(
        &self,
        tool_name: &ToolName,
    ) -> Option<Box<dyn ToolArgumentDiffConsumer<DiffContext>>> {
        self.router.create_diff_consumer(tool_name)
    }

    #[instrument(level = "trace", skip_all)]
    pub fn handle_tool_call(
        self,
        call: ToolCall,
        cancellation_token: CancellationToken,
    ) -> impl Future<Output = Result<ResponseItem, CodexErr>> {
        let error_call = call.clone();
        let future =
            self.handle_tool_call_with_source(call, ToolCallSource::Direct, cancellation_token);
        async move {
            match future.await {
                Ok(response) => Ok(response.into_response().into()),
                Err(FunctionCallError::Fatal(message)) => Err(CodexErr::Fatal(message)),
                Err(other) => Ok(Self::failure_response(error_call, other)),
            }
        }
        .in_current_span()
    }

    #[instrument(level = "trace", skip_all)]
    pub fn handle_tool_call_with_source(
        self,
        call: ToolCall,
        source: ToolCallSource,
        cancellation_token: CancellationToken,
    ) -> impl Future<Output = Result<AnyToolResult, FunctionCallError>> {
        let supports_parallel = self.router.tool_supports_parallel(&call);
        let router = Arc::clone(&self.router);
        let session = self.session.clone();
        let turn = self.turn_context.clone();
        let tracker = self.tracker.clone();
        let lock = Arc::clone(&self.parallel_execution);
        let invocation_cancellation_token = cancellation_token.clone();
        let started = Instant::now();

        let dispatch_span = trace_span!(
            "dispatch_tool_call_with_code_mode_result",
            otel.name = %call.tool_name,
            tool_name = %call.tool_name,
            call_id = call.call_id.as_str(),
            aborted = false,
        );

        let handle: AbortOnDropHandle<Result<AnyToolResult, FunctionCallError>> =
            AbortOnDropHandle::new(tokio::spawn(async move {
                tokio::select! {
                    _ = cancellation_token.cancelled() => {
                        let secs = started.elapsed().as_secs_f32().max(0.1);
                        dispatch_span.record("aborted", true);
                        Ok(Self::aborted_response(&call, secs))
                    },
                    res = async {
                        let _guard = if supports_parallel {
                            Either::Left(lock.read().await)
                        } else {
                            Either::Right(lock.write().await)
                        };

                        router
                            .dispatch_tool_call_with_code_mode_result(
                                session,
                                turn,
                                invocation_cancellation_token,
                                tracker,
                                call.clone(),
                                source,
                            )
                            .instrument(dispatch_span.clone())
                            .await
                    } => res,
                }
            }));

        async move {
            handle.await.map_err(|err| {
                FunctionCallError::Fatal(format!("tool task failed to receive: {err:?}"))
            })?
        }
        .in_current_span()
    }

    fn failure_response(call: ToolCall, err: FunctionCallError) -> ResponseItem {
        let message = err.to_string();
        match call.payload {
            ToolPayload::ToolSearch { .. } => ResponseItem::ToolSearchOutput {
                call_id: Some(call.call_id),
                status: "completed".to_string(),
                execution: "client".to_string(),
                tools: Vec::new(),
            },
            ToolPayload::Custom { .. } => ResponseItem::CustomToolCallOutput {
                call_id: call.call_id,
                name: None,
                output: FunctionCallOutputPayload {
                    body: FunctionCallOutputBody::Text(message),
                    success: Some(false),
                },
            },
            _ => ResponseItem::FunctionCallOutput {
                call_id: call.call_id,
                output: FunctionCallOutputPayload {
                    body: FunctionCallOutputBody::Text(message),
                    success: Some(false),
                },
            },
        }
    }

    fn aborted_response(call: &ToolCall, secs: f32) -> AnyToolResult {
        AnyToolResult {
            call_id: call.call_id.clone(),
            payload: call.payload.clone(),
            result: Box::new(AbortedToolOutput {
                message: Self::abort_message(call, secs),
            }),
            post_tool_use_payload: None,
        }
    }

    fn abort_message(call: &ToolCall, secs: f32) -> String {
        if call.tool_name.namespace.is_none()
            && matches!(
                call.tool_name.name.as_str(),
                "shell_command" | "unified_exec"
            )
        {
            format!("Wall time: {secs:.1} seconds\naborted by user")
        } else {
            format!("aborted by user after {secs:.1}s")
        }
    }
}
