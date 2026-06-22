use crate::context::environment_context_from_turn_context;
use crate::session::PreviousTurnSettings;
use crate::session::turn_context::TurnContext;
use crate::shell::Shell;
use codex_context_manager::PreviousTurnSettingsView;
use codex_context_manager::SettingsUpdateInput;
use codex_execpolicy_api::Policy;
use codex_features::Feature;
use codex_protocol::config_types::Personality;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::protocol::TurnContextItem;

fn previous_turn_settings_view(
    previous_turn_settings: Option<&PreviousTurnSettings>,
) -> Option<PreviousTurnSettingsView<'_>> {
    previous_turn_settings.map(|settings| PreviousTurnSettingsView {
        model: settings.model.as_str(),
        realtime_active: settings.realtime_active,
    })
}

pub(crate) fn build_initial_realtime_item(
    previous: Option<&TurnContextItem>,
    previous_turn_settings: Option<&PreviousTurnSettings>,
    next: &TurnContext,
) -> Option<String> {
    codex_context_manager::build_initial_realtime_item(
        previous,
        previous_turn_settings_view(previous_turn_settings),
        next.realtime_active,
        next.config
            .experimental_realtime_start_instructions
            .as_deref(),
    )
}

pub(crate) fn personality_message_for(
    model_info: &ModelInfo,
    personality: Personality,
) -> Option<String> {
    codex_context_manager::personality_message_for(model_info, personality)
}

pub(crate) fn build_model_instructions_update_item(
    previous_turn_settings: Option<&PreviousTurnSettings>,
    next: &TurnContext,
) -> Option<String> {
    codex_context_manager::build_model_instructions_update_item(
        previous_turn_settings_view(previous_turn_settings),
        &next.model_info,
        next.personality,
    )
}

pub(crate) fn build_developer_update_item(text_sections: Vec<String>) -> Option<ResponseItem> {
    codex_context_manager::build_developer_update_item(text_sections)
}

pub(crate) fn build_contextual_user_message(text_sections: Vec<String>) -> Option<ResponseItem> {
    codex_context_manager::build_contextual_user_message(text_sections)
}

pub(crate) fn build_settings_update_items(
    previous: Option<&TurnContextItem>,
    previous_turn_settings: Option<&PreviousTurnSettings>,
    next: &TurnContext,
    shell: &Shell,
    exec_policy: &Policy,
    personality_feature_enabled: bool,
) -> Vec<ResponseItem> {
    let environment_context = environment_context_from_turn_context(next, shell);
    codex_context_manager::build_settings_update_items(SettingsUpdateInput {
        previous,
        previous_turn_settings: previous_turn_settings_view(previous_turn_settings),
        include_environment_context: next.config.include_environment_context,
        environment_context: Some(&environment_context),
        shell_name: shell.name(),
        include_permissions_instructions: next.config.include_permissions_instructions,
        permission_profile: &next.permission_profile,
        approval_policy: next.approval_policy.value(),
        approvals_reviewer: next.config.approvals_reviewer,
        exec_policy,
        #[allow(deprecated)]
        cwd: &next.cwd,
        exec_permission_approvals_enabled: next.features.enabled(Feature::ExecPermissionApprovals),
        request_permissions_tool_enabled: next.features.enabled(Feature::RequestPermissionsTool),
        include_collaboration_mode_instructions: next
            .config
            .include_collaboration_mode_instructions,
        collaboration_mode: &next.collaboration_mode,
        realtime_active: next.realtime_active,
        experimental_realtime_start_instructions: next
            .config
            .experimental_realtime_start_instructions
            .as_deref(),
        personality_feature_enabled,
        model_info: &next.model_info,
        personality: next.personality,
    })
}
