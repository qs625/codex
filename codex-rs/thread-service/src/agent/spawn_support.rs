use crate::config::Config;
use crate::config::agent_roles::merge_agent_roles_from_dirs;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use codex_file_system::LOCAL_FS;
use codex_git_info::resolve_root_git_project_for_trust;
use protocol::AgentPath;
use protocol::ThreadId;
use protocol::error::CodexErr;
use protocol::models::BaseInstructions;
use protocol::openai_models::ReasoningEffort;
use protocol::openai_models::ReasoningEffortPreset;
use protocol::protocol::Op;
use protocol::protocol::SessionSource;
use protocol::protocol::SubAgentSource;
use protocol::user_input::UserInput;
use std::collections::BTreeSet;
use std::path::PathBuf;
use tool_service_api::FunctionCallError;

pub(crate) fn collab_spawn_error(err: CodexErr) -> FunctionCallError {
    match err {
        CodexErr::UnsupportedOperation(message) if message == "thread manager dropped" => {
            FunctionCallError::RespondToModel("collab manager unavailable".to_string())
        }
        CodexErr::UnsupportedOperation(message) => FunctionCallError::RespondToModel(message),
        CodexErr::AgentLimitReached { max_threads } => FunctionCallError::RespondToModel(format!(
            "agent thread limit reached; configured agents.max_threads is {max_threads}"
        )),
        err => FunctionCallError::RespondToModel(format!("collab spawn failed: {err}")),
    }
}

pub(crate) fn collab_agent_error(agent_id: ThreadId, err: CodexErr) -> FunctionCallError {
    match err {
        CodexErr::ThreadNotFound(id) => {
            FunctionCallError::RespondToModel(format!("agent with id {id} not found"))
        }
        CodexErr::InternalAgentDied => {
            FunctionCallError::RespondToModel(format!("agent with id {agent_id} is closed"))
        }
        CodexErr::UnsupportedOperation(_) => {
            FunctionCallError::RespondToModel("collab manager unavailable".to_string())
        }
        err => FunctionCallError::RespondToModel(format!("collab tool failed: {err}")),
    }
}

pub(crate) fn thread_spawn_source(
    parent_thread_id: ThreadId,
    parent_session_source: &SessionSource,
    depth: i32,
    agent_role: Option<&str>,
    task_name: Option<String>,
) -> Result<SessionSource, FunctionCallError> {
    let agent_path = task_name
        .as_deref()
        .map(|task_name| {
            parent_session_source
                .get_agent_path()
                .unwrap_or_else(AgentPath::root)
                .join(task_name)
                .map_err(FunctionCallError::RespondToModel)
        })
        .transpose()?;
    Ok(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id,
        depth,
        agent_path,
        agent_nickname: None,
        agent_role: agent_role.map(str::to_string),
    }))
}

pub(crate) fn parse_collab_input(
    message: Option<String>,
    items: Option<Vec<UserInput>>,
) -> Result<Op, FunctionCallError> {
    match (message, items) {
        (Some(_), Some(_)) => Err(FunctionCallError::RespondToModel(
            "Provide either message or items, but not both".to_string(),
        )),
        (None, None) => Err(FunctionCallError::RespondToModel(
            "Provide one of: message or items".to_string(),
        )),
        (Some(message), None) => {
            if message.trim().is_empty() {
                return Err(FunctionCallError::RespondToModel(
                    "Empty message can't be sent to an agent".to_string(),
                ));
            }
            Ok(vec![UserInput::Text {
                text: message,
                text_elements: Vec::new(),
            }]
            .into())
        }
        (None, Some(items)) => {
            if items.is_empty() {
                return Err(FunctionCallError::RespondToModel(
                    "Items can't be empty".to_string(),
                ));
            }
            Ok(items.into())
        }
    }
}

/// Builds the base config snapshot for a newly spawned sub-agent.
///
/// The returned config starts from the parent's effective config and then refreshes the
/// runtime-owned fields carried on `turn`, including model selection, reasoning settings,
/// approval policy, sandbox, and cwd. Role-specific overrides are layered after this step;
/// skipping this helper and cloning stale config state directly can send the child agent out with
/// the wrong provider or runtime policy.
use codex_utils_absolute_path::AbsolutePathBuf;

pub(crate) fn build_agent_spawn_config(
    base_instructions: &BaseInstructions,
    turn: &TurnContext,
    cwd: Option<AbsolutePathBuf>,
) -> Result<Config, FunctionCallError> {
    let mut config = build_agent_shared_config(turn, cwd)?;
    config.base_instructions = Some(base_instructions.text.clone());
    Ok(config)
}

pub(crate) async fn refresh_spawn_cwd_agent_roles(
    config: &mut Config,
) -> Result<(), FunctionCallError> {
    let mut seen = BTreeSet::<PathBuf>::new();
    let mut agent_dirs = Vec::new();
    for agents_dir in [
        config.codex_home.join("agents"),
        config.cwd.join(".codex").join("agents"),
    ] {
        if seen.insert(agents_dir.to_path_buf()) {
            agent_dirs.push(agents_dir);
        }
    }

    if let Some(repo_root) =
        resolve_root_git_project_for_trust(LOCAL_FS.as_ref(), &config.cwd).await
    {
        let repo_agents_dir = repo_root.join(".codex").join("agents");
        if seen.insert(repo_agents_dir.to_path_buf()) {
            agent_dirs.push(repo_agents_dir);
        }
    }

    let mut warnings = Vec::new();
    merge_agent_roles_from_dirs(
        LOCAL_FS.as_ref(),
        &mut config.agent_roles,
        &agent_dirs,
        &mut warnings,
    )
    .await
    .map_err(|err| {
        FunctionCallError::RespondToModel(format!(
            "failed to refresh agent roles for child cwd: {err}"
        ))
    })?;
    config.startup_warnings.extend(warnings);
    Ok(())
}

fn build_agent_shared_config(
    turn: &TurnContext,
    cwd: Option<AbsolutePathBuf>,
) -> Result<Config, FunctionCallError> {
    turn.build_agent_shared_config(cwd)
}

pub(crate) fn reject_full_fork_spawn_overrides(
    agent_type: Option<&str>,
    model: Option<&str>,
    reasoning_effort: Option<ReasoningEffort>,
) -> Result<(), FunctionCallError> {
    if agent_type.is_some() || model.is_some() || reasoning_effort.is_some() {
        return Err(FunctionCallError::RespondToModel(
            "Full-history forked agents inherit the parent agent type, model, and reasoning effort; omit agent_type, model, and reasoning_effort, or spawn without a full-history fork.".to_string(),
        ));
    }
    Ok(())
}

pub(crate) async fn apply_requested_spawn_agent_model_overrides(
    session: &Session,
    turn: &TurnContext,
    config: &mut Config,
    requested_model: Option<&str>,
    requested_reasoning_effort: Option<ReasoningEffort>,
) -> Result<(), FunctionCallError> {
    if requested_model.is_none() && requested_reasoning_effort.is_none() {
        return Ok(());
    }

    if let Some(requested_model) = requested_model {
        let available_models = session.list_spawn_agent_models().await;
        let selected_model_name = find_spawn_agent_model_name(&available_models, requested_model)?;
        let selected_model_info = session
            .spawn_agent_model_info(&selected_model_name, config)
            .await
            .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;

        config.model = Some(selected_model_name.clone());
        if let Some(reasoning_effort) = requested_reasoning_effort {
            validate_spawn_agent_reasoning_effort(
                &selected_model_name,
                &selected_model_info.supported_reasoning_levels,
                reasoning_effort,
            )?;
            config.model_reasoning_effort = Some(reasoning_effort);
        } else {
            config.model_reasoning_effort = selected_model_info.default_reasoning_level;
        }

        return Ok(());
    }

    if let Some(reasoning_effort) = requested_reasoning_effort {
        validate_spawn_agent_reasoning_effort(
            turn.model_slug(),
            turn.supported_reasoning_levels(),
            reasoning_effort,
        )?;
        config.model_reasoning_effort = Some(reasoning_effort);
    }

    Ok(())
}

pub(crate) async fn apply_spawn_agent_service_tier(
    session: &Session,
    config: &mut Config,
    parent_service_tier: Option<&str>,
    requested_service_tier: Option<&str>,
) -> Result<(), FunctionCallError> {
    let Some(candidate_service_tier) = requested_service_tier.or(parent_service_tier) else {
        return Ok(());
    };
    let model = config.model.clone().ok_or_else(|| {
        FunctionCallError::RespondToModel(
            "spawn_agent could not resolve the child model for service tier validation".to_string(),
        )
    })?;
    let model_info = session
        .spawn_agent_model_info(model.as_str(), config)
        .await
        .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;

    if model_info.supports_service_tier(candidate_service_tier) {
        config.service_tier = Some(candidate_service_tier.to_string());
        return Ok(());
    }

    if requested_service_tier.is_none() {
        config.service_tier = None;
        return Ok(());
    }

    let supported_service_tiers = if model_info.service_tiers.is_empty() {
        "none".to_string()
    } else {
        model_info
            .service_tiers
            .iter()
            .map(|tier| tier.id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    Err(FunctionCallError::RespondToModel(format!(
        "Service tier `{candidate_service_tier}` is not supported for model `{model}`. Supported service tiers: {supported_service_tiers}"
    )))
}

fn find_spawn_agent_model_name(
    available_models: &[protocol::openai_models::ModelPreset],
    requested_model: &str,
) -> Result<String, FunctionCallError> {
    available_models
        .iter()
        .find(|model| model.model == requested_model)
        .map(|model| model.model.clone())
        .ok_or_else(|| {
            let available = available_models
                .iter()
                .map(|model| model.model.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            FunctionCallError::RespondToModel(format!(
                "Unknown model `{requested_model}` for spawn_agent. Available models: {available}"
            ))
        })
}

fn validate_spawn_agent_reasoning_effort(
    model: &str,
    supported_reasoning_levels: &[ReasoningEffortPreset],
    requested_reasoning_effort: ReasoningEffort,
) -> Result<(), FunctionCallError> {
    if supported_reasoning_levels
        .iter()
        .any(|preset| preset.effort == requested_reasoning_effort)
    {
        return Ok(());
    }

    let supported = supported_reasoning_levels
        .iter()
        .map(|preset| preset.effort.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(FunctionCallError::RespondToModel(format!(
        "Reasoning effort `{requested_reasoning_effort}` is not supported for model `{model}`. Supported reasoning efforts: {supported}"
    )))
}
