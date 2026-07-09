mod compact;
mod lifecycle;
mod regular;
mod review;
mod user_shell;

use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use codex_extension_api::ExtensionData;
use futures::future::BoxFuture;
use tokio::select;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;
use tracing::Instrument;
use tracing::Span;
use tracing::field;
use tracing::info_span;
use tracing::trace;
use tracing::warn;

use crate::PendingInputItem;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::state::ActiveTurn;
use crate::state::RunningTask;
use crate::state::TaskKind;
use codex_analytics_api::TurnTokenUsageFact;
use codex_auth_types::SharedAuthRuntime;
use config_service::Config;
use hooks::PendingInputHookDisposition;
use hooks::inspect_pending_input;
use hooks::record_additional_contexts;
use hooks::record_pending_input;
use metrics_api::TURN_E2E_DURATION_METRIC;
use metrics_api::TURN_MEMORY_METRIC;
use metrics_api::TURN_NETWORK_PROXY_METRIC;
use metrics_api::TURN_TOKEN_USAGE_METRIC;
use metrics_api::TURN_TOOL_CALL_METRIC;
use model_service_api::SharedModelProviderAuthManager;
use protocol::protocol::EventMsg;
use protocol::protocol::RolloutItem;
use protocol::protocol::TokenUsage;
use protocol::protocol::TurnAbortReason;
use protocol::protocol::TurnAbortedEvent;
use protocol::protocol::TurnCompleteEvent;
use protocol::protocol::WarningEvent;
use protocol::user_input::UserInput;
pub(crate) use rollout_api::InterruptedTurnHistoryMarker;
pub(crate) use rollout_api::interrupted_turn_history_marker;

use codex_features::Feature;
pub(crate) use compact::CompactTask;
#[cfg(test)]
pub(crate) use compact::continue_compact_turn_after_success;
pub(crate) use regular::RegularTask;
pub(crate) use review::ReviewTask;
pub(crate) use user_shell::UserShellCommandMode;
pub(crate) use user_shell::UserShellCommandTask;
pub(crate) use user_shell::execute_user_shell_command;

const GRACEFULL_INTERRUPTION_TIMEOUT_MS: u64 = 100;

pub(crate) fn interrupted_turn_history_marker_from_config(
    config: &Config,
) -> InterruptedTurnHistoryMarker {
    if !config.agent_interrupt_message_enabled {
        return InterruptedTurnHistoryMarker::Disabled;
    }
    InterruptedTurnHistoryMarker::Developer
}

fn emit_turn_network_proxy_metric(
    metrics: &dyn session_telemetry_api::SessionTelemetry,
    network_proxy_active: bool,
    tmp_mem: (&str, &str),
) {
    let active = if network_proxy_active {
        "true"
    } else {
        "false"
    };
    metrics.counter(
        TURN_NETWORK_PROXY_METRIC,
        /*inc*/ 1,
        &[("active", active), tmp_mem],
    );
}

fn emit_turn_memory_metric(
    metrics: &dyn session_telemetry_api::SessionTelemetry,
    feature_enabled: bool,
    config_enabled: bool,
    has_citations: bool,
) {
    let read_allowed = feature_enabled && config_enabled;
    metrics.counter(
        TURN_MEMORY_METRIC,
        /*inc*/ 1,
        &[
            ("read_allowed", bool_tag(read_allowed)),
            ("feature_enabled", bool_tag(feature_enabled)),
            ("config_use_memories", bool_tag(config_enabled)),
            ("has_citations", bool_tag(has_citations)),
        ],
    );
}

fn bool_tag(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

enum FollowUpTurnStart {
    PendingWork,
    LeftoverPendingInput,
}

fn spawn_follow_up_turn_start(session: Arc<Session>, start: FollowUpTurnStart) {
    let fut: BoxFuture<'static, ()> = Box::pin(async move {
        match start {
            FollowUpTurnStart::PendingWork => {
                session.maybe_start_turn_for_pending_work().await;
            }
            FollowUpTurnStart::LeftoverPendingInput => {
                session
                    .start_regular_turn_if_idle_with_sub_id(uuid::Uuid::new_v4().to_string())
                    .await;
            }
        }
    });
    tokio::spawn(fut);
}

/// Thin wrapper that exposes the parts of [`Session`] task runners need.
#[derive(Clone)]
pub(crate) struct SessionTaskContext {
    session: Arc<Session>,
    turn_extension_data: Arc<ExtensionData>,
}

impl SessionTaskContext {
    pub(crate) fn new(session: Arc<Session>, turn_extension_data: Arc<ExtensionData>) -> Self {
        Self {
            session,
            turn_extension_data,
        }
    }

    pub(crate) fn clone_session(&self) -> Arc<Session> {
        Arc::clone(&self.session)
    }

    pub(crate) fn turn_extension_data(&self) -> Arc<ExtensionData> {
        Arc::clone(&self.turn_extension_data)
    }

    pub(crate) fn auth_runtime(&self) -> SharedAuthRuntime {
        Arc::clone(&self.session.services.auth_runtime)
    }

    pub(crate) fn provider_auth_manager(&self) -> Option<SharedModelProviderAuthManager> {
        self.session.services.provider_auth_manager.clone()
    }
}

/// Async task that drives a [`Session`] turn.
///
/// Implementations encapsulate a specific Codex workflow (regular chat,
/// reviews, ghost snapshots, etc.). Each task instance is owned by a
/// [`Session`] and executed on a background Tokio task. The trait is
/// intentionally small: implementers identify themselves via
/// [`SessionTask::kind`], perform their work in [`SessionTask::run`], and may
/// release resources in [`SessionTask::abort`].
pub(crate) trait SessionTask: Send + Sync + 'static {
    /// Describes the type of work the task performs so the session can
    /// surface it in telemetry and UI.
    fn kind(&self) -> TaskKind;

    /// Returns the tracing name for a spawned task span.
    fn span_name(&self) -> &'static str;

    /// Returns whether turn token usage should be recorded on this task's turn span.
    fn records_turn_token_usage_on_span(&self) -> bool {
        false
    }

    /// Executes the task until completion or cancellation.
    ///
    /// Implementations typically stream protocol events using `session` and
    /// `ctx`, returning an optional final agent message when finished. The
    /// provided `cancellation_token` is cancelled when the session requests an
    /// abort; implementers should watch for it and terminate quickly once it
    /// fires. Returning [`Some`] yields a final message that
    /// [`Session::on_task_finished`] will emit to the client.
    fn run(
        self: Arc<Self>,
        session: Arc<SessionTaskContext>,
        ctx: Arc<TurnContext>,
        input: Vec<UserInput>,
        cancellation_token: CancellationToken,
    ) -> impl std::future::Future<Output = Option<String>> + Send;

    /// Gives the task a chance to perform cleanup after an abort.
    ///
    /// The default implementation is a no-op; override this if additional
    /// teardown or notifications are required once
    /// [`Session::abort_all_tasks`] cancels the task.
    fn abort(
        &self,
        session: Arc<SessionTaskContext>,
        ctx: Arc<TurnContext>,
    ) -> impl std::future::Future<Output = ()> + Send {
        async move {
            let _ = (session, ctx);
        }
    }
}

pub(crate) trait AnySessionTask: Send + Sync + 'static {
    fn kind(&self) -> TaskKind;

    fn span_name(&self) -> &'static str;

    fn records_turn_token_usage_on_span(&self) -> bool;

    fn run(
        self: Arc<Self>,
        session: Arc<SessionTaskContext>,
        ctx: Arc<TurnContext>,
        input: Vec<UserInput>,
        cancellation_token: CancellationToken,
    ) -> BoxFuture<'static, Option<String>>;

    fn abort<'a>(
        &'a self,
        session: Arc<SessionTaskContext>,
        ctx: Arc<TurnContext>,
    ) -> BoxFuture<'a, ()>;
}

impl<T> AnySessionTask for T
where
    T: SessionTask,
{
    fn kind(&self) -> TaskKind {
        SessionTask::kind(self)
    }

    fn span_name(&self) -> &'static str {
        SessionTask::span_name(self)
    }

    fn records_turn_token_usage_on_span(&self) -> bool {
        SessionTask::records_turn_token_usage_on_span(self)
    }

    fn run(
        self: Arc<Self>,
        session: Arc<SessionTaskContext>,
        ctx: Arc<TurnContext>,
        input: Vec<UserInput>,
        cancellation_token: CancellationToken,
    ) -> BoxFuture<'static, Option<String>> {
        Box::pin(SessionTask::run(
            self,
            session,
            ctx,
            input,
            cancellation_token,
        ))
    }

    fn abort<'a>(
        &'a self,
        session: Arc<SessionTaskContext>,
        ctx: Arc<TurnContext>,
    ) -> BoxFuture<'a, ()> {
        Box::pin(SessionTask::abort(self, session, ctx))
    }
}

impl Session {
    pub(crate) async fn spawn_task<T: SessionTask>(
        self: &Arc<Self>,
        turn_context: Arc<TurnContext>,
        input: Vec<UserInput>,
        task: T,
    ) {
        self.abort_all_tasks(TurnAbortReason::Replaced).await;
        self.clear_connector_selection().await;
        self.start_task(turn_context, input, task).await;
    }

    pub(crate) async fn start_task<T: SessionTask>(
        self: &Arc<Self>,
        turn_context: Arc<TurnContext>,
        input: Vec<UserInput>,
        task: T,
    ) {
        let task: Arc<dyn AnySessionTask> = Arc::new(task);
        let task_kind = task.kind();
        let span_name = task.span_name();
        let started_at = Instant::now();
        let turn_started_at_unix_ms = turn_context
            .turn_timing_state
            .mark_turn_started(started_at)
            .await;
        turn_context
            .turn_metadata_state
            .set_turn_started_at_unix_ms(turn_started_at_unix_ms);
        let token_usage_at_turn_start = self.total_token_usage().await.unwrap_or_default();

        let cancellation_token = CancellationToken::new();
        let done = Arc::new(Notify::new());

        self.services
            .guardian_rejection_circuit_breaker
            .lock()
            .await
            .clear_turn(&turn_context.sub_id);

        if let Err(err) = self
            .services
            .goal_service
            .begin_turn_goal_accounting(
                self.as_ref(),
                turn_context.as_ref(),
                token_usage_at_turn_start.clone(),
            )
            .await
        {
            warn!("failed to begin turn goal accounting: {err}");
        }
        let queued_response_items = self.take_queued_response_items_for_next_turn().await;
        let mailbox_items = self.get_pending_input().await;
        self.mark_direct_child_completions_received_from_pending_input(
            queued_response_items.iter().chain(mailbox_items.iter()),
        )
        .await;
        let turn_state = {
            let mut active = self.active_turn.lock().await;
            let turn = active.get_or_insert_with(ActiveTurn::default);
            debug_assert!(turn.tasks.is_empty());
            Arc::clone(&turn.turn_state)
        };
        {
            let mut turn_state = turn_state.lock().await;
            turn_state.token_usage_at_turn_start = token_usage_at_turn_start;
            for item in queued_response_items {
                turn_state.push_pending_input(item);
            }
            for item in mailbox_items {
                turn_state.push_pending_input(item);
            }
        }
        self.emit_turn_start_lifecycle(turn_context.extension_data.as_ref());

        let turn_extension_data = Arc::clone(&turn_context.extension_data);
        let mut active = self.active_turn.lock().await;
        let turn = active.get_or_insert_with(ActiveTurn::default);
        debug_assert!(turn.tasks.is_empty());
        let done_clone = Arc::clone(&done);
        let session_ctx = Arc::new(SessionTaskContext::new(
            Arc::clone(self),
            Arc::clone(&turn_extension_data),
        ));
        let ctx = Arc::clone(&turn_context);
        let task_for_run = Arc::clone(&task);
        let task_cancellation_token = cancellation_token.child_token();
        // Task-owned turn spans keep a core-owned span open for the
        // full task lifecycle after the submission dispatch span ends.
        let reasoning_effort = turn_context.effective_reasoning_effort_for_tracing();
        let task_span = info_span!(
            "turn",
            otel.name = span_name,
            thread.id = %self.conversation_id,
            turn.id = %turn_context.sub_id,
            model = %turn_context.model_info.slug,
            codex.turn.reasoning_effort = %reasoning_effort,
            codex.turn.token_usage.input_tokens = field::Empty,
            codex.turn.token_usage.cached_input_tokens = field::Empty,
            codex.turn.token_usage.non_cached_input_tokens = field::Empty,
            codex.turn.token_usage.output_tokens = field::Empty,
            codex.turn.token_usage.reasoning_output_tokens = field::Empty,
            codex.turn.token_usage.total_tokens = field::Empty,
        );
        let handle = tokio::spawn(
            async move {
                let ctx_for_finish = Arc::clone(&ctx);
                let last_agent_message = task_for_run
                    .run(
                        Arc::clone(&session_ctx),
                        ctx,
                        input,
                        task_cancellation_token.child_token(),
                    )
                    .await;
                let sess = session_ctx.clone_session();
                if let Err(err) = sess.flush_rollout().await {
                    warn!("failed to flush rollout before completing turn: {err}");
                    sess.send_event(
                        ctx_for_finish.as_ref(),
                        EventMsg::Warning(WarningEvent {
                            message: format!(
                                "Failed to save the conversation transcript; Codex will continue retrying. Error: {err}"
                            ),
                        }),
                    )
                    .await;
                }
                if !task_cancellation_token.is_cancelled() {
                    // Emit completion uniformly from spawn site so all tasks share the same lifecycle.
                    sess.on_task_finished(Arc::clone(&ctx_for_finish), last_agent_message)
                        .await;
                }
                done_clone.notify_waiters();
            }
            .instrument(task_span),
        );
        let timer = turn_context
            .session_telemetry
            .start_timer(TURN_E2E_DURATION_METRIC, &[]);
        let running_task = RunningTask {
            done,
            handle: AbortOnDropHandle::new(handle),
            kind: task_kind,
            records_turn_token_usage_on_span: task.records_turn_token_usage_on_span(),
            task,
            cancellation_token,
            turn_context: Arc::clone(&turn_context),
            turn_extension_data,
            _timer: timer,
        };
        turn.add_task(running_task);
    }

    /// Starts a regular turn when the session is idle and pending work is waiting.
    ///
    /// Pending work currently includes queued next-turn items and mailbox mail marked with
    /// `trigger_turn`.
    ///
    /// This helper generates a fresh sub-id for the synthetic turn before delegating to the
    /// explicit-sub-id variant.
    pub(crate) async fn maybe_start_turn_for_pending_work(self: &Arc<Self>) {
        self.maybe_start_turn_for_pending_work_with_sub_id(uuid::Uuid::new_v4().to_string())
            .await;
    }

    fn start_regular_turn_if_idle_with_sub_id(self: &Arc<Self>, sub_id: String) -> BoxFuture<'static, bool> {
        let session = Arc::clone(self);
        Box::pin(async move {
            {
                let mut active_turn = session.active_turn.lock().await;
                if active_turn.is_some() {
                    return false;
                }
                *active_turn = Some(ActiveTurn::default());
            }

            let turn_context = session.new_default_turn_with_sub_id(sub_id).await;
            session
                .maybe_emit_unknown_model_warning_for_turn(turn_context.as_ref())
                .await;
            session
                .start_task(turn_context, Vec::new(), RegularTask::new())
                .await;
            true
        })
    }

    /// Starts a regular turn with the provided sub-id when pending work should wake an idle
    /// session.
    ///
    /// The turn is created only when there are queued next-turn items or mailbox mail marked with
    /// `trigger_turn`, and only if the session is currently idle.
    pub(crate) async fn maybe_start_turn_for_pending_work_with_sub_id(
        self: &Arc<Self>,
        sub_id: String,
    ) {
        {
            let mut active_turn = self.active_turn.lock().await;
            if active_turn.is_some() {
                return;
            }
            if !self.has_thread_pending_work().await {
                return;
            }
            *active_turn = Some(ActiveTurn::default());
        }

        let turn_context = self.new_default_turn_with_sub_id(sub_id).await;
        self.maybe_emit_unknown_model_warning_for_turn(turn_context.as_ref())
            .await;
        self.start_task(turn_context, Vec::new(), RegularTask::new())
            .await;
    }

    pub async fn abort_all_tasks(self: &Arc<Self>, reason: TurnAbortReason) {
        let mut aborted_turn = false;
        let mut active_turn_to_clear = None;
        let mut turn_context = None;
        if let Some(mut active_turn) = self.take_active_turn().await {
            let tasks = active_turn.drain_tasks();
            aborted_turn = !tasks.is_empty();
            turn_context = tasks.first().map(|task| Arc::clone(&task.turn_context));
            for task in tasks {
                self.handle_task_abort(task, reason.clone()).await;
            }
            if aborted_turn {
                active_turn_to_clear = Some(active_turn);
            }
        }

        if let Some(turn_context) = turn_context.as_deref() {
            self.emit_turn_abort_lifecycle(reason.clone(), turn_context.extension_data.as_ref());
        }
        if (aborted_turn || reason == TurnAbortReason::Interrupted)
            && let Err(err) = self
                .services
                .goal_service
                .handle_goal_turn_abort(
                    self.as_ref(),
                    turn_context
                        .as_deref()
                        .map(|turn| turn as &dyn thread_service_api::ThreadTurnCapability),
                    reason.clone(),
                )
                .await
        {
            warn!("failed to handle goal turn abort: {err}");
        }
        if let Some(active_turn) = active_turn_to_clear {
            // Let interrupted tasks observe cancellation before dropping pending approvals, or an
            // in-flight approval wait can surface as a model-visible rejection before TurnAborted.
            let pending_input = active_turn.clear_pending().await;
            self.mark_direct_child_completions_received_from_pending_input(pending_input.iter())
                .await;
        }
        if reason == TurnAbortReason::Interrupted && aborted_turn {
            self.maybe_start_turn_for_pending_work().await;
        }
    }

    pub(crate) async fn abort_turn_if_active(
        self: &Arc<Self>,
        turn_id: &str,
        reason: TurnAbortReason,
    ) -> bool {
        let active_turn = {
            let mut active = self.active_turn.lock().await;
            if active
                .as_ref()
                .is_some_and(|active_turn| active_turn.tasks.contains_key(turn_id))
            {
                active.take()
            } else {
                None
            }
        };
        let Some(mut active_turn) = active_turn else {
            return false;
        };

        let tasks = active_turn.drain_tasks();
        let turn_context = tasks.first().map(|task| Arc::clone(&task.turn_context));
        for task in tasks {
            self.handle_task_abort(task, reason.clone()).await;
        }
        if let Some(turn_context) = turn_context.as_deref() {
            self.emit_turn_abort_lifecycle(reason.clone(), turn_context.extension_data.as_ref());
        }
        if let Err(err) = self
            .services
            .goal_service
            .handle_goal_turn_abort(
                self.as_ref(),
                turn_context
                    .as_deref()
                    .map(|turn| turn as &dyn thread_service_api::ThreadTurnCapability),
                reason.clone(),
            )
            .await
        {
            warn!("failed to handle goal turn abort: {err}");
        }
        // Let interrupted tasks observe cancellation before dropping pending approvals, or an
        // in-flight approval wait can surface as a model-visible rejection before TurnAborted.
        let pending_input = active_turn.clear_pending().await;
        self.mark_direct_child_completions_received_from_pending_input(pending_input.iter())
            .await;

        if reason == TurnAbortReason::Interrupted {
            self.maybe_start_turn_for_pending_work().await;
        }

        true
    }

    pub async fn on_task_finished(
        self: &Arc<Self>,
        turn_context: Arc<TurnContext>,
        last_agent_message: Option<String>,
    ) {
        turn_context
            .turn_metadata_state
            .cancel_git_enrichment_task();

        let mut should_clear_active_turn = false;
        let mut token_usage_at_turn_start = None;
        let mut turn_had_memory_citation = false;
        let mut turn_tool_calls = 0_u64;
        let mut records_turn_token_usage_on_span = false;
        let turn_state = {
            let mut active = self.active_turn.lock().await;
            if let Some(at) = active.as_mut()
                && let Some(removed_task) = at.remove_task(&turn_context.sub_id)
            {
                records_turn_token_usage_on_span = removed_task.records_turn_token_usage_on_span;
                if removed_task.active_turn_is_empty {
                    should_clear_active_turn = true;
                    let turn_state = Arc::clone(&at.turn_state);
                    Some(turn_state)
                } else {
                    None
                }
            } else {
                None
            }
        };
        if let Some(turn_state) = turn_state.as_ref() {
            let ts = turn_state.lock().await;
            turn_had_memory_citation = ts.has_memory_citation;
            turn_tool_calls = ts.tool_calls;
            token_usage_at_turn_start = Some(ts.token_usage_at_turn_start.clone());
        }
        // Emit token usage metrics.
        if let Some(token_usage_at_turn_start) = token_usage_at_turn_start {
            // TODO(jif): drop this
            let tmp_mem = (
                "tmp_mem_enabled",
                if self.enabled(Feature::MemoryTool) {
                    "true"
                } else {
                    "false"
                },
            );
            let network_proxy_active = match self.services.network_proxy.as_ref() {
                Some(started_network_proxy) => {
                    match started_network_proxy.proxy().current_config().await {
                        Ok(config) => config.network.enabled,
                        Err(err) => {
                            warn!(
                                "failed to read managed network proxy state for turn metrics: {err:#}"
                            );
                            false
                        }
                    }
                }
                None => false,
            };
            emit_turn_network_proxy_metric(
                self.services.session_telemetry.as_ref(),
                network_proxy_active,
                tmp_mem,
            );
            self.services.session_telemetry.histogram(
                TURN_TOOL_CALL_METRIC,
                i64::try_from(turn_tool_calls).unwrap_or(i64::MAX),
                &[tmp_mem],
            );
            let total_token_usage = self.total_token_usage().await.unwrap_or_default();
            let turn_token_usage = TokenUsage {
                input_tokens: (total_token_usage.input_tokens
                    - token_usage_at_turn_start.input_tokens)
                    .max(0),
                cached_input_tokens: (total_token_usage.cached_input_tokens
                    - token_usage_at_turn_start.cached_input_tokens)
                    .max(0),
                output_tokens: (total_token_usage.output_tokens
                    - token_usage_at_turn_start.output_tokens)
                    .max(0),
                reasoning_output_tokens: (total_token_usage.reasoning_output_tokens
                    - token_usage_at_turn_start.reasoning_output_tokens)
                    .max(0),
                total_tokens: (total_token_usage.total_tokens
                    - token_usage_at_turn_start.total_tokens)
                    .max(0),
            };
            if records_turn_token_usage_on_span {
                let current_span = Span::current();
                current_span.record(
                    "codex.turn.token_usage.input_tokens",
                    turn_token_usage.input_tokens,
                );
                current_span.record(
                    "codex.turn.token_usage.cached_input_tokens",
                    turn_token_usage.cached_input(),
                );
                current_span.record(
                    "codex.turn.token_usage.non_cached_input_tokens",
                    turn_token_usage.non_cached_input(),
                );
                current_span.record(
                    "codex.turn.token_usage.output_tokens",
                    turn_token_usage.output_tokens,
                );
                current_span.record(
                    "codex.turn.token_usage.reasoning_output_tokens",
                    turn_token_usage.reasoning_output_tokens,
                );
                current_span.record(
                    "codex.turn.token_usage.total_tokens",
                    turn_token_usage.total_tokens,
                );
            }
            self.services
                .analytics_events_client
                .track_turn_token_usage(TurnTokenUsageFact {
                    turn_id: turn_context.sub_id.clone(),
                    thread_id: self.conversation_id.to_string(),
                    token_usage: turn_token_usage.clone(),
                });
            self.services.session_telemetry.histogram(
                TURN_TOKEN_USAGE_METRIC,
                turn_token_usage.total_tokens,
                &[("token_type", "total"), tmp_mem],
            );
            self.services.session_telemetry.histogram(
                TURN_TOKEN_USAGE_METRIC,
                turn_token_usage.input_tokens,
                &[("token_type", "input"), tmp_mem],
            );
            self.services.session_telemetry.histogram(
                TURN_TOKEN_USAGE_METRIC,
                turn_token_usage.cached_input(),
                &[("token_type", "cached_input"), tmp_mem],
            );
            self.services.session_telemetry.histogram(
                TURN_TOKEN_USAGE_METRIC,
                turn_token_usage.output_tokens,
                &[("token_type", "output"), tmp_mem],
            );
            self.services.session_telemetry.histogram(
                TURN_TOKEN_USAGE_METRIC,
                turn_token_usage.reasoning_output_tokens,
                &[("token_type", "reasoning_output"), tmp_mem],
            );
        }
        emit_turn_memory_metric(
            self.services.session_telemetry.as_ref(),
            turn_context.features.enabled(Feature::MemoryTool),
            turn_context.config.memories.use_memories,
            turn_had_memory_citation,
        );
        let (completed_at, duration_ms) = turn_context
            .turn_timing_state
            .completed_at_and_duration_ms()
            .await;
        let time_to_first_token_ms = turn_context
            .turn_timing_state
            .time_to_first_token_ms()
            .await;
        if should_clear_active_turn {
            self.emit_turn_stop_lifecycle(turn_context.extension_data.as_ref());
        }
        if let Err(err) = self
            .services
            .goal_service
            .finish_turn_goal_accounting(
                self.as_ref(),
                turn_context.as_ref(),
                should_clear_active_turn,
            )
            .await
        {
            warn!("failed to finish turn goal accounting: {err}");
        }
        let event = EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: turn_context.sub_id.clone(),
            last_agent_message,
            completed_at,
            duration_ms,
            time_to_first_token_ms,
        });
        self.send_event(turn_context.as_ref(), event).await;
        self.services
            .guardian_rejection_circuit_breaker
            .lock()
            .await
            .clear_turn(&turn_context.sub_id);

        if should_clear_active_turn {
            let (
                cleared_active_turn,
                pending_input,
                has_thread_pending_work,
            ) = {
                let mut active = self.active_turn.lock().await;
                if let Some(active_turn) = active.as_ref()
                    && active_turn.tasks.is_empty()
                    && turn_state
                        .as_ref()
                        .is_some_and(|turn_state| Arc::ptr_eq(&active_turn.turn_state, turn_state))
                {
                    let mut pending_input = Vec::<PendingInputItem>::new();
                    let mut has_thread_pending_work = false;
                    if let Some(turn_state) = turn_state.as_ref() {
                        let mut ts = turn_state.lock().await;
                        pending_input = ts.take_pending_input();
                        has_thread_pending_work = self.has_thread_pending_work().await;
                    }
                    *active = None;
                    (true, pending_input, has_thread_pending_work)
                } else {
                    (false, Vec::new(), false)
                }
            };
            if !cleared_active_turn {
                return;
            }
            let mut restart_for_leftover_pending_input = false;
            if !pending_input.is_empty() {
                for pending_input_item in pending_input {
                    match inspect_pending_input(
                        self.as_ref(),
                        turn_context.as_ref(),
                        pending_input_item,
                    )
                    .await
                    {
                        PendingInputHookDisposition::Accepted(pending_input) => {
                            restart_for_leftover_pending_input = true;
                            record_pending_input(
                                self.as_ref(),
                                turn_context.as_ref(),
                                *pending_input,
                            )
                            .await;
                        }
                        PendingInputHookDisposition::Blocked {
                            additional_contexts,
                        } => {
                            record_additional_contexts(
                                self.as_ref(),
                                turn_context.as_ref(),
                                additional_contexts,
                            )
                            .await;
                        }
                    }
                }
            }
            if has_thread_pending_work {
                spawn_follow_up_turn_start(Arc::clone(self), FollowUpTurnStart::PendingWork);
                return;
            }
            if restart_for_leftover_pending_input {
                spawn_follow_up_turn_start(
                    Arc::clone(self),
                    FollowUpTurnStart::LeftoverPendingInput,
                );
                return;
            }
            if let Err(err) = self
                .services
                .goal_service
                .maybe_continue_active_goal(self.as_ref())
                .await
            {
                warn!("failed to continue active goal while idle: {err}");
            }
            Box::pin(self.maybe_notify_parent_of_final_status(turn_context.as_ref())).await;
        }
    }

    async fn take_active_turn(&self) -> Option<ActiveTurn> {
        let mut active = self.active_turn.lock().await;
        active.take()
    }

    pub(crate) async fn close_unified_exec_processes(&self) {
        self.services
            .command_service_state
            .terminate_all_processes()
            .await;
    }

    async fn handle_task_abort(self: &Arc<Self>, task: RunningTask, reason: TurnAbortReason) {
        let sub_id = task.turn_context.sub_id.clone();
        if task.cancellation_token.is_cancelled() {
            return;
        }

        trace!(task_kind = ?task.kind, sub_id, "aborting running task");
        task.cancellation_token.cancel();
        task.turn_context
            .turn_metadata_state
            .cancel_git_enrichment_task();
        let session_task = task.task;

        select! {
            _ = task.done.notified() => {
            },
            _ = tokio::time::sleep(Duration::from_millis(GRACEFULL_INTERRUPTION_TIMEOUT_MS)) => {
                warn!("task {sub_id} didn't complete gracefully after {}ms", GRACEFULL_INTERRUPTION_TIMEOUT_MS);
            }
        }

        task.handle.abort();

        let session_ctx = Arc::new(SessionTaskContext::new(
            Arc::clone(self),
            Arc::clone(&task.turn_extension_data),
        ));
        session_task
            .abort(session_ctx, Arc::clone(&task.turn_context))
            .await;

        if reason == TurnAbortReason::Interrupted
            && let Some(marker) = interrupted_turn_history_marker(
                interrupted_turn_history_marker_from_config(task.turn_context.config.as_ref()),
            )
        {
            self.record_into_history(std::slice::from_ref(&marker), task.turn_context.as_ref())
                .await;
            self.persist_rollout_items(&[RolloutItem::ResponseItem(marker)])
                .await;
            // Ensure the marker is durably visible before emitting TurnAborted: some clients
            // synchronously re-read the rollout on receipt of the abort event.
            if let Err(err) = self.flush_rollout().await {
                warn!("failed to flush interrupted-turn marker before emitting TurnAborted: {err}");
            }
        }

        let (completed_at, duration_ms) = task
            .turn_context
            .turn_timing_state
            .completed_at_and_duration_ms()
            .await;
        let event = EventMsg::TurnAborted(TurnAbortedEvent {
            turn_id: Some(task.turn_context.sub_id.clone()),
            reason,
            completed_at,
            duration_ms,
        });
        self.send_event(task.turn_context.as_ref(), event).await;
        self.services
            .guardian_rejection_circuit_breaker
            .lock()
            .await
            .clear_turn(&task.turn_context.sub_id);
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
