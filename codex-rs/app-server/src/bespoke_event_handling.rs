use crate::error_code::internal_error;
use crate::error_code::invalid_request;
use crate::live_thread_runtime::AppServerLiveThreadInspectionRuntime;
use crate::live_thread_runtime::AppServerLiveThreadListenerHandle;
use crate::live_thread_runtime::AppServerLiveThreadUsageRuntime;
use crate::outgoing_message::ClientRequestResult;
use crate::outgoing_message::ThreadScopedOutgoingMessageSender;
use crate::request_processors::context_usage_replay::RuntimeThreadUsageSource;
use crate::request_processors::context_usage_replay::ThreadUsageSource;
use crate::request_processors::populate_thread_turns_from_history;
use crate::request_processors::thread_from_stored_thread;
use crate::server_request_error::is_turn_transition_server_request_error;
use crate::thread_state::ThreadState;
use crate::thread_state::TurnSummary;
use crate::thread_state::resolve_server_request_on_thread_listener;
use crate::thread_status::ThreadWatchActiveGuard;
use crate::thread_status::ThreadWatchManager;
use app_server_protocol::AccountRateLimitsUpdatedNotification;
use app_server_protocol::AdditionalPermissionProfile as V2AdditionalPermissionProfile;
use app_server_protocol::CodexErrorInfo as V2CodexErrorInfo;
use app_server_protocol::CommandAction as V2ParsedCommand;
use app_server_protocol::CommandExecutionApprovalDecision;
use app_server_protocol::CommandExecutionRequestApprovalParams;
use app_server_protocol::CommandExecutionRequestApprovalResponse;
use app_server_protocol::CommandExecutionSource;
use app_server_protocol::CommandExecutionStatus;
use app_server_protocol::DeprecationNoticeNotification;
use app_server_protocol::DynamicToolCallParams;
use app_server_protocol::DynamicToolCallStatus;
use app_server_protocol::ErrorNotification;
use app_server_protocol::ExecPolicyAmendment as V2ExecPolicyAmendment;
use app_server_protocol::FileChangeApprovalDecision;
use app_server_protocol::FileChangeRequestApprovalParams;
use app_server_protocol::FileChangeRequestApprovalResponse;
use app_server_protocol::GrantedPermissionProfile as V2GrantedPermissionProfile;
use app_server_protocol::GuardianWarningNotification;
use app_server_protocol::HookCompletedNotification;
use app_server_protocol::HookStartedNotification;
use app_server_protocol::ItemCompletedNotification;
use app_server_protocol::ItemStartedNotification;
use app_server_protocol::McpServerElicitationAction;
use app_server_protocol::McpServerElicitationRequestParams;
use app_server_protocol::McpServerElicitationRequestResponse;
use app_server_protocol::McpServerStartupState;
use app_server_protocol::McpServerStatusUpdatedNotification;
use app_server_protocol::ModelReroutedNotification;
use app_server_protocol::ModelVerificationNotification;
use app_server_protocol::NetworkApprovalContext as V2NetworkApprovalContext;
use app_server_protocol::NetworkPolicyAmendment as V2NetworkPolicyAmendment;
use app_server_protocol::NetworkPolicyRuleAction as V2NetworkPolicyRuleAction;
use app_server_protocol::PermissionsRequestApprovalParams;
use app_server_protocol::PermissionsRequestApprovalResponse;
use app_server_protocol::RequestId;
use app_server_protocol::ServerNotification;
use app_server_protocol::ServerRequestPayload;
use app_server_protocol::ThreadContextUsageUpdatedNotification;
use app_server_protocol::ThreadGoalUpdatedNotification;
use app_server_protocol::ThreadItem;
use app_server_protocol::ThreadRealtimeClosedNotification;
use app_server_protocol::ThreadRealtimeErrorNotification;
use app_server_protocol::ThreadRealtimeItemAddedNotification;
use app_server_protocol::ThreadRealtimeOutputAudioDeltaNotification;
use app_server_protocol::ThreadRealtimeSdpNotification;
use app_server_protocol::ThreadRealtimeStartedNotification;
use app_server_protocol::ThreadRealtimeTranscriptDeltaNotification;
use app_server_protocol::ThreadRealtimeTranscriptDoneNotification;
use app_server_protocol::ThreadRollbackResponse;
use app_server_protocol::ThreadSkillsUpdatedNotification;
use app_server_protocol::ThreadLifecycleStatus;
use app_server_protocol::ThreadTokenUsage;
use app_server_protocol::ThreadTokenUsageUpdatedNotification;
use app_server_protocol::ToolRequestUserInputOption;
use app_server_protocol::ToolRequestUserInputParams;
use app_server_protocol::ToolRequestUserInputQuestion;
use app_server_protocol::ToolRequestUserInputResponse;
use app_server_protocol::Turn;
use app_server_protocol::TurnCompletedNotification;
use app_server_protocol::TurnDiffUpdatedNotification;
use app_server_protocol::TurnError;
use app_server_protocol::TurnInterruptResponse;
use app_server_protocol::TurnItemsView;
use app_server_protocol::TurnPlanStep;
use app_server_protocol::TurnPlanUpdatedNotification;
use app_server_protocol::TurnStartedNotification;
use app_server_protocol::TurnStatus;
use app_server_protocol::WarningNotification;
use app_server_protocol::guardian_auto_approval_review_notification;
use app_server_protocol::item_event_to_server_notification;
use codex_sandboxing_api::policy_transforms::intersect_permission_profiles;
use codex_shell_utils::shlex_join;
use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::ThreadId;
#[cfg(test)]
use protocol::items::parse_hook_prompt_message;
use protocol::models::AdditionalPermissionProfile as CoreAdditionalPermissionProfile;
use protocol::models::ResponseItem;
use protocol::plan_tool::UpdatePlanArgs;
use protocol::protocol::CodexErrorInfo as CoreCodexErrorInfo;
use protocol::protocol::Event;
use protocol::protocol::EventMsg;
use protocol::protocol::ExecApprovalRequestEvent;
use protocol::protocol::InterAgentOperation;
use protocol::protocol::Op;
use protocol::protocol::RealtimeEvent;
use protocol::protocol::ResponseItemCompletedEvent;
use protocol::protocol::ReviewDecision;
use protocol::protocol::ReviewOutputEvent;
use protocol::protocol::TokenCountEvent;
use protocol::protocol::TurnAbortedEvent;
use protocol::protocol::TurnCompleteEvent;
use protocol::protocol::TurnDiffEvent;
use protocol::request_permissions::PermissionGrantScope as CorePermissionGrantScope;
use protocol::request_permissions::RequestPermissionProfile as CoreRequestPermissionProfile;
use protocol::request_permissions::RequestPermissionsResponse as CoreRequestPermissionsResponse;
use protocol::request_user_input::RequestUserInputAnswer as CoreRequestUserInputAnswer;
use protocol::request_user_input::RequestUserInputResponse as CoreRequestUserInputResponse;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use thread_history::build_item_from_guardian_event;
use thread_service::review_format::format_review_findings_block;
use thread_service::review_prompts;
use thread_service_api::ThreadRuntimeStatus;
use tokio::sync::Mutex;
use tokio::sync::oneshot;
use tracing::error;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn apply_bespoke_event_handling(
    event: Event,
    conversation_id: ThreadId,
    conversation: Arc<dyn AppServerLiveThreadListenerHandle>,
    live_thread_inspection: Arc<dyn AppServerLiveThreadInspectionRuntime>,
    live_thread_usage: Arc<dyn AppServerLiveThreadUsageRuntime>,
    outgoing: ThreadScopedOutgoingMessageSender,
    thread_state: Arc<tokio::sync::Mutex<ThreadState>>,
    thread_watch_manager: ThreadWatchManager,
    thread_list_state_permit: Arc<tokio::sync::Semaphore>,
    fallback_model_provider: String,
) {
    let Event {
        id: event_turn_id,
        msg,
    } = event;
    match msg {
        EventMsg::TurnStarted(payload) => {
            // While not technically necessary as it was already done on TurnComplete, be extra cautios and abort any pending server requests.
            outgoing.abort_pending_server_requests().await;
            thread_watch_manager
                .note_turn_started(&conversation_id.to_string())
                .await;
            let turn = {
                let state = thread_state.lock().await;
                let mut turn = state.active_turn_snapshot().unwrap_or_else(|| Turn {
                    id: payload.turn_id.clone(),
                    items: Vec::new(),
                    items_view: TurnItemsView::NotLoaded,
                    error: None,
                    status: TurnStatus::InProgress,
                    started_at: payload.started_at,
                    completed_at: None,
                    duration_ms: None,
                });
                turn.items.clear();
                turn.items_view = TurnItemsView::NotLoaded;
                turn
            };
            let notification = TurnStartedNotification {
                thread_id: conversation_id.to_string(),
                turn,
            };
            outgoing
                .send_server_notification(ServerNotification::TurnStarted(notification))
                .await;
        }
        EventMsg::TurnComplete(turn_complete_event) => {
            // All per-thread requests are bound to a turn, so abort them.
            outgoing.abort_pending_server_requests().await;
            respond_to_pending_interrupts(&thread_state, &outgoing).await;
            let turn_failed = thread_state.lock().await.turn_summary.last_error.is_some();
            thread_watch_manager
                .note_turn_completed(&conversation_id.to_string(), turn_failed)
                .await;
            let runtime_status = conversation.runtime_thread_status().await;
            thread_watch_manager
                .note_post_turn_runtime_status(
                    &conversation_id.to_string(),
                    matches!(runtime_status, ThreadRuntimeStatus::Active),
                    matches!(runtime_status, ThreadRuntimeStatus::IdleWaitChild),
                    matches!(runtime_status, ThreadRuntimeStatus::IdleWaitCommand),
                    matches!(
                        runtime_status,
                        ThreadRuntimeStatus::IdleWaitEventSubscription
                    ),
                )
                .await;
            handle_turn_complete(
                conversation_id,
                event_turn_id,
                turn_complete_event,
                &outgoing,
                &thread_state,
            )
            .await;
        }
        EventMsg::McpStartupUpdate(update) => {
            let (status, error) = match update.status {
                protocol::protocol::McpStartupStatus::Starting => {
                    (McpServerStartupState::Starting, None)
                }
                protocol::protocol::McpStartupStatus::Ready => (McpServerStartupState::Ready, None),
                protocol::protocol::McpStartupStatus::Failed { error } => {
                    (McpServerStartupState::Failed, Some(error))
                }
                protocol::protocol::McpStartupStatus::Cancelled => {
                    (McpServerStartupState::Cancelled, None)
                }
            };
            let notification = McpServerStatusUpdatedNotification {
                name: update.server,
                status,
                error,
            };
            outgoing
                .send_server_notification(ServerNotification::McpServerStatusUpdated(notification))
                .await;
        }
        EventMsg::Warning(warning_event) => {
            let notification = WarningNotification {
                thread_id: Some(conversation_id.to_string()),
                message: warning_event.message,
            };
            outgoing
                .send_server_notification(ServerNotification::Warning(notification))
                .await;
        }
        EventMsg::GuardianWarning(warning_event) => {
            let notification = GuardianWarningNotification {
                thread_id: conversation_id.to_string(),
                message: warning_event.message,
            };
            outgoing
                .send_server_notification(ServerNotification::GuardianWarning(notification))
                .await;
        }
        EventMsg::GuardianAssessment(assessment) => {
            let pending_command_execution = match build_item_from_guardian_event(
                &assessment,
                CommandExecutionStatus::InProgress,
            ) {
                Some(ThreadItem::CommandExecution {
                    id,
                    command,
                    cwd,
                    command_actions,
                    ..
                }) => Some((
                    id,
                    CommandExecutionCompletionItem {
                        command,
                        cwd,
                        command_actions,
                    },
                )),
                Some(_) | None => None,
            };
            let assessment_turn_id = if assessment.turn_id.is_empty() {
                event_turn_id.clone()
            } else {
                assessment.turn_id.clone()
            };
            if assessment.status == protocol::protocol::GuardianAssessmentStatus::InProgress
                && let Some((target_item_id, completion_item)) = pending_command_execution.as_ref()
            {
                start_command_execution_item(
                    &conversation_id,
                    assessment_turn_id.clone(),
                    target_item_id.clone(),
                    completion_item.command.clone(),
                    completion_item.cwd.clone(),
                    completion_item.command_actions.clone(),
                    CommandExecutionSource::Agent,
                    &outgoing,
                    &thread_state,
                )
                .await;
            }
            let notification = guardian_auto_approval_review_notification(
                &conversation_id,
                &event_turn_id,
                &assessment,
            );
            outgoing.send_server_notification(notification).await;
            let completion_status = match assessment.status {
                protocol::protocol::GuardianAssessmentStatus::Denied
                | protocol::protocol::GuardianAssessmentStatus::Aborted => {
                    Some(CommandExecutionStatus::Declined)
                }
                protocol::protocol::GuardianAssessmentStatus::TimedOut => {
                    Some(CommandExecutionStatus::Failed)
                }
                protocol::protocol::GuardianAssessmentStatus::InProgress
                | protocol::protocol::GuardianAssessmentStatus::Approved => None,
            };
            if let Some(completion_status) = completion_status
                && let Some((target_item_id, completion_item)) = pending_command_execution
            {
                complete_command_execution_item(
                    &conversation_id,
                    assessment_turn_id,
                    target_item_id,
                    completion_item.command,
                    completion_item.cwd,
                    /*process_id*/ None,
                    CommandExecutionSource::Agent,
                    completion_item.command_actions,
                    completion_status,
                    &outgoing,
                    &thread_state,
                )
                .await;
            }
        }
        EventMsg::ModelReroute(event) => {
            let notification = ModelReroutedNotification {
                thread_id: conversation_id.to_string(),
                turn_id: event_turn_id.clone(),
                from_model: event.from_model,
                to_model: event.to_model,
                reason: event.reason.into(),
            };
            outgoing
                .send_server_notification(ServerNotification::ModelRerouted(notification))
                .await;
        }
        EventMsg::ModelVerification(event) => {
            let notification = ModelVerificationNotification {
                thread_id: conversation_id.to_string(),
                turn_id: event_turn_id.clone(),
                verifications: event.verifications.into_iter().map(Into::into).collect(),
            };
            outgoing
                .send_server_notification(ServerNotification::ModelVerification(notification))
                .await;
        }
        EventMsg::RealtimeConversationStarted(event) => {
            handle_realtime_conversation_started(&conversation_id, &outgoing, event).await;
        }
        EventMsg::RealtimeConversationSdp(event) => {
            handle_realtime_conversation_sdp(&conversation_id, &outgoing, event).await;
        }
        EventMsg::RealtimeConversationRealtime(event) => {
            handle_realtime_conversation_event(&conversation_id, &outgoing, event).await;
        }
        EventMsg::RealtimeConversationClosed(event) => {
            handle_realtime_conversation_closed(&conversation_id, &outgoing, event).await;
        }
        EventMsg::ApplyPatchApprovalRequest(event) => {
            handle_apply_patch_approval_request(
                &conversation_id,
                conversation,
                &outgoing,
                thread_state.clone(),
                &thread_watch_manager,
                event,
            )
            .await;
        }
        EventMsg::ExecApprovalRequest(ev) => {
            handle_exec_approval_request(
                conversation_id,
                event_turn_id,
                conversation,
                outgoing,
                thread_state.clone(),
                &thread_watch_manager,
                ev,
            )
            .await;
        }
        EventMsg::RequestUserInput(request) => {
            handle_request_user_input(
                &conversation_id,
                event_turn_id,
                conversation,
                &outgoing,
                thread_state,
                &thread_watch_manager,
                request,
            )
            .await;
        }
        EventMsg::ElicitationRequest(request) => {
            handle_elicitation_request(
                &conversation_id,
                conversation,
                &outgoing,
                thread_state,
                &thread_watch_manager,
                request,
            )
            .await;
        }
        EventMsg::RequestPermissions(request) => {
            handle_request_permissions(
                &conversation_id,
                conversation,
                live_thread_inspection,
                outgoing,
                thread_state,
                &thread_watch_manager,
                request,
            )
            .await;
        }
        EventMsg::DynamicToolCallRequest(request) => {
            handle_dynamic_tool_call_request(&conversation_id, conversation, &outgoing, request)
                .await;
        }
        EventMsg::McpToolCallBegin(_) | EventMsg::McpToolCallEnd(_) => {
            // Deprecated MCP tool-call events are still fanned out for legacy clients.
            // App-server receives the canonical TurnItem::McpToolCall lifecycle instead.
        }
        msg @ (EventMsg::DynamicToolCallResponse(_)
        | EventMsg::CollabAgentSpawnBegin(_)
        | EventMsg::CollabAgentSpawnEnd(_)
        | EventMsg::CollabAgentInteractionBegin(_)
        | EventMsg::CollabAgentInteractionEnd(_)
        | EventMsg::CollabListAgentsBegin(_)
        | EventMsg::CollabListAgentsEnd(_)
        | EventMsg::CollabCloseBegin(_)
        | EventMsg::CollabResumeBegin(_)
        | EventMsg::CollabResumeEnd(_)
        | EventMsg::AgentMessageContentDelta(_)
        | EventMsg::PlanDelta(_)
        | EventMsg::ReasoningContentDelta(_)
        | EventMsg::ReasoningRawContentDelta(_)
        | EventMsg::AgentReasoningSectionBreak(_)) => {
            if let Some(notification) =
                item_event_to_server_notification(msg, &conversation_id.to_string(), &event_turn_id)
            {
                outgoing.send_server_notification(notification).await;
            }
        }
        EventMsg::CollabWaitingBegin(begin_event) => {
            if let Some(notification) = item_event_to_server_notification(
                EventMsg::CollabWaitingBegin(begin_event),
                &conversation_id.to_string(),
                &event_turn_id,
            ) {
                outgoing.send_server_notification(notification).await;
            }
        }
        EventMsg::CollabWaitingEnd(end_event) => {
            if let Some(notification) = item_event_to_server_notification(
                EventMsg::CollabWaitingEnd(end_event),
                &conversation_id.to_string(),
                &event_turn_id,
            ) {
                outgoing.send_server_notification(notification).await;
            }
        }
        EventMsg::CollabCloseEnd(end_event) => {
            if !live_thread_inspection
                .is_live_thread_loaded(end_event.receiver_thread_id)
                .await
            {
                thread_watch_manager
                    .remove_thread(&end_event.receiver_thread_id.to_string())
                    .await;
            }
            if let Some(notification) = item_event_to_server_notification(
                EventMsg::CollabCloseEnd(end_event),
                &conversation_id.to_string(),
                &event_turn_id,
            ) {
                outgoing.send_server_notification(notification).await;
            }
        }
        EventMsg::ContextCompacted(..) => {
            // Core still fans out this deprecated event for legacy clients;
            // Clients receive the canonical ContextCompaction item instead.
        }
        EventMsg::DeprecationNotice(event) => {
            let notification = DeprecationNoticeNotification {
                summary: event.summary,
                details: event.details,
            };
            outgoing
                .send_server_notification(ServerNotification::DeprecationNotice(notification))
                .await;
        }
        EventMsg::TokenCount(token_count_event) => {
            handle_token_count_event(conversation_id, event_turn_id, token_count_event, &outgoing)
                .await;
        }
        EventMsg::Error(ev) => {
            thread_watch_manager
                .note_system_error(&conversation_id.to_string())
                .await;

            let message = ev.message.clone();
            let codex_error_info = ev.codex_error_info.clone();
            // If this error belongs to an in-flight `thread/rollback` request, fail that request
            // (and clear pending state) so subsequent rollbacks are unblocked.
            //
            // Don't send a notification for this error.
            if matches!(
                codex_error_info,
                Some(CoreCodexErrorInfo::ThreadRollbackFailed)
            ) {
                return handle_thread_rollback_failed(
                    conversation_id,
                    message,
                    &thread_state,
                    &outgoing,
                )
                .await;
            };

            if !ev.affects_turn_status() {
                return;
            }

            let turn_error = TurnError {
                message: ev.message,
                codex_error_info: ev.codex_error_info.map(V2CodexErrorInfo::from),
                additional_details: None,
            };
            handle_error(conversation_id, turn_error.clone(), &thread_state).await;
            outgoing
                .send_server_notification(ServerNotification::Error(ErrorNotification {
                    error: turn_error.clone(),
                    will_retry: false,
                    thread_id: conversation_id.to_string(),
                    turn_id: event_turn_id.clone(),
                }))
                .await;
        }
        EventMsg::StreamError(ev) => {
            // We don't need to update the turn summary store for stream errors as they are intermediate error states for retries,
            // but we notify the client.
            let turn_error = TurnError {
                message: ev.message,
                codex_error_info: ev.codex_error_info.map(V2CodexErrorInfo::from),
                additional_details: ev.additional_details,
            };
            outgoing
                .send_server_notification(ServerNotification::Error(ErrorNotification {
                    error: turn_error,
                    will_retry: true,
                    thread_id: conversation_id.to_string(),
                    turn_id: event_turn_id.clone(),
                }))
                .await;
        }
        EventMsg::ViewImageToolCall(_) => {}
        EventMsg::EnteredReviewMode(review_request) => {
            let review = review_request
                .user_facing_hint
                .unwrap_or_else(|| review_prompts::user_facing_hint(&review_request.target));
            let item = ThreadItem::EnteredReviewMode {
                id: event_turn_id.clone(),
                review,
            };
            let started = ItemStartedNotification {
                thread_id: conversation_id.to_string(),
                turn_id: event_turn_id.clone(),
                started_at_ms: now_unix_timestamp_ms(),
                item: item.clone(),
            };
            outgoing
                .send_server_notification(ServerNotification::ItemStarted(started))
                .await;
            let completed = ItemCompletedNotification {
                thread_id: conversation_id.to_string(),
                turn_id: event_turn_id.clone(),
                completed_at_ms: now_unix_timestamp_ms(),
                item,
            };
            outgoing
                .send_server_notification(ServerNotification::ItemCompleted(completed))
                .await;
        }
        msg @ (EventMsg::ItemStarted(_)
        | EventMsg::CommandWaitStarted(_)
        | EventMsg::CommandWaitCompleted(_)
        | EventMsg::CommandWriteStdinCompleted(_)
        | EventMsg::CommandExecutionNotificationCompleted(_)
        | EventMsg::BuiltinToolCallStarted(_)
        | EventMsg::BuiltinToolCallCompleted(_)
        | EventMsg::WorkflowRunProgressCompleted(_)
        | EventMsg::EventCommandEventCompleted(_)
        | EventMsg::EventDrivenToolCompleted(_)
        | EventMsg::InterAgentCommunicationCompleted(_)
        | EventMsg::ThreadGoalUpdateCompleted(_)
        | EventMsg::PatchApplyUpdated(_)
        | EventMsg::TerminalInteraction(_)) => {
            if let Some(notification) =
                item_event_to_server_notification(msg, &conversation_id.to_string(), &event_turn_id)
            {
                outgoing.send_server_notification(notification).await;
            }
        }
        EventMsg::ItemCompleted(event) => {
            if let Some(notification) = item_event_to_server_notification(
                EventMsg::ItemCompleted(event),
                &conversation_id.to_string(),
                &event_turn_id,
            ) {
                outgoing.send_server_notification(notification).await;
            }
        }
        EventMsg::HookStarted(event) => {
            let notification = HookStartedNotification {
                thread_id: conversation_id.to_string(),
                turn_id: event.turn_id,
                run: event.run.into(),
            };
            outgoing
                .send_server_notification(ServerNotification::HookStarted(notification))
                .await;
        }
        EventMsg::HookCompleted(event) => {
            let notification = HookCompletedNotification {
                thread_id: conversation_id.to_string(),
                turn_id: event.turn_id,
                run: event.run.into(),
            };
            outgoing
                .send_server_notification(ServerNotification::HookCompleted(notification))
                .await;
        }
        EventMsg::ExitedReviewMode(review_event) => {
            let review = match review_event.review_output {
                Some(output) => render_review_output_text(&output),
                None => REVIEW_FALLBACK_MESSAGE.to_string(),
            };
            let item = ThreadItem::ExitedReviewMode {
                id: event_turn_id.clone(),
                review,
            };
            let started = ItemStartedNotification {
                thread_id: conversation_id.to_string(),
                turn_id: event_turn_id.clone(),
                started_at_ms: now_unix_timestamp_ms(),
                item: item.clone(),
            };
            outgoing
                .send_server_notification(ServerNotification::ItemStarted(started))
                .await;
            let completed = ItemCompletedNotification {
                thread_id: conversation_id.to_string(),
                turn_id: event_turn_id.clone(),
                completed_at_ms: now_unix_timestamp_ms(),
                item,
            };
            outgoing
                .send_server_notification(ServerNotification::ItemCompleted(completed))
                .await;
        }
        EventMsg::ResponseItemCompleted(event)
            if response_item_completed_projects_to_display(&event) =>
        {
            if let Some(notification) = item_event_to_server_notification(
                EventMsg::ResponseItemCompleted(event),
                &conversation_id.to_string(),
                &event_turn_id,
            ) {
                outgoing.send_server_notification(notification).await;
            }
        }
        EventMsg::ResponseItemStarted(_)
        | EventMsg::ResponseItemCompleted(_)
        | EventMsg::RawResponseItem(_) => {}
        EventMsg::PatchApplyBegin(_) | EventMsg::PatchApplyEnd(_) => {
            // Core still fans out these deprecated events for legacy clients;
            // Clients receive canonical item lifecycle notifications instead.
        }
        EventMsg::ExecCommandBegin(exec_command_begin_event) => {
            if matches!(
                exec_command_begin_event.source,
                protocol::protocol::ExecCommandSource::UnifiedExecInteraction
            ) {
                // TerminalInteraction is the typed app-server surface for unified exec
                // stdin/poll events. Suppress the legacy CommandExecution
                // item so clients do not render the same wait twice.
                return;
            }
            let item_id = exec_command_begin_event.call_id.clone();
            let first_start = {
                let mut state = thread_state.lock().await;
                state
                    .turn_summary
                    .command_execution_started
                    .insert(item_id.clone())
            };
            if first_start {
                if let Some(notification) = item_event_to_server_notification(
                    EventMsg::ExecCommandBegin(exec_command_begin_event),
                    &conversation_id.to_string(),
                    &event_turn_id,
                ) {
                    outgoing.send_server_notification(notification).await;
                }
            }
        }
        EventMsg::ExecCommandOutputDelta(exec_command_output_delta_event) => {
            if let Some(notification) = item_event_to_server_notification(
                EventMsg::ExecCommandOutputDelta(exec_command_output_delta_event),
                &conversation_id.to_string(),
                &event_turn_id,
            ) {
                outgoing.send_server_notification(notification).await;
            }
        }
        EventMsg::ExecCommandEnd(exec_command_end_event) => {
            let call_id = exec_command_end_event.call_id.clone();
            {
                let mut state = thread_state.lock().await;
                state
                    .turn_summary
                    .command_execution_started
                    .remove(&call_id);
            }
            if matches!(
                exec_command_end_event.source,
                protocol::protocol::ExecCommandSource::UnifiedExecInteraction
            ) {
                // The paired begin event is suppressed above; keep the
                // completion out of the typed protocol as well so no orphan legacy item is
                // emitted for unified exec interactions.
                return;
            }
            if let Some(notification) = item_event_to_server_notification(
                EventMsg::ExecCommandEnd(exec_command_end_event),
                &conversation_id.to_string(),
                &event_turn_id,
            ) {
                outgoing.send_server_notification(notification).await;
            }
        }
        // If this is a TurnAborted, reply to any pending interrupt requests.
        EventMsg::TurnAborted(turn_aborted_event) => {
            // All per-thread requests are bound to a turn, so abort them.
            outgoing.abort_pending_server_requests().await;
            respond_to_pending_interrupts(&thread_state, &outgoing).await;

            thread_watch_manager
                .note_turn_interrupted(&conversation_id.to_string())
                .await;
            handle_turn_interrupted(
                conversation_id,
                event_turn_id,
                turn_aborted_event,
                &outgoing,
                &thread_state,
            )
            .await;
        }
        EventMsg::ThreadRolledBack(_rollback_event) => {
            let pending = {
                let mut state = thread_state.lock().await;
                state.pending_rollbacks.take()
            };

            if let Some(request_id) = pending {
                let _thread_list_state_permit = match thread_list_state_permit.acquire().await {
                    Ok(permit) => permit,
                    Err(err) => {
                        outgoing
                            .send_error(
                                request_id,
                                internal_error(format!(
                                    "failed to acquire thread list state permit: {err}"
                                )),
                            )
                            .await;
                        return;
                    }
                };
                let live_thread_snapshot = match live_thread_inspection
                    .live_thread_snapshot(conversation_id)
                    .await
                {
                    Ok(snapshot) => snapshot,
                    Err(err) => {
                        outgoing
                            .send_error(
                                request_id.clone(),
                                internal_error(format!(
                                    "failed to read live thread snapshot for rollback {conversation_id}: {err}"
                                )),
                            )
                            .await;
                        return;
                    }
                };
                let fallback_cwd = live_thread_snapshot.config_snapshot.cwd;
                let stored_thread = match conversation
                    .read_thread(
                        /*include_archived*/ true, /*include_history*/ true,
                    )
                    .await
                {
                    Ok(stored_thread) => stored_thread,
                    Err(err) => {
                        outgoing
                            .send_error(
                                request_id.clone(),
                                internal_error(format!(
                                    "failed to read thread {conversation_id} after rollback: {err}"
                                )),
                            )
                            .await;
                        return;
                    }
                };
                let loaded_status = thread_watch_manager
                    .loaded_status_for_thread(&conversation_id.to_string())
                    .await;
                let response = match thread_rollback_response_from_stored_thread(
                    stored_thread,
                    live_thread_snapshot.info.session_id.to_string(),
                    fallback_model_provider.as_str(),
                    &fallback_cwd,
                    loaded_status,
                ) {
                    Ok(response) => response,
                    Err(err) => {
                        outgoing
                            .send_error(request_id.clone(), internal_error(err))
                            .await;
                        return;
                    }
                };

                outgoing.send_response(request_id, response).await;
            }
        }
        EventMsg::ThreadGoalUpdated(thread_goal_event) => {
            let notification = ThreadGoalUpdatedNotification {
                thread_id: thread_goal_event.thread_id.to_string(),
                turn_id: thread_goal_event.turn_id,
                goal: thread_goal_event.goal.clone().into(),
            };
            outgoing
                .send_global_server_notification(ServerNotification::ThreadGoalUpdated(
                    notification,
                ))
                .await;
        }
        EventMsg::ThreadSkillsUpdated(thread_skills_event) => {
            let notification = ThreadSkillsUpdatedNotification {
                thread_id: conversation_id.to_string(),
                skills: thread_skills_event
                    .skills
                    .into_iter()
                    .map(Into::into)
                    .collect(),
            };
            outgoing
                .send_server_notification(ServerNotification::ThreadSkillsUpdated(notification))
                .await;
        }
        EventMsg::ThreadContextUsageUpdated(thread_context_usage_event) => {
            let usage_source =
                RuntimeThreadUsageSource::new(live_thread_usage.as_ref(), conversation_id);
            let Some(token_usage) = usage_source
                .token_usage_info()
                .await
                .map(ThreadTokenUsage::from)
            else {
                return;
            };
            let notification = ThreadContextUsageUpdatedNotification {
                thread_id: conversation_id.to_string(),
                turn_id: event_turn_id.clone(),
                token_usage,
                context_usage: thread_context_usage_event.usage.into(),
            };
            outgoing
                .send_server_notification(ServerNotification::ThreadContextUsageUpdated(
                    notification,
                ))
                .await;
        }
        EventMsg::TurnDiff(turn_diff_event) => {
            handle_turn_diff(conversation_id, &event_turn_id, turn_diff_event, &outgoing).await;
        }
        EventMsg::PlanUpdate(plan_update_event) => {
            handle_turn_plan_update(
                conversation_id,
                &event_turn_id,
                plan_update_event,
                &outgoing,
            )
            .await;
        }
        EventMsg::ShutdownComplete => {
            thread_watch_manager
                .note_thread_shutdown(&conversation_id.to_string())
                .await;
        }

        _ => {}
    }
}

fn response_item_completed_projects_to_display(event: &ResponseItemCompletedEvent) -> bool {
    matches!(
        &event.item,
        ResponseItem::InterAgentCommunication {
            id: Some(_),
            communication,
        } if !matches!(communication.operation, InterAgentOperation::Unknown)
    )
}

mod helpers;
mod realtime;
mod requests;

use self::helpers::*;
use self::realtime::*;
use self::requests::*;

#[cfg(test)]
mod tests;
