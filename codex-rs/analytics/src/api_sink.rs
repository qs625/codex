use std::sync::Arc;

use crate::client::AnalyticsEventsClient;
use crate::events;
use crate::facts;
use codex_analytics_api as api;
use codex_analytics_api::AnalyticsEventsSink;

impl AnalyticsEventsClient {
    pub fn api_client(&self) -> api::AnalyticsEventsClient {
        api::AnalyticsEventsClient::from_sink(Arc::new(self.clone()))
    }
}

impl AnalyticsEventsSink for AnalyticsEventsClient {
    fn track_skill_invocations(
        &self,
        tracking: api::TrackEventsContext,
        invocations: Vec<api::SkillInvocation>,
    ) {
        self.track_skill_invocations(
            tracking.into(),
            invocations.into_iter().map(Into::into).collect(),
        );
    }

    fn track_subagent_thread_started(&self, input: api::SubAgentThreadStartedInput) {
        self.track_subagent_thread_started(input.into());
    }

    fn track_guardian_review(
        &self,
        tracking: &api::GuardianReviewTrackContext,
        result: api::GuardianReviewAnalyticsResult,
        completed_at_ms: u64,
    ) {
        self.record_fact(facts::AnalyticsFact::Custom(
            facts::CustomAnalyticsFact::GuardianReview(Box::new(
                tracking.event_params(result, completed_at_ms).into(),
            )),
        ));
    }

    fn track_app_mentioned(
        &self,
        tracking: api::TrackEventsContext,
        mentions: Vec<api::AppInvocation>,
    ) {
        self.track_app_mentioned(
            tracking.into(),
            mentions.into_iter().map(Into::into).collect(),
        );
    }

    fn track_app_used(&self, tracking: api::TrackEventsContext, app: api::AppInvocation) {
        self.track_app_used(tracking.into(), app.into());
    }

    fn track_hook_run(&self, tracking: api::TrackEventsContext, hook: api::HookRunFact) {
        self.track_hook_run(tracking.into(), hook.into());
    }

    fn track_plugin_used(
        &self,
        tracking: api::TrackEventsContext,
        plugin: plugin_service_api::PluginTelemetryMetadata,
    ) {
        self.track_plugin_used(tracking.into(), plugin);
    }

    fn track_compaction(&self, event: api::CodexCompactionEvent) {
        self.track_compaction(event.into());
    }

    fn track_turn_resolved_config(&self, fact: api::TurnResolvedConfigFact) {
        self.track_turn_resolved_config(fact.into());
    }

    fn track_turn_token_usage(&self, fact: api::TurnTokenUsageFact) {
        self.track_turn_token_usage(fact.into());
    }
}

impl From<api::TrackEventsContext> for facts::TrackEventsContext {
    fn from(value: api::TrackEventsContext) -> Self {
        Self {
            model_slug: value.model_slug,
            thread_id: value.thread_id,
            turn_id: value.turn_id,
        }
    }
}

impl From<api::TurnSubmissionType> for facts::TurnSubmissionType {
    fn from(value: api::TurnSubmissionType) -> Self {
        match value {
            api::TurnSubmissionType::Default => Self::Default,
            api::TurnSubmissionType::Queued => Self::Queued,
        }
    }
}

impl From<api::TurnResolvedConfigFact> for facts::TurnResolvedConfigFact {
    fn from(value: api::TurnResolvedConfigFact) -> Self {
        Self {
            turn_id: value.turn_id,
            thread_id: value.thread_id,
            num_input_images: value.num_input_images,
            submission_type: value.submission_type.map(Into::into),
            ephemeral: value.ephemeral,
            session_source: value.session_source,
            model: value.model,
            model_provider: value.model_provider,
            permission_profile: value.permission_profile,
            permission_profile_cwd: value.permission_profile_cwd,
            reasoning_effort: value.reasoning_effort,
            reasoning_summary: value.reasoning_summary,
            service_tier: value.service_tier,
            approval_policy: value.approval_policy,
            approvals_reviewer: value.approvals_reviewer,
            sandbox_network_access: value.sandbox_network_access,
            collaboration_mode: value.collaboration_mode,
            personality: value.personality,
            is_first_turn: value.is_first_turn,
        }
    }
}

impl From<api::TurnTokenUsageFact> for facts::TurnTokenUsageFact {
    fn from(value: api::TurnTokenUsageFact) -> Self {
        Self {
            turn_id: value.turn_id,
            thread_id: value.thread_id,
            token_usage: value.token_usage,
        }
    }
}

impl From<api::SkillInvocation> for facts::SkillInvocation {
    fn from(value: api::SkillInvocation) -> Self {
        Self {
            skill_name: value.skill_name,
            skill_scope: value.skill_scope,
            skill_path: value.skill_path,
            plugin_id: value.plugin_id,
            invocation_type: value.invocation_type.into(),
        }
    }
}

impl From<api::InvocationType> for facts::InvocationType {
    fn from(value: api::InvocationType) -> Self {
        match value {
            api::InvocationType::Explicit => Self::Explicit,
            api::InvocationType::Implicit => Self::Implicit,
        }
    }
}

impl From<api::AppInvocation> for facts::AppInvocation {
    fn from(value: api::AppInvocation) -> Self {
        Self {
            connector_id: value.connector_id,
            app_name: value.app_name,
            invocation_type: value.invocation_type.map(Into::into),
        }
    }
}

impl From<api::SubAgentThreadStartedInput> for facts::SubAgentThreadStartedInput {
    fn from(value: api::SubAgentThreadStartedInput) -> Self {
        Self {
            thread_id: value.thread_id,
            parent_thread_id: value.parent_thread_id,
            product_client_id: value.product_client_id,
            client_name: value.client_name,
            client_version: value.client_version,
            model: value.model,
            ephemeral: value.ephemeral,
            subagent_source: value.subagent_source,
            created_at: value.created_at,
        }
    }
}

impl From<api::CompactionTrigger> for facts::CompactionTrigger {
    fn from(value: api::CompactionTrigger) -> Self {
        match value {
            api::CompactionTrigger::Manual => Self::Manual,
            api::CompactionTrigger::Auto => Self::Auto,
        }
    }
}

impl From<api::CompactionReason> for facts::CompactionReason {
    fn from(value: api::CompactionReason) -> Self {
        match value {
            api::CompactionReason::UserRequested => Self::UserRequested,
            api::CompactionReason::ContextLimit => Self::ContextLimit,
            api::CompactionReason::ModelDownshift => Self::ModelDownshift,
        }
    }
}

impl From<api::CompactionImplementation> for facts::CompactionImplementation {
    fn from(value: api::CompactionImplementation) -> Self {
        match value {
            api::CompactionImplementation::Responses => Self::Responses,
            api::CompactionImplementation::ResponsesCompact => Self::ResponsesCompact,
        }
    }
}

impl From<api::CompactionPhase> for facts::CompactionPhase {
    fn from(value: api::CompactionPhase) -> Self {
        match value {
            api::CompactionPhase::StandaloneTurn => Self::StandaloneTurn,
            api::CompactionPhase::PreTurn => Self::PreTurn,
            api::CompactionPhase::MidTurn => Self::MidTurn,
        }
    }
}

impl From<api::CompactionStrategy> for facts::CompactionStrategy {
    fn from(value: api::CompactionStrategy) -> Self {
        match value {
            api::CompactionStrategy::Memento => Self::Memento,
            api::CompactionStrategy::PrefixCompaction => Self::PrefixCompaction,
        }
    }
}

impl From<api::CompactionStatus> for facts::CompactionStatus {
    fn from(value: api::CompactionStatus) -> Self {
        match value {
            api::CompactionStatus::Completed => Self::Completed,
            api::CompactionStatus::Failed => Self::Failed,
            api::CompactionStatus::Interrupted => Self::Interrupted,
        }
    }
}

impl From<api::CodexCompactionEvent> for facts::CodexCompactionEvent {
    fn from(value: api::CodexCompactionEvent) -> Self {
        Self {
            thread_id: value.thread_id,
            turn_id: value.turn_id,
            trigger: value.trigger.into(),
            reason: value.reason.into(),
            implementation: value.implementation.into(),
            phase: value.phase.into(),
            strategy: value.strategy.into(),
            status: value.status.into(),
            error: value.error,
            active_context_tokens_before: value.active_context_tokens_before,
            active_context_tokens_after: value.active_context_tokens_after,
            started_at: value.started_at,
            completed_at: value.completed_at,
            duration_ms: value.duration_ms,
        }
    }
}

impl From<api::HookRunFact> for facts::HookRunFact {
    fn from(value: api::HookRunFact) -> Self {
        Self {
            event_name: value.event_name,
            hook_source: value.hook_source,
            status: value.status,
        }
    }
}

impl From<api::GuardianReviewDecision> for events::GuardianReviewDecision {
    fn from(value: api::GuardianReviewDecision) -> Self {
        match value {
            api::GuardianReviewDecision::Approved => Self::Approved,
            api::GuardianReviewDecision::Denied => Self::Denied,
            api::GuardianReviewDecision::Aborted => Self::Aborted,
        }
    }
}

impl From<api::GuardianReviewTerminalStatus> for events::GuardianReviewTerminalStatus {
    fn from(value: api::GuardianReviewTerminalStatus) -> Self {
        match value {
            api::GuardianReviewTerminalStatus::Approved => Self::Approved,
            api::GuardianReviewTerminalStatus::Denied => Self::Denied,
            api::GuardianReviewTerminalStatus::Aborted => Self::Aborted,
            api::GuardianReviewTerminalStatus::TimedOut => Self::TimedOut,
            api::GuardianReviewTerminalStatus::FailedClosed => Self::FailedClosed,
        }
    }
}

impl From<api::GuardianReviewFailureReason> for events::GuardianReviewFailureReason {
    fn from(value: api::GuardianReviewFailureReason) -> Self {
        match value {
            api::GuardianReviewFailureReason::Timeout => Self::Timeout,
            api::GuardianReviewFailureReason::Cancelled => Self::Cancelled,
            api::GuardianReviewFailureReason::PromptBuildError => Self::PromptBuildError,
            api::GuardianReviewFailureReason::SessionError => Self::SessionError,
            api::GuardianReviewFailureReason::ParseError => Self::ParseError,
        }
    }
}

impl From<api::GuardianReviewSessionKind> for events::GuardianReviewSessionKind {
    fn from(value: api::GuardianReviewSessionKind) -> Self {
        match value {
            api::GuardianReviewSessionKind::TrunkNew => Self::TrunkNew,
            api::GuardianReviewSessionKind::TrunkReused => Self::TrunkReused,
            api::GuardianReviewSessionKind::EphemeralForked => Self::EphemeralForked,
        }
    }
}

impl From<api::GuardianApprovalRequestSource> for events::GuardianApprovalRequestSource {
    fn from(value: api::GuardianApprovalRequestSource) -> Self {
        match value {
            api::GuardianApprovalRequestSource::MainTurn => Self::MainTurn,
            api::GuardianApprovalRequestSource::DelegatedSubagent => Self::DelegatedSubagent,
        }
    }
}

impl From<api::GuardianReviewedAction> for events::GuardianReviewedAction {
    fn from(value: api::GuardianReviewedAction) -> Self {
        match value {
            api::GuardianReviewedAction::Shell {
                sandbox_permissions,
                additional_permissions,
            } => Self::Shell {
                sandbox_permissions,
                additional_permissions,
            },
            api::GuardianReviewedAction::UnifiedExec {
                sandbox_permissions,
                additional_permissions,
                tty,
            } => Self::UnifiedExec {
                sandbox_permissions,
                additional_permissions,
                tty,
            },
            api::GuardianReviewedAction::Execve {
                source,
                program,
                additional_permissions,
            } => Self::Execve {
                source,
                program,
                additional_permissions,
            },
            api::GuardianReviewedAction::ApplyPatch {} => Self::ApplyPatch {},
            api::GuardianReviewedAction::NetworkAccess { protocol, port } => {
                Self::NetworkAccess { protocol, port }
            }
            api::GuardianReviewedAction::McpToolCall {
                server,
                tool_name,
                connector_id,
                connector_name,
                tool_title,
            } => Self::McpToolCall {
                server,
                tool_name,
                connector_id,
                connector_name,
                tool_title,
            },
            api::GuardianReviewedAction::RequestPermissions {} => Self::RequestPermissions {},
        }
    }
}

impl From<api::GuardianReviewEventParams> for events::GuardianReviewEventParams {
    fn from(value: api::GuardianReviewEventParams) -> Self {
        Self {
            thread_id: value.thread_id,
            turn_id: value.turn_id,
            review_id: value.review_id,
            target_item_id: value.target_item_id,
            approval_request_source: value.approval_request_source.into(),
            reviewed_action: value.reviewed_action.into(),
            reviewed_action_truncated: value.reviewed_action_truncated,
            decision: value.decision.into(),
            terminal_status: value.terminal_status.into(),
            failure_reason: value.failure_reason.map(Into::into),
            risk_level: value.risk_level,
            user_authorization: value.user_authorization,
            outcome: value.outcome,
            guardian_thread_id: value.guardian_thread_id,
            guardian_session_kind: value.guardian_session_kind.map(Into::into),
            guardian_model: value.guardian_model,
            guardian_reasoning_effort: value.guardian_reasoning_effort,
            had_prior_review_context: value.had_prior_review_context,
            review_timeout_ms: value.review_timeout_ms,
            tool_call_count: value.tool_call_count,
            time_to_first_token_ms: value.time_to_first_token_ms,
            completion_latency_ms: value.completion_latency_ms,
            started_at: value.started_at,
            completed_at: value.completed_at,
            input_tokens: value.input_tokens,
            cached_input_tokens: value.cached_input_tokens,
            output_tokens: value.output_tokens,
            reasoning_output_tokens: value.reasoning_output_tokens,
            total_tokens: value.total_tokens,
        }
    }
}
