use std::path::PathBuf;

use codex_analytics_api::TurnResolvedConfigFact;
use protocol::config_types::ApprovalsReviewer;
use protocol::config_types::ModeKind;
use protocol::config_types::Personality;
use protocol::config_types::ReasoningSummary;
use protocol::config_types::ServiceTier;
use protocol::models::PermissionProfile;
use protocol::openai_models::ReasoningEffort;
use protocol::protocol::AskForApproval;
use protocol::protocol::SessionSource;

pub struct TurnResolvedConfigFactInput {
    pub turn_id: String,
    pub thread_id: String,
    pub num_input_images: usize,
    pub ephemeral: bool,
    pub session_source: SessionSource,
    pub model: String,
    pub model_provider: String,
    pub permission_profile: PermissionProfile,
    pub permission_profile_cwd: PathBuf,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub reasoning_summary: ReasoningSummary,
    pub service_tier: Option<String>,
    pub approval_policy: AskForApproval,
    pub approvals_reviewer: ApprovalsReviewer,
    pub sandbox_network_access: bool,
    pub collaboration_mode: ModeKind,
    pub personality: Option<Personality>,
    pub is_first_turn: bool,
}

pub fn build_turn_resolved_config_fact(
    input: TurnResolvedConfigFactInput,
) -> TurnResolvedConfigFact {
    TurnResolvedConfigFact {
        turn_id: input.turn_id,
        thread_id: input.thread_id,
        num_input_images: input.num_input_images,
        submission_type: None,
        ephemeral: input.ephemeral,
        session_source: input.session_source,
        model: input.model,
        model_provider: input.model_provider,
        permission_profile: input.permission_profile,
        permission_profile_cwd: input.permission_profile_cwd,
        reasoning_effort: input.reasoning_effort,
        reasoning_summary: Some(input.reasoning_summary),
        service_tier: input
            .service_tier
            .as_deref()
            .and_then(ServiceTier::from_request_value),
        approval_policy: input.approval_policy,
        approvals_reviewer: input.approvals_reviewer,
        sandbox_network_access: input.sandbox_network_access,
        collaboration_mode: input.collaboration_mode,
        personality: input.personality,
        is_first_turn: input.is_first_turn,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::config_types::ReasoningSummary;
    use protocol::models::PermissionProfile;
    use protocol::openai_models::ReasoningEffort;
    use protocol::protocol::SessionSource;

    fn sample_input(service_tier: Option<String>) -> TurnResolvedConfigFactInput {
        TurnResolvedConfigFactInput {
            turn_id: "turn-1".to_string(),
            thread_id: "thread-1".to_string(),
            num_input_images: 2,
            ephemeral: true,
            session_source: SessionSource::Exec,
            model: "gpt-test".to_string(),
            model_provider: "openai".to_string(),
            permission_profile: PermissionProfile::read_only(),
            permission_profile_cwd: PathBuf::from("/tmp/work"),
            reasoning_effort: Some(ReasoningEffort::High),
            reasoning_summary: ReasoningSummary::Detailed,
            service_tier,
            approval_policy: AskForApproval::Never,
            approvals_reviewer: ApprovalsReviewer::User,
            sandbox_network_access: false,
            collaboration_mode: ModeKind::Default,
            personality: None,
            is_first_turn: true,
        }
    }

    #[test]
    fn builds_turn_resolved_config_fact() {
        let fact = build_turn_resolved_config_fact(sample_input(Some("priority".to_string())));

        assert_eq!(fact.turn_id, "turn-1");
        assert_eq!(fact.thread_id, "thread-1");
        assert_eq!(fact.num_input_images, 2);
        assert!(fact.ephemeral);
        assert_eq!(fact.model, "gpt-test");
        assert_eq!(fact.model_provider, "openai");
        assert_eq!(fact.reasoning_effort, Some(ReasoningEffort::High));
        assert_eq!(fact.reasoning_summary, Some(ReasoningSummary::Detailed));
        assert_eq!(fact.service_tier, Some(ServiceTier::Fast));
        assert_eq!(fact.approval_policy, AskForApproval::Never);
        assert_eq!(fact.approvals_reviewer, ApprovalsReviewer::User);
        assert_eq!(fact.collaboration_mode, ModeKind::Default);
        assert!(fact.is_first_turn);
    }

    #[test]
    fn preserves_unknown_service_tier_as_none() {
        let fact = build_turn_resolved_config_fact(sample_input(Some("unknown".to_string())));

        assert_eq!(fact.service_tier, None);
    }
}
