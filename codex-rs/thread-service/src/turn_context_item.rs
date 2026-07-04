use std::path::PathBuf;

use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::config_types::CollaborationMode;
use protocol::config_types::Personality;
use protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use protocol::models::PermissionProfile;
use protocol::openai_models::ReasoningEffort;
use protocol::permissions::FileSystemSandboxPolicy;
use protocol::protocol::AskForApproval;
use protocol::protocol::SandboxPolicy;
use protocol::protocol::TruncationPolicy;
use protocol::protocol::TurnContextItem;
use protocol::protocol::TurnContextNetworkItem;
use serde_json::Value;

/// Fully materialized data needed to project a runtime turn context into the
/// persisted/model-visible turn context item.
#[derive(Clone, Debug)]
pub struct TurnContextItemBuildInput {
    pub turn_id: Option<String>,
    pub trace_id: Option<String>,
    pub cwd: AbsolutePathBuf,
    pub current_date: Option<String>,
    pub timezone: Option<String>,
    pub approval_policy: AskForApproval,
    pub sandbox_policy: SandboxPolicy,
    pub permission_profile: PermissionProfile,
    pub network: Option<TurnContextNetworkItem>,
    pub file_system_sandbox_policy: FileSystemSandboxPolicy,
    pub model: String,
    pub personality: Option<Personality>,
    pub collaboration_mode: CollaborationMode,
    pub realtime_active: bool,
    pub effort: Option<ReasoningEffort>,
    pub summary: ReasoningSummaryConfig,
    pub user_instructions: Option<String>,
    pub developer_instructions: Option<String>,
    pub final_output_json_schema: Option<Value>,
    pub truncation_policy: TruncationPolicy,
}

pub fn build_turn_context_item(input: TurnContextItemBuildInput) -> TurnContextItem {
    let file_system_sandbox_policy = non_legacy_file_system_sandbox_policy(
        &input.sandbox_policy,
        &input.cwd,
        &input.file_system_sandbox_policy,
    );
    TurnContextItem {
        turn_id: input.turn_id,
        trace_id: input.trace_id,
        cwd: PathBuf::from(input.cwd),
        current_date: input.current_date,
        timezone: input.timezone,
        approval_policy: input.approval_policy,
        sandbox_policy: input.sandbox_policy,
        permission_profile: Some(input.permission_profile),
        network: input.network,
        file_system_sandbox_policy,
        model: input.model,
        personality: input.personality,
        collaboration_mode: Some(input.collaboration_mode),
        realtime_active: Some(input.realtime_active),
        effort: input.effort,
        summary: input.summary,
        user_instructions: input.user_instructions,
        developer_instructions: input.developer_instructions,
        final_output_json_schema: input.final_output_json_schema,
        truncation_policy: Some(input.truncation_policy),
    }
}

fn non_legacy_file_system_sandbox_policy(
    sandbox_policy: &SandboxPolicy,
    cwd: &AbsolutePathBuf,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
) -> Option<FileSystemSandboxPolicy> {
    // Omit the derived split filesystem policy when it is equivalent to the
    // legacy sandbox policy. This keeps turn-context payloads stable while
    // both fields exist; once callers consume only the split policy, this
    // comparison and the legacy projection should go away.
    let legacy_file_system_sandbox_policy =
        FileSystemSandboxPolicy::from_legacy_sandbox_policy_for_cwd(sandbox_policy, cwd);
    (file_system_sandbox_policy != &legacy_file_system_sandbox_policy)
        .then_some(file_system_sandbox_policy.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::config_types::ModeKind;
    use protocol::config_types::Settings;
    use protocol::models::SandboxEnforcement;
    use protocol::permissions::NetworkSandboxPolicy;

    fn cwd() -> AbsolutePathBuf {
        AbsolutePathBuf::from_absolute_path("/tmp/project").expect("absolute test path")
    }

    fn unrestricted_permission_profile() -> PermissionProfile {
        PermissionProfile::from_runtime_permissions_with_enforcement(
            SandboxEnforcement::Disabled,
            &FileSystemSandboxPolicy::unrestricted(),
            NetworkSandboxPolicy::Enabled,
        )
    }

    fn collaboration_mode() -> CollaborationMode {
        CollaborationMode {
            mode: ModeKind::Default,
            settings: Settings {
                model: "gpt-test".to_string(),
                reasoning_effort: None,
                developer_instructions: None,
            },
        }
    }

    fn sample_input() -> TurnContextItemBuildInput {
        TurnContextItemBuildInput {
            turn_id: Some("turn-1".to_string()),
            trace_id: Some("trace-1".to_string()),
            cwd: cwd(),
            current_date: Some("2026-06-24".to_string()),
            timezone: Some("Asia/Shanghai".to_string()),
            approval_policy: AskForApproval::OnRequest,
            sandbox_policy: SandboxPolicy::DangerFullAccess,
            permission_profile: unrestricted_permission_profile(),
            network: Some(TurnContextNetworkItem {
                allowed_domains: vec!["example.com".to_string()],
                denied_domains: vec!["blocked.example".to_string()],
            }),
            file_system_sandbox_policy: FileSystemSandboxPolicy::unrestricted(),
            model: "gpt-test".to_string(),
            personality: None,
            collaboration_mode: collaboration_mode(),
            realtime_active: true,
            effort: None,
            summary: ReasoningSummaryConfig::Auto,
            user_instructions: Some("user".to_string()),
            developer_instructions: Some("developer".to_string()),
            final_output_json_schema: Some(serde_json::json!({"type": "object"})),
            truncation_policy: TruncationPolicy::Tokens(1_000),
        }
    }

    #[test]
    fn turn_context_projection_materializes_item_fields() {
        let item = build_turn_context_item(sample_input());

        assert_eq!(item.turn_id.as_deref(), Some("turn-1"));
        assert_eq!(item.trace_id.as_deref(), Some("trace-1"));
        assert_eq!(item.cwd, PathBuf::from("/tmp/project"));
        assert_eq!(item.current_date.as_deref(), Some("2026-06-24"));
        assert_eq!(item.timezone.as_deref(), Some("Asia/Shanghai"));
        assert_eq!(item.approval_policy, AskForApproval::OnRequest);
        assert_eq!(item.sandbox_policy, SandboxPolicy::DangerFullAccess);
        assert_eq!(
            item.permission_profile,
            Some(unrestricted_permission_profile())
        );
        assert_eq!(
            item.network,
            Some(TurnContextNetworkItem {
                allowed_domains: vec!["example.com".to_string()],
                denied_domains: vec!["blocked.example".to_string()],
            })
        );
        assert_eq!(item.file_system_sandbox_policy, None);
        assert_eq!(item.model, "gpt-test");
        assert_eq!(item.collaboration_mode, Some(collaboration_mode()));
        assert_eq!(item.realtime_active, Some(true));
        assert_eq!(item.user_instructions.as_deref(), Some("user"));
        assert_eq!(item.developer_instructions.as_deref(), Some("developer"));
        assert_eq!(
            item.truncation_policy,
            Some(TruncationPolicy::Tokens(1_000))
        );
    }

    #[test]
    fn turn_context_projection_keeps_non_legacy_file_system_policy() {
        let mut input = sample_input();
        input.file_system_sandbox_policy = FileSystemSandboxPolicy::default();

        let item = build_turn_context_item(input);

        assert_eq!(
            item.file_system_sandbox_policy,
            Some(FileSystemSandboxPolicy::default())
        );
    }
}
