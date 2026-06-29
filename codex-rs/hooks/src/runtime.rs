use std::future::Future;
use std::path::PathBuf;

use codex_context_manager::ContextualUserFragment;
use codex_context_manager::HookAdditionalContext;
use codex_hooks_api::PermissionRequestDecision;
use codex_hooks_api::PermissionRequestOutcome;
use codex_hooks_api::PermissionRequestRequest;
use codex_hooks_api::PostToolUseOutcome;
use codex_hooks_api::PostToolUseRequest;
use codex_hooks_api::PreToolUseOutcome;
use codex_hooks_api::PreToolUseRequest;
use codex_hooks_api::SessionStartOutcome;
use codex_hooks_api::SessionStartSource;
use codex_hooks_api::SharedHookRuntime;
use codex_hooks_api::UserPromptSubmitOutcome;
use codex_hooks_api::UserPromptSubmitRequest;
use codex_protocol::items::TurnItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::HookCompletedEvent;
use codex_protocol::protocol::HookRunSummary;
use codex_protocol::user_input::UserInput;
use thread_service_api::PendingInputItem;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde_json::Value;

pub struct HookRuntimeContext {
    pub session_id: codex_protocol::ThreadId,
    pub turn_id: String,
    pub cwd: AbsolutePathBuf,
    pub transcript_path: Option<PathBuf>,
    pub model: String,
    pub permission_mode: String,
}

/// Turn-scoped data that the hook runtime needs from its embedding session
/// implementation.
///
/// Implementations should expose only stable turn metadata. They must not run
/// hooks, mutate history, or emit events; those side effects belong to
/// [`HookRuntimeHost`].
pub trait HookRuntimeTurn: Send + Sync {
    fn turn_id(&self) -> &str;
    fn cwd(&self) -> &AbsolutePathBuf;
    fn model_slug(&self) -> &str;
    fn approval_policy(&self) -> AskForApproval;
}

/// Host interface used by `codex-hooks` to run hook flows without depending on
/// a concrete session runtime.
///
/// Implementations own the runtime side effects for their domain: event
/// emission, analytics/metrics, and model-visible history writes. The hook
/// crate owns the hook ordering and outcome handling, while the host remains
/// responsible for persisting or displaying the resulting items.
pub trait HookRuntimeHost: Send + Sync {
    type Turn: HookRuntimeTurn;

    fn session_id(&self) -> codex_protocol::ThreadId;
    fn hooks(&self) -> SharedHookRuntime;
    fn take_pending_session_start_source(
        &self,
    ) -> impl Future<Output = Option<SessionStartSource>> + Send;
    fn hook_transcript_path(&self) -> impl Future<Output = Option<PathBuf>> + Send;
    fn emit_hook_started(
        &self,
        turn: &Self::Turn,
        run: HookRunSummary,
    ) -> impl Future<Output = ()> + Send;
    fn emit_hook_completed(
        &self,
        turn: &Self::Turn,
        completed: HookCompletedEvent,
    ) -> impl Future<Output = ()> + Send;
    fn record_conversation_items(
        &self,
        turn: &Self::Turn,
        items: Vec<ResponseItem>,
    ) -> impl Future<Output = ()> + Send;
    fn record_model_items_and_emit_display_events(
        &self,
        turn: &Self::Turn,
        items: Vec<ResponseItem>,
    ) -> impl Future<Output = ()> + Send;
    fn record_user_prompt_and_emit_turn_item(
        &self,
        turn: &Self::Turn,
        content: Vec<UserInput>,
        response_item: ResponseItem,
    ) -> impl Future<Output = ()> + Send;
}

pub struct HookRuntimeOutcome {
    pub should_stop: bool,
    pub additional_contexts: Vec<String>,
}

pub enum PreToolUseHookResult {
    Continue { updated_input: Option<Value> },
    Blocked(String),
}

pub enum PendingInputHookDisposition {
    Accepted(Box<PendingInputRecord>),
    Blocked { additional_contexts: Vec<String> },
}

pub enum PendingInputRecord {
    UserMessage {
        content: Vec<UserInput>,
        response_item: ResponseItem,
        additional_contexts: Vec<String>,
    },
    ConversationItem {
        response_item: ResponseItem,
    },
    InterAgentCommunication {
        pending_input: PendingInputItem,
    },
}

pub struct PermissionRequestHookPayload {
    pub tool_name: String,
    pub matcher_aliases: Vec<String>,
    pub tool_input: Value,
}

struct ContextInjectingHookOutcome {
    hook_events: Vec<HookCompletedEvent>,
    outcome: HookRuntimeOutcome,
}

impl From<SessionStartOutcome> for ContextInjectingHookOutcome {
    fn from(value: SessionStartOutcome) -> Self {
        let SessionStartOutcome {
            hook_events,
            should_stop,
            stop_reason: _,
            additional_contexts,
        } = value;
        Self {
            hook_events,
            outcome: HookRuntimeOutcome {
                should_stop,
                additional_contexts,
            },
        }
    }
}

impl From<UserPromptSubmitOutcome> for ContextInjectingHookOutcome {
    fn from(value: UserPromptSubmitOutcome) -> Self {
        let UserPromptSubmitOutcome {
            hook_events,
            should_stop,
            stop_reason: _,
            additional_contexts,
        } = value;
        Self {
            hook_events,
            outcome: HookRuntimeOutcome {
                should_stop,
                additional_contexts,
            },
        }
    }
}

pub async fn run_pending_session_start_hooks<H>(host: &H, turn: &H::Turn) -> bool
where
    H: HookRuntimeHost,
{
    let Some(session_start_source) = host.take_pending_session_start_source().await else {
        return false;
    };

    let context = hook_runtime_context(host, turn).await;
    let request = codex_hooks_api::SessionStartRequest {
        session_id: context.session_id,
        cwd: context.cwd,
        transcript_path: context.transcript_path,
        model: context.model,
        permission_mode: context.permission_mode,
        source: session_start_source,
    };
    let hooks = host.hooks();
    let preview_runs = hooks.preview_session_start(&request);
    run_context_injecting_hook(
        host,
        turn,
        preview_runs,
        hooks.run_session_start(request, Some(context.turn_id)),
    )
    .await
    .record_additional_contexts(host, turn)
    .await
}

pub async fn run_pre_tool_use_hooks<H>(
    host: &H,
    turn: &H::Turn,
    tool_use_id: String,
    tool_name: &str,
    matcher_aliases: Vec<String>,
    tool_input: &Value,
) -> PreToolUseHookResult
where
    H: HookRuntimeHost,
{
    let context = hook_runtime_context(host, turn).await;
    let request = PreToolUseRequest {
        session_id: context.session_id,
        turn_id: context.turn_id,
        cwd: context.cwd,
        transcript_path: context.transcript_path,
        model: context.model,
        permission_mode: context.permission_mode,
        tool_name: tool_name.to_string(),
        matcher_aliases,
        tool_use_id,
        tool_input: tool_input.clone(),
    };
    let hooks = host.hooks();
    let preview_runs = hooks.preview_pre_tool_use(&request);
    emit_hook_started_events(host, turn, preview_runs).await;

    let PreToolUseOutcome {
        hook_events,
        should_block,
        block_reason,
        additional_contexts,
        updated_input,
    } = hooks.run_pre_tool_use(request).await;
    emit_hook_completed_events(host, turn, hook_events).await;
    record_additional_contexts(host, turn, additional_contexts).await;

    if !should_block {
        return PreToolUseHookResult::Continue { updated_input };
    }

    let Some(reason) = block_reason else {
        return PreToolUseHookResult::Continue {
            updated_input: None,
        };
    };

    if (tool_name == "Bash" || tool_name == "apply_patch")
        && let Some(command) = tool_input.get("command").and_then(Value::as_str)
    {
        PreToolUseHookResult::Blocked(format!(
            "Command blocked by PreToolUse hook: {reason}. Command: {command}"
        ))
    } else {
        PreToolUseHookResult::Blocked(format!(
            "Tool call blocked by PreToolUse hook: {reason}. Tool: {tool_name}"
        ))
    }
}

pub async fn run_permission_request_hooks<H>(
    host: &H,
    turn: &H::Turn,
    run_id_suffix: &str,
    payload: PermissionRequestHookPayload,
) -> Option<PermissionRequestDecision>
where
    H: HookRuntimeHost,
{
    let context = hook_runtime_context(host, turn).await;
    let request = PermissionRequestRequest {
        session_id: context.session_id,
        turn_id: context.turn_id,
        cwd: context.cwd.to_path_buf(),
        transcript_path: context.transcript_path,
        model: context.model,
        permission_mode: context.permission_mode,
        tool_name: payload.tool_name,
        matcher_aliases: payload.matcher_aliases,
        run_id_suffix: run_id_suffix.to_string(),
        tool_input: payload.tool_input,
    };
    let hooks = host.hooks();
    let preview_runs = hooks.preview_permission_request(&request);
    emit_hook_started_events(host, turn, preview_runs).await;

    let PermissionRequestOutcome {
        hook_events,
        decision,
    } = hooks.run_permission_request(request).await;
    emit_hook_completed_events(host, turn, hook_events).await;

    decision
}

pub async fn run_post_tool_use_hooks<H>(
    host: &H,
    turn: &H::Turn,
    tool_use_id: String,
    tool_name: String,
    matcher_aliases: Vec<String>,
    tool_input: Value,
    tool_response: Value,
) -> PostToolUseOutcome
where
    H: HookRuntimeHost,
{
    let context = hook_runtime_context(host, turn).await;
    let request = PostToolUseRequest {
        session_id: context.session_id,
        turn_id: context.turn_id,
        cwd: context.cwd,
        transcript_path: context.transcript_path,
        model: context.model,
        permission_mode: context.permission_mode,
        tool_name,
        matcher_aliases,
        tool_use_id,
        tool_input,
        tool_response,
    };
    let hooks = host.hooks();
    let preview_runs = hooks.preview_post_tool_use(&request);
    emit_hook_started_events(host, turn, preview_runs).await;

    let outcome = hooks.run_post_tool_use(request).await;
    emit_hook_completed_events(host, turn, outcome.hook_events.clone()).await;
    outcome
}

pub async fn run_pre_compact_hooks<H>(
    host: &H,
    turn: &H::Turn,
    trigger: codex_analytics_api::CompactionTrigger,
) -> PreCompactHookOutcome
where
    H: HookRuntimeHost,
{
    let context = hook_runtime_context(host, turn).await;
    let request = codex_hooks_api::PreCompactRequest {
        session_id: context.session_id,
        turn_id: context.turn_id,
        cwd: context.cwd,
        transcript_path: context.transcript_path,
        model: context.model,
        trigger: compaction_trigger_label(trigger).to_string(),
    };
    let preview_runs = host.hooks().preview_pre_compact(&request);
    emit_hook_started_events(host, turn, preview_runs).await;

    let outcome = host.hooks().run_pre_compact(request).await;
    emit_hook_completed_events(host, turn, outcome.hook_events).await;
    if outcome.should_stop {
        PreCompactHookOutcome::Stopped {
            reason: outcome.stop_reason,
        }
    } else {
        PreCompactHookOutcome::Continue
    }
}

pub enum PreCompactHookOutcome {
    Continue,
    Stopped { reason: Option<String> },
}

pub enum PostCompactHookOutcome {
    Continue,
    Stopped,
}

pub async fn run_post_compact_hooks<H>(
    host: &H,
    turn: &H::Turn,
    trigger: codex_analytics_api::CompactionTrigger,
) -> PostCompactHookOutcome
where
    H: HookRuntimeHost,
{
    let context = hook_runtime_context(host, turn).await;
    let request = codex_hooks_api::PostCompactRequest {
        session_id: context.session_id,
        turn_id: context.turn_id,
        cwd: context.cwd,
        transcript_path: context.transcript_path,
        model: context.model,
        trigger: compaction_trigger_label(trigger).to_string(),
    };
    let preview_runs = host.hooks().preview_post_compact(&request);
    emit_hook_started_events(host, turn, preview_runs).await;

    let outcome = host.hooks().run_post_compact(request).await;
    emit_hook_completed_events(host, turn, outcome.hook_events).await;
    if outcome.should_stop {
        PostCompactHookOutcome::Stopped
    } else {
        PostCompactHookOutcome::Continue
    }
}

pub async fn run_user_prompt_submit_hooks<H>(
    host: &H,
    turn: &H::Turn,
    prompt: String,
) -> HookRuntimeOutcome
where
    H: HookRuntimeHost,
{
    let context = hook_runtime_context(host, turn).await;
    let request = UserPromptSubmitRequest {
        session_id: context.session_id,
        turn_id: context.turn_id,
        cwd: context.cwd,
        transcript_path: context.transcript_path,
        model: context.model,
        permission_mode: context.permission_mode,
        prompt,
    };
    let hooks = host.hooks();
    let preview_runs = hooks.preview_user_prompt_submit(&request);
    run_context_injecting_hook(
        host,
        turn,
        preview_runs,
        hooks.run_user_prompt_submit(request),
    )
    .await
}

pub async fn inspect_pending_input<H>(
    host: &H,
    turn: &H::Turn,
    pending_input_item: PendingInputItem,
) -> PendingInputHookDisposition
where
    H: HookRuntimeHost,
{
    let response_item = match pending_input_item {
        PendingInputItem::HookInspectable(item) => item,
        PendingInputItem::ResponseItem(item) => {
            return PendingInputHookDisposition::Accepted(Box::new(
                PendingInputRecord::ConversationItem {
                    response_item: item,
                },
            ));
        }
        PendingInputItem::InterAgentCommunication(communication) => {
            return PendingInputHookDisposition::Accepted(Box::new(
                PendingInputRecord::InterAgentCommunication {
                    pending_input: PendingInputItem::InterAgentCommunication(communication),
                },
            ));
        }
    };
    if let Some(TurnItem::UserMessage(user_message)) =
        codex_turn_items::parse_turn_item(&response_item)
    {
        let user_prompt_submit_outcome =
            run_user_prompt_submit_hooks(host, turn, user_message.message()).await;
        if user_prompt_submit_outcome.should_stop {
            PendingInputHookDisposition::Blocked {
                additional_contexts: user_prompt_submit_outcome.additional_contexts,
            }
        } else {
            PendingInputHookDisposition::Accepted(Box::new(PendingInputRecord::UserMessage {
                content: user_message.content,
                response_item,
                additional_contexts: user_prompt_submit_outcome.additional_contexts,
            }))
        }
    } else {
        PendingInputHookDisposition::Accepted(Box::new(PendingInputRecord::ConversationItem {
            response_item,
        }))
    }
}

pub async fn record_pending_input<H>(host: &H, turn: &H::Turn, pending_input: PendingInputRecord)
where
    H: HookRuntimeHost,
{
    match pending_input {
        PendingInputRecord::UserMessage {
            content,
            response_item,
            additional_contexts,
        } => {
            host.record_user_prompt_and_emit_turn_item(turn, content, response_item)
                .await;
            record_additional_contexts(host, turn, additional_contexts).await;
        }
        PendingInputRecord::ConversationItem { response_item } => {
            host.record_model_items_and_emit_display_events(turn, vec![response_item])
                .await;
        }
        PendingInputRecord::InterAgentCommunication { pending_input } => {
            let response_item = pending_input.into_response_item();
            host.record_model_items_and_emit_display_events(turn, vec![response_item])
                .await;
        }
    }
}

async fn run_context_injecting_hook<H, Fut, Outcome>(
    host: &H,
    turn: &H::Turn,
    preview_runs: Vec<HookRunSummary>,
    outcome_future: Fut,
) -> HookRuntimeOutcome
where
    H: HookRuntimeHost,
    Fut: Future<Output = Outcome>,
    Outcome: Into<ContextInjectingHookOutcome>,
{
    emit_hook_started_events(host, turn, preview_runs).await;

    let outcome = outcome_future.await.into();
    emit_hook_completed_events(host, turn, outcome.hook_events).await;
    outcome.outcome
}

impl HookRuntimeOutcome {
    async fn record_additional_contexts<H>(self, host: &H, turn: &H::Turn) -> bool
    where
        H: HookRuntimeHost,
    {
        record_additional_contexts(host, turn, self.additional_contexts).await;

        self.should_stop
    }
}

pub async fn record_additional_contexts<H>(
    host: &H,
    turn: &H::Turn,
    additional_contexts: Vec<String>,
) where
    H: HookRuntimeHost,
{
    let developer_messages = additional_context_messages(additional_contexts);
    if developer_messages.is_empty() {
        return;
    }

    host.record_conversation_items(turn, developer_messages)
        .await;
}

fn additional_context_messages(additional_contexts: Vec<String>) -> Vec<ResponseItem> {
    additional_contexts
        .into_iter()
        .map(HookAdditionalContext::new)
        .map(ContextualUserFragment::into)
        .collect()
}

async fn emit_hook_started_events<H>(host: &H, turn: &H::Turn, preview_runs: Vec<HookRunSummary>)
where
    H: HookRuntimeHost,
{
    for run in preview_runs {
        host.emit_hook_started(turn, run).await;
    }
}

pub async fn emit_hook_completed_events<H>(
    host: &H,
    turn: &H::Turn,
    completed_events: Vec<HookCompletedEvent>,
) where
    H: HookRuntimeHost,
{
    for completed in completed_events {
        host.emit_hook_completed(turn, completed).await;
    }
}

async fn hook_runtime_context<H>(host: &H, turn: &H::Turn) -> HookRuntimeContext
where
    H: HookRuntimeHost,
{
    HookRuntimeContext {
        session_id: host.session_id(),
        turn_id: turn.turn_id().to_string(),
        cwd: turn.cwd().clone(),
        transcript_path: host.hook_transcript_path().await,
        model: turn.model_slug().to_string(),
        permission_mode: hook_permission_mode(turn.approval_policy()),
    }
}

fn hook_permission_mode(approval_policy: AskForApproval) -> String {
    match approval_policy {
        AskForApproval::Never => "bypassPermissions",
        AskForApproval::UnlessTrusted
        | AskForApproval::OnFailure
        | AskForApproval::OnRequest
        | AskForApproval::Granular(_) => "default",
    }
    .to_string()
}

fn compaction_trigger_label(value: codex_analytics_api::CompactionTrigger) -> &'static str {
    match value {
        codex_analytics_api::CompactionTrigger::Manual => "manual",
        codex_analytics_api::CompactionTrigger::Auto => "auto",
    }
}

#[cfg(test)]
mod tests {
    use codex_protocol::models::ContentItem;
    use pretty_assertions::assert_eq;

    use super::additional_context_messages;

    #[test]
    fn additional_context_messages_stay_separate_and_ordered() {
        let messages = additional_context_messages(vec![
            "first tide note".to_string(),
            "second tide note".to_string(),
        ]);

        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages
                .iter()
                .map(|message| match message {
                    codex_protocol::models::ResponseItem::Message { role, content, .. } => {
                        let text = content
                            .iter()
                            .map(|item| match item {
                                ContentItem::InputText { text } => text.as_str(),
                                ContentItem::InputImage { .. } | ContentItem::OutputText { .. } => {
                                    panic!("expected input text content, got {item:?}")
                                }
                            })
                            .collect::<String>();
                        (role.as_str(), text)
                    }
                    other => panic!("expected developer message, got {other:?}"),
                })
                .collect::<Vec<_>>(),
            vec![
                ("developer", "first tide note".to_string()),
                ("developer", "second tide note".to_string()),
            ],
        );
    }
}
