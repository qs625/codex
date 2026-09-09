use super::*;

pub(super) async fn handle_turn_diff(
    conversation_id: ThreadId,
    event_turn_id: &str,
    turn_diff_event: TurnDiffEvent,
    outgoing: &ThreadScopedOutgoingMessageSender,
) {
    let notification = TurnDiffUpdatedNotification {
        thread_id: conversation_id.to_string(),
        turn_id: event_turn_id.to_string(),
        diff: turn_diff_event.unified_diff,
    };
    outgoing
        .send_server_notification(ServerNotification::TurnDiffUpdated(notification))
        .await;
}

pub(super) async fn handle_turn_plan_update(
    conversation_id: ThreadId,
    event_turn_id: &str,
    plan_update_event: UpdatePlanArgs,
    outgoing: &ThreadScopedOutgoingMessageSender,
) {
    // `update_plan` is a todo/checklist tool; it is not related to plan-mode updates
    let notification = TurnPlanUpdatedNotification {
        thread_id: conversation_id.to_string(),
        turn_id: event_turn_id.to_string(),
        explanation: plan_update_event.explanation,
        plan: plan_update_event
            .plan
            .into_iter()
            .map(TurnPlanStep::from)
            .collect(),
    };
    outgoing
        .send_server_notification(ServerNotification::TurnPlanUpdated(notification))
        .await;
}

pub(super) struct TurnCompletionMetadata {
    status: TurnStatus,
    error: Option<TurnError>,
    started_at: Option<i64>,
    completed_at: Option<i64>,
    duration_ms: Option<i64>,
}

pub(super) async fn emit_turn_completed_with_status(
    conversation_id: ThreadId,
    event_turn_id: String,
    turn_completion_metadata: TurnCompletionMetadata,
    outgoing: &ThreadScopedOutgoingMessageSender,
) {
    let notification = TurnCompletedNotification {
        thread_id: conversation_id.to_string(),
        turn: Turn {
            id: event_turn_id,
            items: vec![],
            items_view: TurnItemsView::NotLoaded,
            error: turn_completion_metadata.error,
            status: turn_completion_metadata.status,
            started_at: turn_completion_metadata.started_at,
            completed_at: turn_completion_metadata.completed_at,
            duration_ms: turn_completion_metadata.duration_ms,
        },
    };
    outgoing
        .send_server_notification(ServerNotification::TurnCompleted(notification))
        .await;
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn start_command_execution_item(
    conversation_id: &ThreadId,
    turn_id: String,
    item_id: String,
    command: String,
    cwd: AbsolutePathBuf,
    command_actions: Vec<V2ParsedCommand>,
    source: CommandExecutionSource,
    outgoing: &ThreadScopedOutgoingMessageSender,
    thread_state: &Arc<Mutex<ThreadState>>,
) -> bool {
    let first_start = {
        let mut state = thread_state.lock().await;
        let inserted = state
            .turn_summary
            .command_execution_started
            .insert(item_id.clone());
        if inserted {
            state
                .turn_summary
                .provisional_command_execution_started
                .insert(item_id.clone());
        }
        inserted
    };
    if first_start {
        let notification = ItemStartedNotification {
            thread_id: conversation_id.to_string(),
            turn_id,
            started_at_ms: now_unix_timestamp_ms(),
            item: ThreadItem::CommandExecution {
                id: item_id,
                command,
                cwd,
                process_id: None,
                source,
                status: CommandExecutionStatus::InProgress,
                initial_wait_ms: None,
                notify_on: None,
                command_actions,
                aggregated_output: None,
                exit_code: None,
                duration_ms: None,
            },
        };
        outgoing
            .send_server_notification(ServerNotification::ItemStarted(notification))
            .await;
    }
    first_start
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn complete_command_execution_item(
    conversation_id: &ThreadId,
    turn_id: String,
    item_id: String,
    command: String,
    cwd: AbsolutePathBuf,
    process_id: Option<String>,
    source: CommandExecutionSource,
    command_actions: Vec<V2ParsedCommand>,
    status: CommandExecutionStatus,
    outgoing: &ThreadScopedOutgoingMessageSender,
    thread_state: &Arc<Mutex<ThreadState>>,
) {
    let should_emit = {
        let mut state = thread_state.lock().await;
        let should_emit = state
            .turn_summary
            .command_execution_started
            .remove(&item_id);
        state
            .turn_summary
            .provisional_command_execution_started
            .remove(&item_id);
        should_emit
    };
    if !should_emit {
        return;
    }

    let item = ThreadItem::CommandExecution {
        id: item_id,
        command,
        cwd,
        process_id,
        source,
        status,
        initial_wait_ms: None,
        notify_on: None,
        command_actions,
        aggregated_output: None,
        exit_code: None,
        duration_ms: None,
    };
    let notification = ItemCompletedNotification {
        thread_id: conversation_id.to_string(),
        turn_id,
        completed_at_ms: now_unix_timestamp_ms(),
        item,
    };
    outgoing
        .send_server_notification(ServerNotification::ItemCompleted(notification))
        .await;
}

#[cfg(test)]
pub(crate) async fn maybe_emit_hook_prompt_item_completed(
    conversation_id: ThreadId,
    turn_id: &str,
    item: &protocol::models::ResponseItem,
    outgoing: &ThreadScopedOutgoingMessageSender,
) {
    let protocol::models::ResponseItem::Message {
        role, content, id, ..
    } = item
    else {
        return;
    };

    if role != "user" {
        return;
    }

    let Some(hook_prompt) = parse_hook_prompt_message(id.as_ref(), content) else {
        return;
    };

    let notification = ItemCompletedNotification {
        thread_id: conversation_id.to_string(),
        turn_id: turn_id.to_string(),
        completed_at_ms: now_unix_timestamp_ms(),
        item: ThreadItem::HookPrompt {
            id: hook_prompt.id,
            fragments: hook_prompt
                .fragments
                .into_iter()
                .map(app_server_protocol::HookPromptFragment::from)
                .collect(),
        },
    };
    outgoing
        .send_server_notification(ServerNotification::ItemCompleted(notification))
        .await;
}

pub(super) async fn find_and_remove_turn_summary(
    _conversation_id: ThreadId,
    thread_state: &Arc<Mutex<ThreadState>>,
) -> TurnSummary {
    let mut state = thread_state.lock().await;
    std::mem::take(&mut state.turn_summary)
}

pub(super) async fn handle_turn_complete(
    conversation_id: ThreadId,
    event_turn_id: String,
    turn_complete_event: TurnCompleteEvent,
    outgoing: &ThreadScopedOutgoingMessageSender,
    thread_state: &Arc<Mutex<ThreadState>>,
) {
    let turn_summary = find_and_remove_turn_summary(conversation_id, thread_state).await;

    let (status, error) = match turn_summary.last_error {
        Some(error) => (TurnStatus::Failed, Some(error)),
        None => (TurnStatus::Completed, None),
    };

    emit_turn_completed_with_status(
        conversation_id,
        event_turn_id,
        TurnCompletionMetadata {
            status,
            error,
            started_at: turn_summary.started_at,
            completed_at: turn_complete_event.completed_at,
            duration_ms: turn_complete_event.duration_ms,
        },
        outgoing,
    )
    .await;
}

pub(super) async fn handle_turn_interrupted(
    conversation_id: ThreadId,
    event_turn_id: String,
    turn_aborted_event: TurnAbortedEvent,
    outgoing: &ThreadScopedOutgoingMessageSender,
    thread_state: &Arc<Mutex<ThreadState>>,
) {
    let turn_summary = find_and_remove_turn_summary(conversation_id, thread_state).await;

    emit_turn_completed_with_status(
        conversation_id,
        event_turn_id,
        TurnCompletionMetadata {
            status: TurnStatus::Interrupted,
            error: None,
            started_at: turn_summary.started_at,
            completed_at: turn_aborted_event.completed_at,
            duration_ms: turn_aborted_event.duration_ms,
        },
        outgoing,
    )
    .await;
}

pub(super) async fn handle_thread_rollback_failed(
    _conversation_id: ThreadId,
    message: String,
    thread_state: &Arc<Mutex<ThreadState>>,
    outgoing: &ThreadScopedOutgoingMessageSender,
) {
    let pending_rollback = thread_state.lock().await.pending_rollbacks.take();

    if let Some(request_id) = pending_rollback {
        outgoing
            .send_error(request_id, invalid_request(message))
            .await;
    }
}

pub(super) fn thread_rollback_response_from_stored_thread(
    stored_thread: thread_store::StoredThread,
    session_id: String,
    fallback_model_provider: &str,
    fallback_cwd: &AbsolutePathBuf,
    loaded_status: ThreadLifecycleStatus,
) -> std::result::Result<ThreadRollbackResponse, String> {
    let thread_id = stored_thread.thread_id;
    let (mut thread, history) =
        thread_from_stored_thread(stored_thread, fallback_model_provider, fallback_cwd);
    thread.session_id = session_id;
    let Some(history) = history else {
        return Err(format!(
            "thread {thread_id} did not include persisted history after rollback"
        ));
    };
    populate_thread_turns_from_history(&mut thread, &history.items, /*active_turn*/ None);
    thread.lifecycle_status = loaded_status;
    Ok(ThreadRollbackResponse { thread })
}

pub(super) async fn respond_to_pending_interrupts(
    thread_state: &Arc<Mutex<ThreadState>>,
    outgoing: &ThreadScopedOutgoingMessageSender,
) {
    let pending = {
        let mut state = thread_state.lock().await;
        std::mem::take(&mut state.pending_interrupts)
    };

    for request_id in pending {
        outgoing
            .send_response(request_id, TurnInterruptResponse {})
            .await;
    }
}

pub(super) async fn handle_token_count_event(
    conversation_id: ThreadId,
    turn_id: String,
    token_count_event: TokenCountEvent,
    outgoing: &ThreadScopedOutgoingMessageSender,
) {
    let TokenCountEvent { info, rate_limits } = token_count_event;
    if let Some(token_usage) = info.map(ThreadTokenUsage::from) {
        let notification = ThreadTokenUsageUpdatedNotification {
            thread_id: conversation_id.to_string(),
            turn_id,
            token_usage,
        };
        outgoing
            .send_server_notification(ServerNotification::ThreadTokenUsageUpdated(notification))
            .await;
    }
    if let Some(rate_limits) = rate_limits {
        outgoing
            .send_server_notification(ServerNotification::AccountRateLimitsUpdated(
                AccountRateLimitsUpdatedNotification {
                    rate_limits: rate_limits.into(),
                },
            ))
            .await;
    }
}

pub(super) async fn handle_error(
    _conversation_id: ThreadId,
    error: TurnError,
    thread_state: &Arc<Mutex<ThreadState>>,
) {
    let mut state = thread_state.lock().await;
    state.turn_summary.last_error = Some(error);
}

pub(super) async fn on_request_user_input_response(
    conversation_id: ThreadId,
    event_turn_id: String,
    pending_request_id: RequestId,
    receiver: oneshot::Receiver<ClientRequestResult>,
    live_thread_command: Arc<dyn AppServerLiveThreadCommandRuntime>,
    thread_state: Arc<Mutex<ThreadState>>,
    user_input_guard: ThreadWatchActiveGuard,
) {
    let response = receiver.await;
    resolve_server_request_on_thread_listener(&thread_state, pending_request_id).await;
    drop(user_input_guard);
    let value = match response {
        Ok(Ok(value)) => value,
        Ok(Err(err)) if is_turn_transition_server_request_error(&err) => return,
        Ok(Err(err)) => {
            error!("request failed with client error: {err:?}");
            let empty = CoreRequestUserInputResponse {
                answers: HashMap::new(),
            };
            if let Err(err) = live_thread_command
                .submit_live_thread_op(conversation_id, Op::UserInputAnswer {
                    id: event_turn_id,
                    response: empty,
                })
                .await
            {
                error!("failed to submit UserInputAnswer: {err}");
            }
            return;
        }
        Err(err) => {
            error!("request failed: {err:?}");
            let empty = CoreRequestUserInputResponse {
                answers: HashMap::new(),
            };
            if let Err(err) = live_thread_command
                .submit_live_thread_op(conversation_id, Op::UserInputAnswer {
                    id: event_turn_id,
                    response: empty,
                })
                .await
            {
                error!("failed to submit UserInputAnswer: {err}");
            }
            return;
        }
    };

    let response =
        serde_json::from_value::<ToolRequestUserInputResponse>(value).unwrap_or_else(|err| {
            error!("failed to deserialize ToolRequestUserInputResponse: {err}");
            ToolRequestUserInputResponse {
                answers: HashMap::new(),
            }
        });
    let response = CoreRequestUserInputResponse {
        answers: response
            .answers
            .into_iter()
            .map(|(id, answer)| {
                (
                    id,
                    CoreRequestUserInputAnswer {
                        answers: answer.answers,
                    },
                )
            })
            .collect(),
    };

    if let Err(err) = live_thread_command
        .submit_live_thread_op(conversation_id, Op::UserInputAnswer {
            id: event_turn_id,
            response,
        })
        .await
    {
        error!("failed to submit UserInputAnswer: {err}");
    }
}

pub(super) async fn on_mcp_server_elicitation_response(
    conversation_id: ThreadId,
    server_name: String,
    request_id: protocol::mcp::RequestId,
    pending_request_id: RequestId,
    receiver: oneshot::Receiver<ClientRequestResult>,
    live_thread_command: Arc<dyn AppServerLiveThreadCommandRuntime>,
    thread_state: Arc<Mutex<ThreadState>>,
    permission_guard: ThreadWatchActiveGuard,
) {
    let response = receiver.await;
    resolve_server_request_on_thread_listener(&thread_state, pending_request_id).await;
    drop(permission_guard);
    let response = mcp_server_elicitation_response_from_client_result(response);

    if let Err(err) = live_thread_command
        .submit_live_thread_op(conversation_id, Op::ResolveElicitation {
            server_name,
            request_id,
            decision: response.action.to_core(),
            content: response.content,
            meta: response.meta,
        })
        .await
    {
        error!("failed to submit ResolveElicitation: {err}");
    }
}

pub(super) fn mcp_server_elicitation_response_from_client_result(
    response: std::result::Result<ClientRequestResult, oneshot::error::RecvError>,
) -> McpServerElicitationRequestResponse {
    match response {
        Ok(Ok(value)) => serde_json::from_value::<McpServerElicitationRequestResponse>(value)
            .unwrap_or_else(|err| {
                error!("failed to deserialize McpServerElicitationRequestResponse: {err}");
                McpServerElicitationRequestResponse {
                    action: McpServerElicitationAction::Decline,
                    content: None,
                    meta: None,
                }
            }),
        Ok(Err(err)) if is_turn_transition_server_request_error(&err) => {
            McpServerElicitationRequestResponse {
                action: McpServerElicitationAction::Cancel,
                content: None,
                meta: None,
            }
        }
        Ok(Err(err)) => {
            error!("request failed with client error: {err:?}");
            McpServerElicitationRequestResponse {
                action: McpServerElicitationAction::Decline,
                content: None,
                meta: None,
            }
        }
        Err(err) => {
            error!("request failed: {err:?}");
            McpServerElicitationRequestResponse {
                action: McpServerElicitationAction::Decline,
                content: None,
                meta: None,
            }
        }
    }
}

pub(super) async fn on_request_permissions_response(
    pending_response: PendingRequestPermissionsResponse,
    live_thread_command: Arc<dyn AppServerLiveThreadCommandRuntime>,
    thread_state: Arc<Mutex<ThreadState>>,
) {
    let PendingRequestPermissionsResponse {
        conversation_id,
        call_id,
        requested_permissions,
        request_cwd,
        pending_request_id,
        outgoing,
        receiver,
        request_permissions_guard,
    } = pending_response;
    let response = receiver.await;
    resolve_server_request_on_thread_listener(&thread_state, pending_request_id.clone()).await;
    drop(request_permissions_guard);
    let Some(response) = request_permissions_response_from_client_result(
        requested_permissions,
        response,
        request_cwd.as_path(),
    ) else {
        return;
    };
    outgoing.track_effective_permissions_approval_response(pending_request_id, response.clone());

    if let Err(err) = live_thread_command
        .submit_live_thread_op(conversation_id, Op::RequestPermissionsResponse {
            id: call_id,
            response,
        })
        .await
    {
        error!("failed to submit RequestPermissionsResponse: {err}");
    }
}

pub(super) struct PendingRequestPermissionsResponse {
    pub(super) conversation_id: ThreadId,
    pub(super) call_id: String,
    pub(super) requested_permissions: CoreRequestPermissionProfile,
    pub(super) request_cwd: AbsolutePathBuf,
    pub(super) pending_request_id: RequestId,
    pub(super) outgoing: ThreadScopedOutgoingMessageSender,
    pub(super) receiver: oneshot::Receiver<ClientRequestResult>,
    pub(super) request_permissions_guard: ThreadWatchActiveGuard,
}

pub(super) fn request_permissions_response_from_client_result(
    requested_permissions: CoreRequestPermissionProfile,
    response: std::result::Result<ClientRequestResult, oneshot::error::RecvError>,
    cwd: &std::path::Path,
) -> Option<CoreRequestPermissionsResponse> {
    let value = match response {
        Ok(Ok(value)) => value,
        Ok(Err(err)) if is_turn_transition_server_request_error(&err) => return None,
        Ok(Err(err)) => {
            error!("request failed with client error: {err:?}");
            return Some(CoreRequestPermissionsResponse {
                permissions: Default::default(),
                scope: CorePermissionGrantScope::Turn,
                strict_auto_review: false,
            });
        }
        Err(err) => {
            error!("request failed: {err:?}");
            return Some(CoreRequestPermissionsResponse {
                permissions: Default::default(),
                scope: CorePermissionGrantScope::Turn,
                strict_auto_review: false,
            });
        }
    };

    let response = serde_json::from_value::<PermissionsRequestApprovalResponse>(value)
        .unwrap_or_else(|err| {
            error!("failed to deserialize PermissionsRequestApprovalResponse: {err}");
            PermissionsRequestApprovalResponse {
                permissions: V2GrantedPermissionProfile::default(),
                scope: app_server_protocol::PermissionGrantScope::Turn,
                strict_auto_review: None,
            }
        });
    let strict_auto_review = response.strict_auto_review.unwrap_or(false);
    if strict_auto_review
        && matches!(
            response.scope,
            app_server_protocol::PermissionGrantScope::Session
        )
    {
        error!("strict auto review is only supported for turn-scoped permission grants");
        return Some(CoreRequestPermissionsResponse {
            permissions: Default::default(),
            scope: CorePermissionGrantScope::Turn,
            strict_auto_review: false,
        });
    }
    let granted_permissions: CoreAdditionalPermissionProfile = response.permissions.into();
    let permissions = if granted_permissions.is_empty() {
        CoreRequestPermissionProfile::default()
    } else {
        intersect_permission_profiles(requested_permissions.into(), granted_permissions, cwd).into()
    };
    Some(CoreRequestPermissionsResponse {
        permissions,
        scope: response.scope.to_core(),
        strict_auto_review,
    })
}

pub(super) const REVIEW_FALLBACK_MESSAGE: &str = "Reviewer failed to output a response.";

pub(super) fn render_review_output_text(output: &ReviewOutputEvent) -> String {
    let mut sections = Vec::new();
    let explanation = output.overall_explanation.trim();
    if !explanation.is_empty() {
        sections.push(explanation.to_string());
    }
    if !output.findings.is_empty() {
        let findings = format_review_findings_block(&output.findings, /*selection*/ None);
        let trimmed = findings.trim();
        if !trimmed.is_empty() {
            sections.push(trimmed.to_string());
        }
    }
    if sections.is_empty() {
        REVIEW_FALLBACK_MESSAGE.to_string()
    } else {
        sections.join("\n\n")
    }
}

pub(super) fn map_file_change_approval_decision(
    decision: FileChangeApprovalDecision,
) -> ReviewDecision {
    match decision {
        FileChangeApprovalDecision::Accept => ReviewDecision::Approved,
        FileChangeApprovalDecision::AcceptForSession => ReviewDecision::ApprovedForSession,
        FileChangeApprovalDecision::Decline => ReviewDecision::Denied,
        FileChangeApprovalDecision::Cancel => ReviewDecision::Abort,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn on_file_change_request_approval_response(
    conversation_id: ThreadId,
    item_id: String,
    pending_request_id: RequestId,
    receiver: oneshot::Receiver<ClientRequestResult>,
    live_thread_command: Arc<dyn AppServerLiveThreadCommandRuntime>,
    thread_state: Arc<Mutex<ThreadState>>,
    permission_guard: ThreadWatchActiveGuard,
) {
    let response = receiver.await;
    resolve_server_request_on_thread_listener(&thread_state, pending_request_id).await;
    drop(permission_guard);
    let decision = match response {
        Ok(Ok(value)) => {
            let response = serde_json::from_value::<FileChangeRequestApprovalResponse>(value)
                .unwrap_or_else(|err| {
                    error!("failed to deserialize FileChangeRequestApprovalResponse: {err}");
                    FileChangeRequestApprovalResponse {
                        decision: FileChangeApprovalDecision::Decline,
                    }
                });

            map_file_change_approval_decision(response.decision)
        }
        Ok(Err(err)) if is_turn_transition_server_request_error(&err) => return,
        Ok(Err(err)) => {
            error!("request failed with client error: {err:?}");
            ReviewDecision::Denied
        }
        Err(err) => {
            error!("request failed: {err:?}");
            ReviewDecision::Denied
        }
    };

    if let Err(err) = live_thread_command
        .submit_live_thread_op(conversation_id, Op::PatchApproval {
            id: item_id,
            decision,
        })
        .await
    {
        error!("failed to submit PatchApproval: {err}");
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn on_command_execution_request_approval_response(
    event_turn_id: String,
    conversation_id: ThreadId,
    approval_id: Option<String>,
    item_id: String,
    completion_item: Option<CommandExecutionCompletionItem>,
    pending_request_id: RequestId,
    receiver: oneshot::Receiver<ClientRequestResult>,
    live_thread_command: Arc<dyn AppServerLiveThreadCommandRuntime>,
    outgoing: ThreadScopedOutgoingMessageSender,
    thread_state: Arc<Mutex<ThreadState>>,
    permission_guard: ThreadWatchActiveGuard,
) {
    let response = receiver.await;
    resolve_server_request_on_thread_listener(&thread_state, pending_request_id).await;
    drop(permission_guard);
    let (decision, completion_status) = match response {
        Ok(Ok(value)) => {
            let response = serde_json::from_value::<CommandExecutionRequestApprovalResponse>(value)
                .unwrap_or_else(|err| {
                    error!("failed to deserialize CommandExecutionRequestApprovalResponse: {err}");
                    CommandExecutionRequestApprovalResponse {
                        decision: CommandExecutionApprovalDecision::Decline,
                    }
                });

            let decision = response.decision;

            let (decision, completion_status) = match decision {
                CommandExecutionApprovalDecision::Accept => (ReviewDecision::Approved, None),
                CommandExecutionApprovalDecision::AcceptForSession => {
                    (ReviewDecision::ApprovedForSession, None)
                }
                CommandExecutionApprovalDecision::AcceptWithExecpolicyAmendment {
                    execpolicy_amendment,
                } => (
                    ReviewDecision::ApprovedExecpolicyAmendment {
                        proposed_execpolicy_amendment: execpolicy_amendment.into_core(),
                    },
                    None,
                ),
                CommandExecutionApprovalDecision::ApplyNetworkPolicyAmendment {
                    network_policy_amendment,
                } => {
                    let completion_status = match network_policy_amendment.action {
                        V2NetworkPolicyRuleAction::Allow => None,
                        V2NetworkPolicyRuleAction::Deny => Some(CommandExecutionStatus::Declined),
                    };
                    (
                        ReviewDecision::NetworkPolicyAmendment {
                            network_policy_amendment: network_policy_amendment.into_core(),
                        },
                        completion_status,
                    )
                }
                CommandExecutionApprovalDecision::Decline => (
                    ReviewDecision::Denied,
                    Some(CommandExecutionStatus::Declined),
                ),
                CommandExecutionApprovalDecision::Cancel => (
                    ReviewDecision::Abort,
                    Some(CommandExecutionStatus::Declined),
                ),
            };
            (decision, completion_status)
        }
        Ok(Err(err)) if is_turn_transition_server_request_error(&err) => return,
        Ok(Err(err)) => {
            error!("request failed with client error: {err:?}");
            (ReviewDecision::Denied, Some(CommandExecutionStatus::Failed))
        }
        Err(err) => {
            error!("request failed: {err:?}");
            (ReviewDecision::Denied, Some(CommandExecutionStatus::Failed))
        }
    };

    let suppress_subcommand_completion_item = {
        // For regular shell/unified_exec approvals, approval_id is null.
        // For zsh-fork subcommand approvals, approval_id is present and
        // item_id points to the parent command item.
        if approval_id.is_some() {
            let state = thread_state.lock().await;
            state
                .turn_summary
                .command_execution_started
                .contains(&item_id)
        } else {
            false
        }
    };

    if let Some(status) = completion_status
        && !suppress_subcommand_completion_item
        && let Some(completion_item) = completion_item
    {
        complete_command_execution_item(
            &conversation_id,
            event_turn_id.clone(),
            item_id.clone(),
            completion_item.command,
            completion_item.cwd,
            /*process_id*/ None,
            CommandExecutionSource::Agent,
            completion_item.command_actions,
            status,
            &outgoing,
            &thread_state,
        )
        .await;
    }

    if let Err(err) = live_thread_command
        .submit_live_thread_op(conversation_id, Op::ExecApproval {
            id: approval_id.unwrap_or_else(|| item_id.clone()),
            turn_id: Some(event_turn_id),
            decision,
        })
        .await
    {
        error!("failed to submit ExecApproval: {err}");
    }
}

pub(super) fn now_unix_timestamp_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}
