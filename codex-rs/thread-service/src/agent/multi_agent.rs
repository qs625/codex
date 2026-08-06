//! Thread-owned multi-agent operation helpers.

use crate::agent::SpawnAgentOptions;
use crate::agent::agent_resolver::resolve_agent_target;
use crate::agent::exceeds_thread_spawn_depth_limit;
use crate::agent::external::provider_is_external;
use crate::agent::role::apply_role_to_config;
use crate::agent::spawn_support::*;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use codex_agent_roles::DEFAULT_ROLE_NAME;
use codex_agent_runtime::CloseAgentToolResult;
use codex_agent_runtime::ListAgentsToolResult;
use codex_agent_runtime::ReadAgentToolResult;
use codex_agent_runtime::SpawnAgentToolRequest;
use codex_agent_runtime::SpawnAgentToolResult;
use codex_agent_runtime::SpawnExternalAgentToolRequest;
use codex_agent_runtime::render_input_preview;
use protocol::AgentPath;
use protocol::ThreadId;
use protocol::models::ContentItem;
use protocol::models::ResponseItem;
use protocol::models::image_attachment_id_from_open_tag_text;
use protocol::protocol::CollabAgentInteractionBeginEvent;
use protocol::protocol::CollabAgentInteractionEndEvent;
use protocol::protocol::CollabAgentSpawnBeginEvent;
use protocol::protocol::CollabAgentSpawnEndEvent;
use protocol::protocol::CollabCloseBeginEvent;
use protocol::protocol::CollabCloseEndEvent;
use protocol::protocol::CollabListAgentsBeginEvent;
use protocol::protocol::CollabListAgentsEndEvent;
use protocol::protocol::CollabListedAgent;
use protocol::protocol::InterAgentCommunication;
use protocol::protocol::InterAgentContentPart;
use protocol::protocol::InterAgentOperation;
use protocol::protocol::Op;
use protocol::protocol::SessionSource;
use protocol::protocol::SubAgentSource;
use std::sync::Arc;
use tool_service_api::FunctionCallError;

pub(crate) async fn spawn_agent_tool(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    call_id: String,
    request: SpawnAgentToolRequest,
) -> Result<SpawnAgentToolResult, FunctionCallError> {
    let prompt = message_content(request.message.clone())?;
    session
        .send_event(
            turn.as_ref(),
            CollabAgentSpawnBeginEvent {
                call_id: call_id.clone(),
                started_at_ms: crate::turn_timing::now_unix_timestamp_ms(),
                sender_thread_id: session.thread_id(),
                sender_agent_path: session
                    .current_agent_path_for_turn(turn.as_ref())
                    .to_string(),
                prompt,
                model: request.model.clone().unwrap_or_default(),
                reasoning_effort: request.reasoning_effort.unwrap_or_default(),
            }
            .into(),
        )
        .await;
    handle_spawn_agent_request(session, turn, call_id, request).await
}

pub(crate) async fn spawn_external_agent_tool(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    call_id: String,
    request: SpawnExternalAgentToolRequest,
) -> Result<SpawnAgentToolResult, FunctionCallError> {
    if matches!(
        request.provider,
        codex_agent_runtime::SpawnAgentProvider::Native
    ) {
        return Err(FunctionCallError::RespondToModel(
            "spawn_external_agent requires an external provider".to_string(),
        ));
    }
    let spawn_request = SpawnAgentToolRequest {
        message: request.message,
        task_name: request.task_name,
        provider: Some(request.provider),
        agent_type: None,
        cwd: Some(request.cwd),
        model: None,
        reasoning_effort: None,
        service_tier: None,
        fork_mode: None,
    };
    spawn_external_agent_request(session, turn, call_id, spawn_request).await
}

pub(crate) async fn followup_external_task_tool(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    call_id: String,
    target: String,
    message: String,
    content_parts: Vec<InterAgentContentPart>,
) -> Result<(), FunctionCallError> {
    followup_task_tool(session, turn, call_id, target, message, content_parts).await
}

pub(crate) async fn close_external_agent_tool(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    call_id: String,
    target: String,
) -> Result<CloseAgentToolResult, FunctionCallError> {
    close_agent_tool(session, turn, call_id, target).await
}

pub(crate) async fn list_external_agents_tool(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    call_id: String,
    path_prefix: Option<String>,
) -> Result<ListAgentsToolResult, FunctionCallError> {
    list_agents_tool(session, turn, call_id, path_prefix).await
}

pub(crate) async fn read_external_agent_tool(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    call_id: String,
    target: String,
) -> Result<ReadAgentToolResult, FunctionCallError> {
    read_agent_tool(session, turn, call_id, target).await
}

pub(crate) async fn followup_task_tool(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    call_id: String,
    target: String,
    message: String,
    content_parts: Vec<InterAgentContentPart>,
) -> Result<(), FunctionCallError> {
    let visible_attachments =
        visible_image_attachments_from_history(session.clone_history().await.raw_items());
    let (prompt, content_parts) =
        followup_message_content(message, content_parts, &visible_attachments)?;
    let sender_thread_id = session.thread_id();
    let sender_agent_path = session.current_agent_path_for_turn(turn.as_ref());
    let receiver_thread_id = resolve_agent_target(&session, &turn, &target).await?;
    let receiver_agent = session.agent_metadata(receiver_thread_id);
    reject_root_agent(
        receiver_agent.agent_path.as_ref(),
        "Tasks can't be assigned to the root agent",
    )?;
    let receiver_agent_path = receiver_agent.agent_path.clone().ok_or_else(|| {
        FunctionCallError::RespondToModel("target agent is missing an agent_path".to_string())
    })?;
    let receiver_agent_path_string = receiver_agent_path.to_string();
    session
        .send_event(
            turn.as_ref(),
            CollabAgentInteractionBeginEvent {
                call_id: call_id.clone(),
                started_at_ms: crate::turn_timing::now_unix_timestamp_ms(),
                sender_thread_id,
                sender_agent_path: sender_agent_path.to_string(),
                receiver_thread_id,
                receiver_agent_path: receiver_agent_path_string.clone(),
                prompt: prompt.clone(),
            }
            .into(),
        )
        .await;
    let communication = InterAgentCommunication::new(
        sender_agent_path.clone(),
        receiver_agent_path,
        Vec::new(),
        prompt.clone(),
        InterAgentOperation::FollowupTask,
    )
    .with_content_parts(content_parts)
    .with_thread_ids(sender_thread_id, receiver_thread_id);
    let result = session
        .send_inter_agent_communication(receiver_thread_id, communication.with_trigger_turn(true))
        .await
        .map_err(|err| collab_agent_error(receiver_thread_id, err));
    let status = session.agent_status(receiver_thread_id).await;
    session
        .send_event(
            turn.as_ref(),
            CollabAgentInteractionEndEvent {
                call_id,
                completed_at_ms: crate::turn_timing::now_unix_timestamp_ms(),
                sender_thread_id,
                sender_agent_path: sender_agent_path.to_string(),
                receiver_thread_id,
                receiver_agent_path: receiver_agent_path_string,
                receiver_agent_nickname: receiver_agent.agent_nickname,
                receiver_agent_role: receiver_agent.agent_role,
                prompt,
                status,
            }
            .into(),
        )
        .await;
    result?;
    Ok(())
}

pub(crate) async fn close_agent_tool(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    call_id: String,
    target: String,
) -> Result<CloseAgentToolResult, FunctionCallError> {
    let sender_thread_id = session.thread_id();
    let sender_agent_path = session.current_agent_path_for_turn(turn.as_ref());
    let agent_id = resolve_agent_target(&session, &turn, &target).await?;
    let receiver_agent = session.agent_metadata(agent_id);
    reject_root_agent(
        receiver_agent.agent_path.as_ref(),
        "root is not a spawned agent",
    )?;
    let receiver_agent_path = receiver_agent.agent_path.clone().ok_or_else(|| {
        FunctionCallError::RespondToModel("target agent is missing an agent_path".to_string())
    })?;
    session
        .send_event(
            turn.as_ref(),
            CollabCloseBeginEvent {
                call_id: call_id.clone(),
                started_at_ms: crate::turn_timing::now_unix_timestamp_ms(),
                sender_thread_id,
                sender_agent_path: sender_agent_path.to_string(),
                receiver_thread_id: agent_id,
                receiver_agent_path: receiver_agent_path.to_string(),
            }
            .into(),
        )
        .await;
    let status = session.agent_status(agent_id).await;
    let result = session
        .close_agent(agent_id)
        .await
        .map_err(|err| collab_agent_error(agent_id, err));
    session
        .send_event(
            turn.as_ref(),
            CollabCloseEndEvent {
                call_id,
                completed_at_ms: crate::turn_timing::now_unix_timestamp_ms(),
                sender_thread_id,
                sender_agent_path: sender_agent_path.to_string(),
                receiver_thread_id: agent_id,
                receiver_agent_path: receiver_agent_path.to_string(),
                receiver_agent_nickname: receiver_agent.agent_nickname,
                receiver_agent_role: receiver_agent.agent_role,
                status: status.clone(),
            }
            .into(),
        )
        .await;
    result?;

    Ok(CloseAgentToolResult {
        previous_status: status,
    })
}

pub(crate) async fn list_agents_tool(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    call_id: String,
    path_prefix: Option<String>,
) -> Result<ListAgentsToolResult, FunctionCallError> {
    let sender_thread_id = session.thread_id();
    let sender_agent_path = session
        .current_agent_path_for_turn(turn.as_ref())
        .to_string();
    session
        .send_event(
            turn.as_ref(),
            CollabListAgentsBeginEvent {
                call_id: call_id.clone(),
                started_at_ms: crate::turn_timing::now_unix_timestamp_ms(),
                sender_thread_id,
                sender_agent_path: sender_agent_path.clone(),
                path_prefix: path_prefix.clone(),
            }
            .into(),
        )
        .await;
    session.register_session_root_for_turn(turn.as_ref());
    let agents = session
        .list_agents_for_turn(turn.as_ref(), path_prefix.as_deref())
        .await
        .map_err(collab_spawn_error);
    let listed_agents = agents.as_ref().map_or_else(
        |_| Vec::new(),
        |agents| {
            agents
                .iter()
                .map(|agent| CollabListedAgent {
                    agent_path: agent.agent_name.clone(),
                    agent_nickname: agent.agent_nickname.clone(),
                    agent_role: agent.agent_role.clone(),
                    lifecycle_status: agent.lifecycle_status.clone(),
                })
                .collect()
        },
    );
    session
        .send_event(
            turn.as_ref(),
            CollabListAgentsEndEvent {
                call_id,
                completed_at_ms: crate::turn_timing::now_unix_timestamp_ms(),
                sender_thread_id,
                sender_agent_path,
                path_prefix,
                success: agents.is_ok(),
                agents: listed_agents,
            }
            .into(),
        )
        .await;

    Ok(ListAgentsToolResult { agents: agents? })
}

pub(crate) async fn read_agent_tool(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    _call_id: String,
    target: String,
) -> Result<ReadAgentToolResult, FunctionCallError> {
    session.register_session_root_for_turn(turn.as_ref());
    let agent_id = if let Ok(thread_id) = ThreadId::from_string(&target) {
        thread_id
    } else {
        session
            .resolve_agent_reference_for_read(turn.as_ref(), &target)
            .await
            .map_err(|err| match err {
                protocol::error::CodexErr::UnsupportedOperation(message) => {
                    FunctionCallError::RespondToModel(message)
                }
                other => FunctionCallError::RespondToModel(other.to_string()),
            })?
    };
    let agent = session
        .read_agent_for_turn(turn.as_ref(), agent_id)
        .await
        .map_err(collab_spawn_error)?;

    Ok(ReadAgentToolResult { agent })
}

fn reject_root_agent(
    agent_path: Option<&AgentPath>,
    message: &str,
) -> Result<(), FunctionCallError> {
    if agent_path.is_some_and(AgentPath::is_root) {
        return Err(FunctionCallError::RespondToModel(message.to_string()));
    }
    Ok(())
}

fn message_content(message: String) -> Result<String, FunctionCallError> {
    if message.trim().is_empty() {
        return Err(FunctionCallError::RespondToModel(
            "Empty message can't be sent to an agent".to_string(),
        ));
    }
    Ok(message)
}

const IMAGE_REF_TEXT_MISUSE_ERROR: &str = "Image references in followup_task must use structured content parts. Use \
     `content: [{\"type\":\"image_ref\",\"attachment_id\":\"image-1\"}]` instead of writing \
     image placeholders in `message` or text parts.";

pub(crate) fn validate_no_text_image_ref_misuse(text: &str) -> Result<(), FunctionCallError> {
    if looks_like_image_ref_text_misuse(text) {
        return Err(FunctionCallError::RespondToModel(
            IMAGE_REF_TEXT_MISUSE_ERROR.to_string(),
        ));
    }
    Ok(())
}

fn looks_like_image_ref_text_misuse(text: &str) -> bool {
    contains_bracket_image_ref(text) || contains_image_attachment_tag(text)
}

fn contains_bracket_image_ref(text: &str) -> bool {
    let mut remaining = text;
    while let Some(start) = remaining.find("[image:") {
        let after_start = &remaining[start + "[image:".len()..];
        let Some(end) = after_start.find(']') else {
            return false;
        };
        if is_model_visible_image_attachment_id(&after_start[..end]) {
            return true;
        }
        remaining = &after_start[end + 1..];
    }
    false
}

fn contains_image_attachment_tag(text: &str) -> bool {
    let mut remaining = text;
    while let Some(start) = remaining.find("<image") {
        let after_start = &remaining[start..];
        let Some(end) = after_start.find('>') else {
            return false;
        };
        let tag = &after_start[..=end];
        if let Some((_, after_key)) = tag.split_once("attachment_id=") {
            let attachment_id = after_key
                .trim_start()
                .trim_start_matches('"')
                .trim_start_matches('\'')
                .split(|ch: char| ch.is_whitespace() || ch == '>' || ch == '"' || ch == '\'')
                .next()
                .unwrap_or_default();
            if is_model_visible_image_attachment_id(attachment_id) {
                return true;
            }
        }
        remaining = &after_start[end + 1..];
    }
    false
}

fn is_model_visible_image_attachment_id(value: &str) -> bool {
    let Some(label) = value.trim().strip_prefix("image-") else {
        return false;
    };
    !label.is_empty() && label.bytes().all(|byte| byte.is_ascii_digit())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VisibleImageAttachment {
    attachment_id: String,
    image_url: String,
}

fn visible_image_attachments_from_history(items: &[ResponseItem]) -> Vec<VisibleImageAttachment> {
    let mut attachments = Vec::new();
    for item in items {
        let ResponseItem::Message { content, .. } = item else {
            continue;
        };
        for (index, content_item) in content.iter().enumerate() {
            let ContentItem::InputImage { image_url, .. } = content_item else {
                continue;
            };
            let Some(ContentItem::InputText { text }) = index
                .checked_sub(1)
                .and_then(|previous| content.get(previous))
            else {
                continue;
            };
            let Some(attachment_id) = image_attachment_id_from_open_tag_text(text) else {
                continue;
            };
            attachments.push(VisibleImageAttachment {
                attachment_id: attachment_id.to_string(),
                image_url: image_url.clone(),
            });
        }
    }
    attachments
}

fn followup_message_content(
    message: String,
    content_parts: Vec<InterAgentContentPart>,
    visible_attachments: &[VisibleImageAttachment],
) -> Result<(String, Vec<InterAgentContentPart>), FunctionCallError> {
    validate_no_text_image_ref_misuse(&message)?;
    if content_parts.is_empty() {
        return message_content(message).map(|message| (message, Vec::new()));
    }

    let mut preview_parts = Vec::new();
    let mut resolved_content_parts = Vec::with_capacity(content_parts.len());
    for part in content_parts {
        match part {
            InterAgentContentPart::Text { text } => {
                let text = text.trim();
                if !text.is_empty() {
                    validate_no_text_image_ref_misuse(text)?;
                    preview_parts.push(text.to_string());
                    resolved_content_parts.push(InterAgentContentPart::Text {
                        text: text.to_string(),
                    });
                }
            }
            InterAgentContentPart::ImageRef {
                attachment_id,
                image_url: _,
            } => {
                let attachment_id = attachment_id.trim();
                if attachment_id.is_empty() {
                    return Err(FunctionCallError::RespondToModel(
                        "image_ref content requires a non-empty attachment_id".to_string(),
                    ));
                }
                let image_url = resolve_visible_image_ref(attachment_id, visible_attachments)?;
                preview_parts.push(format!("[image:{attachment_id}]"));
                resolved_content_parts.push(InterAgentContentPart::ImageRef {
                    attachment_id: attachment_id.to_string(),
                    image_url: Some(image_url.clone()),
                });
            }
        }
    }

    if preview_parts.is_empty() {
        return Err(FunctionCallError::RespondToModel(
            "Empty content can't be sent to an agent".to_string(),
        ));
    }
    Ok((preview_parts.join("\n"), resolved_content_parts))
}

fn resolve_visible_image_ref(
    attachment_id: &str,
    visible_attachments: &[VisibleImageAttachment],
) -> Result<String, FunctionCallError> {
    let mut matches = visible_attachments
        .iter()
        .filter(|attachment| attachment.attachment_id == attachment_id)
        .map(|attachment| attachment.image_url.clone());
    let Some(image_url) = matches.next() else {
        return Err(FunctionCallError::RespondToModel(format!(
            "image_ref `{attachment_id}` is not visible in the parent thread"
        )));
    };
    if matches.next().is_some() {
        return Err(FunctionCallError::RespondToModel(format!(
            "image_ref `{attachment_id}` is ambiguous in the parent thread"
        )));
    }
    Ok(image_url)
}

async fn handle_spawn_agent_request(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    call_id: String,
    request: SpawnAgentToolRequest,
) -> Result<SpawnAgentToolResult, FunctionCallError> {
    let role_name = request
        .agent_type
        .as_deref()
        .map(str::trim)
        .filter(|role| !role.is_empty());
    let initial_operation = parse_collab_input(Some(request.message.clone()), /*items*/ None)?;
    let prompt = render_input_preview(&initial_operation);
    let child_depth = turn.next_child_spawn_depth();
    let agent_max_depth = turn.agent_max_depth();
    if exceeds_thread_spawn_depth_limit(child_depth, agent_max_depth) {
        return Err(FunctionCallError::RespondToModel(format!(
            "agent depth limit reached: cannot spawn depth {child_depth}; configured agents.max_depth is {agent_max_depth}"
        )));
    }
    let current_agent_path = session.current_agent_path_for_turn(turn.as_ref());
    let mut config = build_agent_spawn_config(
        &session.get_base_instructions().await,
        turn.as_ref(),
        request.cwd.clone(),
    )
    .await?;
    if provider_is_external(request.provider) {
        return Err(FunctionCallError::RespondToModel(
            "spawn_agent is only for Morpheus native agents; use spawn_external_agent for external code agents".to_string(),
        ));
    }
    if matches!(
        request.fork_mode,
        Some(codex_agent_runtime::SpawnAgentForkMode::FullHistory)
    ) {
        reject_full_fork_spawn_overrides(
            role_name,
            request.model.as_deref(),
            request.reasoning_effort,
        )?;
    } else {
        apply_requested_spawn_agent_model_overrides(
            &session,
            turn.as_ref(),
            &mut config,
            request.model.as_deref(),
            request.reasoning_effort,
        )
        .await?;
        refresh_spawn_cwd_agent_roles(&mut config).await?;
        apply_role_to_config(&mut config, role_name)
            .await
            .map_err(FunctionCallError::RespondToModel)?;
    }
    apply_spawn_agent_service_tier(
        &session,
        &mut config,
        turn.service_tier(),
        request.service_tier.as_deref(),
    )
    .await?;

    let spawn_source = thread_spawn_source(
        session.thread_id(),
        &SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id: session.thread_id(),
            depth: child_depth.saturating_sub(1),
            agent_path: Some(current_agent_path.clone()),
            agent_nickname: None,
            agent_role: None,
        }),
        child_depth,
        role_name,
        Some(request.task_name.clone()),
    )?;
    let result =
        Box::pin(session.spawn_agent_with_metadata(
            config,
            match (spawn_source.get_agent_path(), initial_operation) {
                (Some(recipient), Op::UserInput { items, .. })
                    if items.iter().all(|item| {
                        matches!(item, protocol::user_input::UserInput::Text { .. })
                    }) =>
                {
                    Op::InterAgentCommunication {
                        communication: InterAgentCommunication::new(
                            current_agent_path.clone(),
                            recipient,
                            Vec::new(),
                            prompt.clone(),
                            InterAgentOperation::SpawnAgent,
                        ),
                    }
                }
                (_, initial_operation) => initial_operation,
            },
            Some(spawn_source),
            SpawnAgentOptions {
                fork_parent_spawn_call_id: request.fork_mode.as_ref().map(|_| call_id.clone()),
                fork_mode: request.fork_mode,
                environments: Some(turn.spawn_agent_environment_selections(request.cwd.as_ref())),
                agent_mode: Default::default(),
            },
        ))
        .await
        .map_err(collab_spawn_error);
    let (new_thread_id, new_agent_metadata, status) = match &result {
        Ok(spawned_agent) => (
            Some(spawned_agent.thread_id),
            Some(spawned_agent.metadata.clone()),
            spawned_agent.status.clone(),
        ),
        Err(_) => (None, None, crate::agent::AgentStatus::NotFound),
    };
    let agent_snapshot = match new_thread_id {
        Some(thread_id) => session.agent_config_snapshot(thread_id).await,
        None => None,
    };
    let (new_agent_path, new_agent_nickname, new_agent_role) =
        match (&agent_snapshot, new_agent_metadata) {
            (Some(snapshot), _) => (
                snapshot.session_source.get_agent_path().map(String::from),
                snapshot.session_source.get_nickname(),
                snapshot.session_source.get_agent_role(),
            ),
            (None, Some(metadata)) => (
                metadata.agent_path.map(String::from),
                metadata.agent_nickname,
                metadata.agent_role,
            ),
            (None, None) => (None, None, None),
        };
    let effective_model = agent_snapshot
        .as_ref()
        .map(|snapshot| snapshot.model.clone())
        .unwrap_or_else(|| request.model.clone().unwrap_or_default());
    let effective_reasoning_effort = agent_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.reasoning_effort)
        .unwrap_or(request.reasoning_effort.unwrap_or_default());
    let nickname = new_agent_nickname.clone();
    session
        .send_event(
            &turn,
            CollabAgentSpawnEndEvent {
                call_id,
                completed_at_ms: crate::turn_timing::now_unix_timestamp_ms(),
                sender_thread_id: session.thread_id(),
                sender_agent_path: current_agent_path.to_string(),
                new_thread_id,
                new_agent_path: new_agent_path.clone(),
                new_agent_nickname,
                new_agent_role,
                prompt,
                model: effective_model,
                reasoning_effort: effective_reasoning_effort,
                status,
            }
            .into(),
        )
        .await;
    let _ = result?;
    let role_tag = role_name.unwrap_or(DEFAULT_ROLE_NAME);
    turn.record_multi_agent_spawn_metric(role_tag);
    let task_name = new_agent_path.ok_or_else(|| {
        FunctionCallError::RespondToModel(
            "spawned agent is missing a canonical task name".to_string(),
        )
    })?;

    if turn.hide_spawn_agent_metadata() {
        Ok(SpawnAgentToolResult::HiddenMetadata { task_name })
    } else {
        Ok(SpawnAgentToolResult::WithNickname {
            task_name,
            nickname,
        })
    }
}

async fn spawn_external_agent_request(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    call_id: String,
    request: SpawnAgentToolRequest,
) -> Result<SpawnAgentToolResult, FunctionCallError> {
    let prompt = message_content(request.message.clone())?;
    session
        .send_event(
            turn.as_ref(),
            CollabAgentSpawnBeginEvent {
                call_id: call_id.clone(),
                started_at_ms: crate::turn_timing::now_unix_timestamp_ms(),
                sender_thread_id: session.thread_id(),
                sender_agent_path: session
                    .current_agent_path_for_turn(turn.as_ref())
                    .to_string(),
                prompt: prompt.clone(),
                model: String::new(),
                reasoning_effort: Default::default(),
            }
            .into(),
        )
        .await;
    let child_depth = turn.next_child_spawn_depth();
    let agent_max_depth = turn.agent_max_depth();
    if exceeds_thread_spawn_depth_limit(child_depth, agent_max_depth) {
        return Err(FunctionCallError::RespondToModel(format!(
            "agent depth limit reached: cannot spawn depth {child_depth}; configured agents.max_depth is {agent_max_depth}"
        )));
    }
    let current_agent_path = session.current_agent_path_for_turn(turn.as_ref());
    let mut config = build_agent_spawn_config(
        &session.get_base_instructions().await,
        turn.as_ref(),
        request.cwd.clone(),
    )
    .await?;
    refresh_spawn_cwd_agent_roles(&mut config).await?;
    let provider = request.provider.ok_or_else(|| {
        FunctionCallError::RespondToModel("spawn_external_agent requires provider".to_string())
    })?;
    let spawn_source = thread_spawn_source(
        session.thread_id(),
        &SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id: session.thread_id(),
            depth: child_depth.saturating_sub(1),
            agent_path: Some(current_agent_path.clone()),
            agent_nickname: None,
            agent_role: None,
        }),
        child_depth,
        None,
        Some(request.task_name.clone()),
    )?;
    let result = session
        .spawn_external_agent_with_metadata(
            config,
            provider,
            request.message.clone(),
            spawn_source,
            SpawnAgentOptions {
                fork_parent_spawn_call_id: None,
                fork_mode: None,
                environments: None,
                agent_mode: Default::default(),
            },
        )
        .await
        .map_err(collab_spawn_error);
    let (new_thread_id, new_agent_metadata, status) = match &result {
        Ok(spawned_agent) => (
            Some(spawned_agent.thread_id),
            Some(spawned_agent.metadata.clone()),
            spawned_agent.status.clone(),
        ),
        Err(_) => (None, None, crate::agent::AgentStatus::NotFound),
    };
    let (new_agent_path, new_agent_nickname, new_agent_role) = match new_agent_metadata {
        Some(metadata) => (
            metadata.agent_path.map(String::from),
            metadata.agent_nickname,
            metadata.agent_role,
        ),
        None => (None, None, None),
    };
    let nickname = new_agent_nickname.clone();
    session
        .send_event(
            &turn,
            CollabAgentSpawnEndEvent {
                call_id,
                completed_at_ms: crate::turn_timing::now_unix_timestamp_ms(),
                sender_thread_id: session.thread_id(),
                sender_agent_path: current_agent_path.to_string(),
                new_thread_id,
                new_agent_path: new_agent_path.clone(),
                new_agent_nickname,
                new_agent_role,
                prompt,
                model: String::new(),
                reasoning_effort: Default::default(),
                status,
            }
            .into(),
        )
        .await;
    let _ = result?;
    let task_name = new_agent_path.ok_or_else(|| {
        FunctionCallError::RespondToModel(
            "spawned external agent is missing a canonical task name".to_string(),
        )
    })?;
    Ok(SpawnAgentToolResult::WithNickname {
        task_name,
        nickname,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn followup_message_content_uses_structured_preview_without_image_payload() {
        let (preview, content_parts) = followup_message_content(
            String::new(),
            vec![
                InterAgentContentPart::Text {
                    text: "inspect this".to_string(),
                },
                InterAgentContentPart::ImageRef {
                    attachment_id: "image-1".to_string(),
                    image_url: None,
                },
            ],
            &[VisibleImageAttachment {
                attachment_id: "image-1".to_string(),
                image_url: "data:image/png;base64,abc".to_string(),
            }],
        )
        .expect("structured followup should normalize");

        assert_eq!(preview, "inspect this\n[image:image-1]");
        assert!(!preview.contains("base64"));
        assert_eq!(
            content_parts,
            vec![
                InterAgentContentPart::Text {
                    text: "inspect this".to_string(),
                },
                InterAgentContentPart::ImageRef {
                    attachment_id: "image-1".to_string(),
                    image_url: Some("data:image/png;base64,abc".to_string()),
                },
            ]
        );
    }

    #[test]
    fn followup_message_content_rejects_image_ref_placeholder_in_message() {
        let err = followup_message_content(
            "please inspect [image:image-1]".to_string(),
            Vec::new(),
            &[],
        )
        .expect_err("image placeholder text should fail");

        assert!(matches!(err, FunctionCallError::RespondToModel(message)
                if message.contains("content: [{\"type\":\"image_ref\",\"attachment_id\":\"image-1\"}]")
                    && message.contains("message")));
    }

    #[test]
    fn followup_message_content_rejects_image_attachment_tag_in_text_part() {
        let err = followup_message_content(
            String::new(),
            vec![InterAgentContentPart::Text {
                text: "look at <image attachment_id=image-1>".to_string(),
            }],
            &[],
        )
        .expect_err("image open tag text should fail");

        assert!(matches!(err, FunctionCallError::RespondToModel(message)
                if message.contains("content: [{\"type\":\"image_ref\",\"attachment_id\":\"image-1\"}]")
                    && message.contains("text parts")));
    }

    #[test]
    fn followup_message_content_rejects_local_image_open_tag_in_text_part() {
        let err = followup_message_content(
            String::new(),
            vec![InterAgentContentPart::Text {
                text: "look at <image name=[Image #1] attachment_id=image-1>".to_string(),
            }],
            &[],
        )
        .expect_err("local image open tag text should fail");

        assert!(matches!(err, FunctionCallError::RespondToModel(message)
                if message.contains("content: [{\"type\":\"image_ref\",\"attachment_id\":\"image-1\"}]")));
    }

    #[test]
    fn followup_message_content_allows_plain_discussion_about_image_refs() {
        let (preview, content_parts) = followup_message_content(
            "image ref validation should explain the structured format".to_string(),
            Vec::new(),
            &[],
        )
        .expect("plain discussion should remain valid text");

        assert_eq!(
            preview,
            "image ref validation should explain the structured format"
        );
        assert!(content_parts.is_empty());
    }

    #[test]
    fn followup_message_content_rejects_unresolved_image_ref() {
        let err = followup_message_content(
            String::new(),
            vec![InterAgentContentPart::ImageRef {
                attachment_id: "image-1".to_string(),
                image_url: None,
            }],
            &[],
        )
        .expect_err("unresolved image_ref should fail");

        assert!(
            matches!(err, FunctionCallError::RespondToModel(message) if message.contains("not visible"))
        );
    }

    #[test]
    fn followup_message_content_rejects_ambiguous_image_ref() {
        let err = followup_message_content(
            String::new(),
            vec![InterAgentContentPart::ImageRef {
                attachment_id: "image-1".to_string(),
                image_url: None,
            }],
            &[
                VisibleImageAttachment {
                    attachment_id: "image-1".to_string(),
                    image_url: "data:image/png;base64,one".to_string(),
                },
                VisibleImageAttachment {
                    attachment_id: "image-1".to_string(),
                    image_url: "data:image/png;base64,two".to_string(),
                },
            ],
        )
        .expect_err("ambiguous image_ref should fail");

        assert!(
            matches!(err, FunctionCallError::RespondToModel(message) if message.contains("ambiguous"))
        );
    }

    #[test]
    fn followup_message_content_ignores_unrelated_duplicate_image_refs() {
        let (preview, content_parts) = followup_message_content(
            String::new(),
            vec![InterAgentContentPart::ImageRef {
                attachment_id: "image-2".to_string(),
                image_url: None,
            }],
            &[
                VisibleImageAttachment {
                    attachment_id: "image-1".to_string(),
                    image_url: "data:image/png;base64,old-one".to_string(),
                },
                VisibleImageAttachment {
                    attachment_id: "image-1".to_string(),
                    image_url: "data:image/png;base64,new-one".to_string(),
                },
                VisibleImageAttachment {
                    attachment_id: "image-2".to_string(),
                    image_url: "data:image/png;base64,two".to_string(),
                },
            ],
        )
        .expect("unrelated duplicate ids should not block unique requested id");

        assert_eq!(preview, "[image:image-2]");
        assert_eq!(
            content_parts,
            vec![InterAgentContentPart::ImageRef {
                attachment_id: "image-2".to_string(),
                image_url: Some("data:image/png;base64,two".to_string()),
            }]
        );
    }

    #[test]
    fn visible_image_attachments_include_forwarded_inter_agent_images() {
        let image_url = "data:image/png;base64,forwarded".to_string();
        let communication = InterAgentCommunication::new(
            AgentPath::root(),
            AgentPath::root().join("worker").expect("agent path"),
            Vec::new(),
            "[image:image-1]".to_string(),
            InterAgentOperation::FollowupTask,
        )
        .with_content_parts(vec![InterAgentContentPart::ImageRef {
            attachment_id: "image-1".to_string(),
            image_url: Some(image_url.clone()),
        }]);
        let history_item: ResponseItem = communication.to_response_input_item().into();

        assert_eq!(
            visible_image_attachments_from_history(&[history_item]),
            vec![VisibleImageAttachment {
                attachment_id: "image-1".to_string(),
                image_url,
            }]
        );
    }
}
