use std::path::Path;

use codex_execpolicy_api::Policy;
use codex_protocol::config_types::ApprovalsReviewer;
use codex_protocol::config_types::CollaborationMode;
use codex_protocol::config_types::Personality;
use codex_protocol::models::ContentItem;
use codex_protocol::models::PermissionProfile;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::TurnContextItem;

use crate::CollaborationModeInstructions;
use crate::ContextualUserFragment;
use crate::EnvironmentContext;
use crate::ModelSwitchInstructions;
use crate::PermissionsInstructions;
use crate::PersonalitySpecInstructions;
use crate::RealtimeEndInstructions;
use crate::RealtimeStartInstructions;
use crate::RealtimeStartWithInstructions;

/// Previous turn settings needed to decide whether to emit model-visible
/// context updates. This mirrors persisted rollout state without making
/// `codex-context-manager` depend on rollout/session crates.
#[derive(Clone, Copy, Debug)]
pub struct PreviousTurnSettingsView<'a> {
    pub model: &'a str,
    pub realtime_active: Option<bool>,
}

/// Inputs for building model-visible context update items between turns.
pub struct SettingsUpdateInput<'a> {
    pub previous: Option<&'a TurnContextItem>,
    pub previous_turn_settings: Option<PreviousTurnSettingsView<'a>>,
    pub include_environment_context: bool,
    pub environment_context: Option<&'a EnvironmentContext>,
    pub shell_name: &'a str,
    pub include_permissions_instructions: bool,
    pub permission_profile: &'a PermissionProfile,
    pub approval_policy: AskForApproval,
    pub approvals_reviewer: ApprovalsReviewer,
    pub exec_policy: &'a Policy,
    pub cwd: &'a Path,
    pub exec_permission_approvals_enabled: bool,
    pub request_permissions_tool_enabled: bool,
    pub include_collaboration_mode_instructions: bool,
    pub collaboration_mode: &'a CollaborationMode,
    pub realtime_active: bool,
    pub experimental_realtime_start_instructions: Option<&'a str>,
    pub personality_feature_enabled: bool,
    pub model_info: &'a ModelInfo,
    pub personality: Option<Personality>,
}

fn build_environment_update_item(input: &SettingsUpdateInput<'_>) -> Option<ResponseItem> {
    if !input.include_environment_context {
        return None;
    }

    let prev = input.previous?;
    let prev_context =
        EnvironmentContext::from_turn_context_item(prev, input.shell_name.to_string());
    let next_context = input.environment_context?;
    if prev_context.equals_except_shell(next_context) {
        return None;
    }

    Some(ContextualUserFragment::into(
        EnvironmentContext::diff_from_turn_context_item(prev, next_context),
    ))
}

fn build_permissions_update_item(input: &SettingsUpdateInput<'_>) -> Option<String> {
    if !input.include_permissions_instructions {
        return None;
    }

    let prev = input.previous?;
    if prev.permission_profile() == *input.permission_profile
        && prev.approval_policy == input.approval_policy
    {
        return None;
    }

    Some(
        PermissionsInstructions::from_permission_profile(
            input.permission_profile,
            input.approval_policy,
            input.approvals_reviewer,
            input.exec_policy,
            input.cwd,
            input.exec_permission_approvals_enabled,
            input.request_permissions_tool_enabled,
        )
        .render(),
    )
}

fn build_collaboration_mode_update_item(input: &SettingsUpdateInput<'_>) -> Option<String> {
    if !input.include_collaboration_mode_instructions {
        return None;
    }

    let prev = input.previous?;
    if prev.collaboration_mode.as_ref() != Some(input.collaboration_mode) {
        // If the next mode has empty developer instructions, this returns None
        // and we emit no update, so prior collaboration instructions remain in
        // the prompt history.
        Some(
            CollaborationModeInstructions::from_collaboration_mode(input.collaboration_mode)?
                .render(),
        )
    } else {
        None
    }
}

pub fn build_realtime_update_item(
    previous: Option<&TurnContextItem>,
    previous_turn_settings: Option<PreviousTurnSettingsView<'_>>,
    realtime_active: bool,
    experimental_realtime_start_instructions: Option<&str>,
) -> Option<String> {
    match (
        previous.and_then(|item| item.realtime_active),
        realtime_active,
    ) {
        (Some(true), false) => Some(RealtimeEndInstructions::new("inactive").render()),
        (Some(false), true) | (None, true) => Some(
            if let Some(instructions) = experimental_realtime_start_instructions {
                RealtimeStartWithInstructions::new(instructions).render()
            } else {
                RealtimeStartInstructions.render()
            },
        ),
        (Some(true), true) | (Some(false), false) => None,
        (None, false) => previous_turn_settings
            .and_then(|settings| settings.realtime_active)
            .filter(|realtime_active| *realtime_active)
            .map(|_| RealtimeEndInstructions::new("inactive").render()),
    }
}

pub fn build_initial_realtime_item(
    previous: Option<&TurnContextItem>,
    previous_turn_settings: Option<PreviousTurnSettingsView<'_>>,
    realtime_active: bool,
    experimental_realtime_start_instructions: Option<&str>,
) -> Option<String> {
    build_realtime_update_item(
        previous,
        previous_turn_settings,
        realtime_active,
        experimental_realtime_start_instructions,
    )
}

fn build_personality_update_item(input: &SettingsUpdateInput<'_>) -> Option<String> {
    if !input.personality_feature_enabled {
        return None;
    }
    let previous = input.previous?;
    if input.model_info.slug != previous.model {
        return None;
    }

    if let Some(personality) = input.personality
        && input.personality != previous.personality
    {
        personality_message_for(input.model_info, personality)
            .map(|message| PersonalitySpecInstructions::new(message).render())
    } else {
        None
    }
}

pub fn personality_message_for(model_info: &ModelInfo, personality: Personality) -> Option<String> {
    model_info
        .model_messages
        .as_ref()
        .and_then(|spec| spec.get_personality_message(Some(personality)))
        .filter(|message| !message.is_empty())
}

pub fn build_model_instructions_update_item(
    previous_turn_settings: Option<PreviousTurnSettingsView<'_>>,
    model_info: &ModelInfo,
    personality: Option<Personality>,
) -> Option<String> {
    let previous_turn_settings = previous_turn_settings?;
    if previous_turn_settings.model == model_info.slug {
        return None;
    }

    let model_instructions = model_info.get_model_instructions(personality);
    if model_instructions.is_empty() {
        return None;
    }

    Some(ModelSwitchInstructions::new(model_instructions).render())
}

pub fn build_developer_update_item(text_sections: Vec<String>) -> Option<ResponseItem> {
    build_text_message("developer", text_sections)
}

pub fn build_contextual_user_message(text_sections: Vec<String>) -> Option<ResponseItem> {
    build_text_message("user", text_sections)
}

fn build_text_message(role: &str, text_sections: Vec<String>) -> Option<ResponseItem> {
    if text_sections.is_empty() {
        return None;
    }

    let content = text_sections
        .into_iter()
        .map(|text| ContentItem::InputText { text })
        .collect();

    Some(ResponseItem::Message {
        id: None,
        role: role.to_string(),
        content,
        phase: None,
    })
}

pub fn build_settings_update_items(input: SettingsUpdateInput<'_>) -> Vec<ResponseItem> {
    // TODO(ccunningham): build_settings_update_items still does not cover every
    // model-visible item emitted by build_initial_context. Persist the remaining
    // inputs or add explicit replay events so fork/resume can diff everything
    // deterministically.
    let contextual_user_message = build_environment_update_item(&input);
    let developer_update_sections = [
        // Keep model-switch instructions first so model-specific guidance is
        // read before any other context diffs on this turn.
        build_model_instructions_update_item(
            input.previous_turn_settings,
            input.model_info,
            input.personality,
        ),
        build_permissions_update_item(&input),
        build_collaboration_mode_update_item(&input),
        build_realtime_update_item(
            input.previous,
            input.previous_turn_settings,
            input.realtime_active,
            input.experimental_realtime_start_instructions,
        ),
        build_personality_update_item(&input),
    ]
    .into_iter()
    .flatten()
    .collect();

    let mut items = Vec::with_capacity(2);
    if let Some(developer_message) = build_developer_update_item(developer_update_sections) {
        items.push(developer_message);
    }
    if let Some(contextual_user_message) = contextual_user_message {
        items.push(contextual_user_message);
    }
    items
}
