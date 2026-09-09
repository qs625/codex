use super::*;

enum CommandExecutionApprovalPresentation {
    Network(V2NetworkApprovalContext),
    Command(CommandExecutionCompletionItem),
}

#[derive(Debug, PartialEq)]
pub(super) struct CommandExecutionCompletionItem {
    pub(super) command: String,
    pub(super) cwd: AbsolutePathBuf,
    pub(super) command_actions: Vec<V2ParsedCommand>,
}

pub(super) async fn handle_apply_patch_approval_request(
    conversation_id: &ThreadId,
    live_thread_command: Arc<dyn AppServerLiveThreadCommandRuntime>,
    outgoing: &ThreadScopedOutgoingMessageSender,
    thread_state: Arc<Mutex<ThreadState>>,
    thread_watch_manager: &ThreadWatchManager,
    event: protocol::protocol::ApplyPatchApprovalRequestEvent,
) {
    let conversation_id = *conversation_id;
    let permission_guard = thread_watch_manager
        .note_permission_requested(&conversation_id.to_string())
        .await;
    let item_id = event.call_id.clone();

    let params = FileChangeRequestApprovalParams {
        thread_id: conversation_id.to_string(),
        turn_id: event.turn_id.clone(),
        item_id: item_id.clone(),
        started_at_ms: event.started_at_ms,
        reason: event.reason.clone(),
        grant_root: event.grant_root.clone(),
    };
    let (pending_request_id, rx) = outgoing
        .send_request(ServerRequestPayload::FileChangeRequestApproval(params))
        .await;
    tokio::spawn(async move {
        on_file_change_request_approval_response(
            conversation_id,
            item_id,
            pending_request_id,
            rx,
            live_thread_command,
            thread_state.clone(),
            permission_guard,
        )
        .await;
    });
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_exec_approval_request(
    conversation_id: ThreadId,
    event_turn_id: String,
    live_thread_command: Arc<dyn AppServerLiveThreadCommandRuntime>,
    outgoing: ThreadScopedOutgoingMessageSender,
    thread_state: Arc<Mutex<ThreadState>>,
    thread_watch_manager: &ThreadWatchManager,
    ev: ExecApprovalRequestEvent,
) {
    let permission_guard = thread_watch_manager
        .note_permission_requested(&conversation_id.to_string())
        .await;
    let available_decisions = ev
        .effective_available_decisions()
        .into_iter()
        .map(CommandExecutionApprovalDecision::from)
        .collect::<Vec<_>>();
    let ExecApprovalRequestEvent {
        call_id,
        approval_id,
        turn_id,
        started_at_ms,
        command,
        cwd,
        reason,
        network_approval_context,
        proposed_execpolicy_amendment,
        proposed_network_policy_amendments,
        additional_permissions,
        parsed_cmd,
        ..
    } = ev;
    let command_actions = parsed_cmd
        .iter()
        .cloned()
        .map(|parsed| V2ParsedCommand::from_core_with_cwd(parsed, &cwd))
        .collect::<Vec<_>>();
    let presentation =
        if let Some(network_approval_context) = network_approval_context.map(Into::into) {
            CommandExecutionApprovalPresentation::Network(network_approval_context)
        } else {
            let command_string = shlex_join(&command);
            let completion_item = CommandExecutionCompletionItem {
                command: command_string,
                cwd: cwd.clone(),
                command_actions: command_actions.clone(),
            };
            CommandExecutionApprovalPresentation::Command(completion_item)
        };
    let (network_approval_context, command, cwd, command_actions, completion_item) =
        match presentation {
            CommandExecutionApprovalPresentation::Network(network_approval_context) => {
                (Some(network_approval_context), None, None, None, None)
            }
            CommandExecutionApprovalPresentation::Command(completion_item) => (
                None,
                Some(completion_item.command.clone()),
                Some(completion_item.cwd.clone()),
                Some(completion_item.command_actions.clone()),
                Some(completion_item),
            ),
        };
    if approval_id.is_none()
        && let Some(completion_item) = completion_item.as_ref()
    {
        start_command_execution_item(
            &conversation_id,
            event_turn_id.clone(),
            call_id.clone(),
            completion_item.command.clone(),
            completion_item.cwd.clone(),
            completion_item.command_actions.clone(),
            CommandExecutionSource::Agent,
            &outgoing,
            &thread_state,
        )
        .await;
    }
    let proposed_execpolicy_amendment_v2 = proposed_execpolicy_amendment.map(V2ExecPolicyAmendment::from);
    let proposed_network_policy_amendments_v2 = proposed_network_policy_amendments.map(|amendments| {
        amendments
            .into_iter()
            .map(V2NetworkPolicyAmendment::from)
            .collect()
    });
    let additional_permissions = additional_permissions.map(V2AdditionalPermissionProfile::from);

    let params = CommandExecutionRequestApprovalParams {
        thread_id: conversation_id.to_string(),
        turn_id: turn_id.clone(),
        item_id: call_id.clone(),
        started_at_ms,
        approval_id: approval_id.clone(),
        reason,
        network_approval_context,
        command,
        cwd,
        command_actions,
        additional_permissions,
        proposed_execpolicy_amendment: proposed_execpolicy_amendment_v2,
        proposed_network_policy_amendments: proposed_network_policy_amendments_v2,
        available_decisions: Some(available_decisions),
    };
    let (pending_request_id, rx) = outgoing
        .send_request(ServerRequestPayload::CommandExecutionRequestApproval(
            params,
        ))
        .await;
    tokio::spawn(async move {
        on_command_execution_request_approval_response(
            event_turn_id,
            conversation_id,
            approval_id,
            call_id,
            completion_item,
            pending_request_id,
            rx,
            live_thread_command,
            outgoing,
            thread_state.clone(),
            permission_guard,
        )
        .await;
    });
}

pub(super) async fn handle_request_user_input(
    conversation_id: &ThreadId,
    event_turn_id: String,
    live_thread_command: Arc<dyn AppServerLiveThreadCommandRuntime>,
    outgoing: &ThreadScopedOutgoingMessageSender,
    thread_state: Arc<Mutex<ThreadState>>,
    thread_watch_manager: &ThreadWatchManager,
    request: protocol::protocol::RequestUserInputEvent,
) {
    let conversation_id = *conversation_id;
    let user_input_guard = thread_watch_manager
        .note_user_input_requested(&conversation_id.to_string())
        .await;
    let questions = request
        .questions
        .into_iter()
        .map(|question| ToolRequestUserInputQuestion {
            id: question.id,
            header: question.header,
            question: question.question,
            is_other: question.is_other,
            is_secret: question.is_secret,
            options: question.options.map(|options| {
                options
                    .into_iter()
                    .map(|option| ToolRequestUserInputOption {
                        label: option.label,
                        description: option.description,
                    })
                    .collect()
            }),
        })
        .collect();
    let params = ToolRequestUserInputParams {
        thread_id: conversation_id.to_string(),
        turn_id: request.turn_id,
        item_id: request.call_id,
        questions,
    };
    let (pending_request_id, rx) = outgoing
        .send_request(ServerRequestPayload::ToolRequestUserInput(params))
        .await;
    tokio::spawn(async move {
        on_request_user_input_response(
            conversation_id,
            event_turn_id,
            pending_request_id,
            rx,
            live_thread_command,
            thread_state,
            user_input_guard,
        )
        .await;
    });
}

pub(super) async fn handle_elicitation_request(
    conversation_id: &ThreadId,
    live_thread_command: Arc<dyn AppServerLiveThreadCommandRuntime>,
    outgoing: &ThreadScopedOutgoingMessageSender,
    thread_state: Arc<Mutex<ThreadState>>,
    thread_watch_manager: &ThreadWatchManager,
    request: protocol::approvals::ElicitationRequestEvent,
) {
    let conversation_id = *conversation_id;
    let permission_guard = thread_watch_manager
        .note_permission_requested(&conversation_id.to_string())
        .await;
    let turn_id = match request.turn_id.clone() {
        Some(turn_id) => Some(turn_id),
        None => {
            let state = thread_state.lock().await;
            state.active_turn_snapshot().map(|turn| turn.id)
        }
    };
    let server_name = request.server_name.clone();
    let request_body = match request.request.try_into() {
        Ok(request_body) => request_body,
        Err(err) => {
            error!(
                error = %err,
                server_name,
                request_id = ?request.id,
                "failed to parse typed MCP elicitation schema"
            );
            if let Err(err) = live_thread_command
                .submit_live_thread_op(conversation_id, Op::ResolveElicitation {
                    server_name: request.server_name,
                    request_id: request.id,
                    decision: protocol::approvals::ElicitationAction::Cancel,
                    content: None,
                    meta: None,
                })
                .await
            {
                error!("failed to submit ResolveElicitation: {err}");
            }
            return;
        }
    };
    let params = McpServerElicitationRequestParams {
        thread_id: conversation_id.to_string(),
        turn_id,
        server_name: request.server_name.clone(),
        request: request_body,
    };
    let (pending_request_id, rx) = outgoing
        .send_request(ServerRequestPayload::McpServerElicitationRequest(params))
        .await;
    tokio::spawn(async move {
        on_mcp_server_elicitation_response(
            conversation_id,
            request.server_name,
            request.id,
            pending_request_id,
            rx,
            live_thread_command,
            thread_state,
            permission_guard,
        )
        .await;
    });
}

pub(super) async fn handle_request_permissions(
    conversation_id: &ThreadId,
    live_thread_inspection: Arc<dyn AppServerLiveThreadInspectionRuntime>,
    live_thread_command: Arc<dyn AppServerLiveThreadCommandRuntime>,
    outgoing: ThreadScopedOutgoingMessageSender,
    thread_state: Arc<Mutex<ThreadState>>,
    thread_watch_manager: &ThreadWatchManager,
    request: protocol::request_permissions::RequestPermissionsEvent,
) {
    let conversation_id = *conversation_id;
    let permission_guard = thread_watch_manager
        .note_permission_requested(&conversation_id.to_string())
        .await;
    let requested_permissions = request.permissions.clone();
    let request_cwd = match request.cwd.clone() {
        Some(cwd) => cwd,
        None => match live_thread_inspection
            .live_thread_config_snapshot(conversation_id)
            .await
        {
            Ok(snapshot) => snapshot.cwd,
            Err(err) => {
                error!(
                    "failed to read live thread config snapshot for request permissions {conversation_id}: {err}"
                );
                drop(permission_guard);
                if let Err(err) = live_thread_command
                    .submit_live_thread_op(conversation_id, Op::RequestPermissionsResponse {
                        id: request.call_id,
                        response: CoreRequestPermissionsResponse {
                            permissions: Default::default(),
                            scope: CorePermissionGrantScope::Turn,
                            strict_auto_review: false,
                        },
                    })
                    .await
                {
                    error!("failed to submit RequestPermissionsResponse: {err}");
                }
                return;
            }
        },
    };
    let params = PermissionsRequestApprovalParams {
        thread_id: conversation_id.to_string(),
        turn_id: request.turn_id.clone(),
        item_id: request.call_id.clone(),
        started_at_ms: request.started_at_ms,
        cwd: request_cwd.clone(),
        reason: request.reason,
        permissions: request.permissions.into(),
    };
    let (pending_request_id, rx) = outgoing
        .send_request(ServerRequestPayload::PermissionsRequestApproval(params))
        .await;
    let pending_response = PendingRequestPermissionsResponse {
        conversation_id,
        call_id: request.call_id,
        requested_permissions,
        request_cwd,
        pending_request_id,
        outgoing,
        receiver: rx,
        request_permissions_guard: permission_guard,
    };
    tokio::spawn(async move {
        on_request_permissions_response(pending_response, live_thread_command, thread_state).await;
    });
}

pub(super) async fn handle_dynamic_tool_call_request(
    conversation_id: &ThreadId,
    live_thread_command: Arc<dyn AppServerLiveThreadCommandRuntime>,
    outgoing: &ThreadScopedOutgoingMessageSender,
    request: protocol::dynamic_tools::DynamicToolCallRequest,
) {
    let conversation_id = *conversation_id;
    let call_id = request.call_id;
    let turn_id = request.turn_id;
    let namespace = request.namespace;
    let tool = request.tool;
    let arguments = request.arguments;
    let item = ThreadItem::DynamicToolCall {
        id: call_id.clone(),
        namespace: namespace.clone(),
        tool: tool.clone(),
        arguments: arguments.clone(),
        status: DynamicToolCallStatus::InProgress,
        content_items: None,
        success: None,
        duration_ms: None,
    };
    let notification = ItemStartedNotification {
        thread_id: conversation_id.to_string(),
        turn_id: turn_id.clone(),
        started_at_ms: request.started_at_ms,
        item,
    };
    outgoing
        .send_server_notification(ServerNotification::ItemStarted(notification))
        .await;
    let params = DynamicToolCallParams {
        thread_id: conversation_id.to_string(),
        turn_id: turn_id.clone(),
        call_id: call_id.clone(),
        namespace,
        tool: tool.clone(),
        arguments: arguments.clone(),
    };
    let (_pending_request_id, rx) = outgoing
        .send_request(ServerRequestPayload::DynamicToolCall(params))
        .await;
    tokio::spawn(async move {
        crate::dynamic_tools::on_call_response(
            conversation_id,
            call_id,
            rx,
            live_thread_command,
        )
        .await;
    });
}
