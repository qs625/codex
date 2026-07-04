use std::collections::HashMap;

use protocol::config_types::CollaborationMode;
use protocol::config_types::ModeKind;
use protocol::config_types::Settings;
use protocol::protocol::Op;
use protocol::user_input::UserInput;

use crate::SessionSettingsUpdate;

pub struct UserTurnSubmission {
    pub items: Vec<UserInput>,
    pub updates: SessionSettingsUpdate,
    pub responsesapi_client_metadata: Option<HashMap<String, String>>,
}

pub fn user_turn_submission_from_op(
    op: Op,
    current_collaboration_mode: &CollaborationMode,
) -> Option<UserTurnSubmission> {
    match op {
        Op::UserTurn {
            cwd,
            approval_policy,
            approvals_reviewer,
            sandbox_policy,
            permission_profile,
            model,
            effort,
            summary,
            service_tier,
            final_output_json_schema,
            items,
            collaboration_mode,
            personality,
            environments,
        } => {
            let collaboration_mode = collaboration_mode.or_else(|| {
                Some(CollaborationMode {
                    mode: ModeKind::Default,
                    settings: Settings {
                        model,
                        reasoning_effort: effort,
                        developer_instructions: None,
                    },
                })
            });
            Some(UserTurnSubmission {
                items,
                updates: SessionSettingsUpdate {
                    cwd: Some(cwd),
                    approval_policy: Some(approval_policy),
                    approvals_reviewer,
                    sandbox_policy: Some(sandbox_policy),
                    workspace_roots: None,
                    profile_workspace_roots: None,
                    permission_profile,
                    active_permission_profile: None,
                    windows_sandbox_level: None,
                    model_provider: None,
                    collaboration_mode,
                    reasoning_summary: summary,
                    service_tier,
                    final_output_json_schema: Some(final_output_json_schema),
                    environments,
                    personality,
                    app_server_client_name: None,
                    app_server_client_version: None,
                },
                responsesapi_client_metadata: None,
            })
        }
        Op::UserInputWithTurnContext {
            cwd,
            workspace_roots,
            profile_workspace_roots,
            approval_policy,
            approvals_reviewer,
            sandbox_policy,
            permission_profile,
            active_permission_profile,
            windows_sandbox_level,
            model,
            model_provider,
            effort,
            summary,
            service_tier,
            final_output_json_schema,
            items,
            responsesapi_client_metadata,
            collaboration_mode,
            personality,
            environments,
        } => {
            let collaboration_mode = collaboration_mode.or_else(|| {
                Some(
                    current_collaboration_mode
                        .with_updates(model, effort, /*developer_instructions*/ None),
                )
            });
            Some(UserTurnSubmission {
                items,
                updates: SessionSettingsUpdate {
                    cwd,
                    workspace_roots,
                    profile_workspace_roots,
                    approval_policy,
                    approvals_reviewer,
                    sandbox_policy,
                    permission_profile,
                    active_permission_profile,
                    windows_sandbox_level,
                    model_provider,
                    collaboration_mode,
                    reasoning_summary: summary,
                    service_tier,
                    final_output_json_schema: Some(final_output_json_schema),
                    environments,
                    personality,
                    app_server_client_name: None,
                    app_server_client_version: None,
                },
                responsesapi_client_metadata,
            })
        }
        Op::UserInput {
            items,
            environments,
            final_output_json_schema,
            responsesapi_client_metadata,
        } => Some(UserTurnSubmission {
            items,
            updates: SessionSettingsUpdate {
                final_output_json_schema: Some(final_output_json_schema),
                environments,
                ..Default::default()
            },
            responsesapi_client_metadata,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::config_types::ApprovalsReviewer;
    use protocol::openai_models::ReasoningEffort;
    use protocol::protocol::AskForApproval;
    use protocol::protocol::SandboxPolicy;
    use serde_json::json;
    use std::path::PathBuf;

    fn text_input(text: &str) -> UserInput {
        UserInput::Text {
            text: text.to_string(),
            text_elements: Vec::new(),
        }
    }

    fn current_collaboration_mode() -> CollaborationMode {
        CollaborationMode {
            mode: ModeKind::Default,
            settings: Settings {
                model: "gpt-current".to_string(),
                reasoning_effort: Some(ReasoningEffort::Low),
                developer_instructions: Some("keep me".to_string()),
            },
        }
    }

    #[test]
    fn user_turn_builds_default_collaboration_mode_when_missing() {
        let submission = user_turn_submission_from_op(
            Op::UserTurn {
                items: vec![text_input("hello")],
                cwd: PathBuf::from("/tmp/work"),
                approval_policy: AskForApproval::Never,
                approvals_reviewer: Some(ApprovalsReviewer::User),
                sandbox_policy: SandboxPolicy::DangerFullAccess,
                permission_profile: None,
                model: "gpt-next".to_string(),
                effort: Some(ReasoningEffort::High),
                summary: None,
                service_tier: Some(Some("priority".to_string())),
                final_output_json_schema: None,
                collaboration_mode: None,
                personality: None,
                environments: None,
            },
            &current_collaboration_mode(),
        )
        .expect("user turn op should convert");

        assert_eq!(submission.items, vec![text_input("hello")]);
        assert_eq!(submission.updates.cwd, Some(PathBuf::from("/tmp/work")));
        assert_eq!(
            submission.updates.approval_policy,
            Some(AskForApproval::Never)
        );
        assert_eq!(
            submission
                .updates
                .collaboration_mode
                .expect("collaboration mode")
                .settings
                .model,
            "gpt-next"
        );
        assert_eq!(
            submission.updates.service_tier,
            Some(Some("priority".to_string()))
        );
        assert_eq!(submission.responsesapi_client_metadata, None);
    }

    #[test]
    fn user_input_with_turn_context_updates_current_collaboration_mode() {
        let submission = user_turn_submission_from_op(
            Op::UserInputWithTurnContext {
                items: vec![text_input("steer")],
                environments: None,
                final_output_json_schema: Some(json!({"type": "object"})),
                responsesapi_client_metadata: Some(HashMap::from([(
                    "trace".to_string(),
                    "abc".to_string(),
                )])),
                cwd: None,
                workspace_roots: None,
                profile_workspace_roots: None,
                approval_policy: None,
                approvals_reviewer: None,
                sandbox_policy: None,
                permission_profile: None,
                active_permission_profile: None,
                windows_sandbox_level: None,
                model: Some("gpt-updated".to_string()),
                model_provider: Some("openai".to_string()),
                effort: Some(Some(ReasoningEffort::Medium)),
                summary: None,
                service_tier: None,
                collaboration_mode: None,
                personality: None,
            },
            &current_collaboration_mode(),
        )
        .expect("turn-context op should convert");

        let collaboration_mode = submission
            .updates
            .collaboration_mode
            .expect("collaboration mode");
        assert_eq!(collaboration_mode.settings.model, "gpt-updated");
        assert_eq!(
            collaboration_mode.settings.reasoning_effort,
            Some(ReasoningEffort::Medium)
        );
        assert_eq!(
            submission.updates.final_output_json_schema,
            Some(Some(json!({"type": "object"})))
        );
        assert_eq!(
            submission.responsesapi_client_metadata,
            Some(HashMap::from([("trace".to_string(), "abc".to_string())]))
        );
    }

    #[test]
    fn user_input_only_updates_turn_local_fields() {
        let submission = user_turn_submission_from_op(
            Op::UserInput {
                items: vec![text_input("plain")],
                environments: Some(Vec::new()),
                final_output_json_schema: None,
                responsesapi_client_metadata: Some(HashMap::from([(
                    "client".to_string(),
                    "desktop".to_string(),
                )])),
            },
            &current_collaboration_mode(),
        )
        .expect("user input op should convert");

        assert_eq!(submission.items, vec![text_input("plain")]);
        assert_eq!(submission.updates.final_output_json_schema, Some(None));
        assert_eq!(submission.updates.environments, Some(Vec::new()));
        assert_eq!(submission.updates.collaboration_mode, None);
        assert_eq!(
            submission.responsesapi_client_metadata,
            Some(HashMap::from([(
                "client".to_string(),
                "desktop".to_string()
            )]))
        );
    }
}
