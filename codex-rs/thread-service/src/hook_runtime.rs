use std::path::PathBuf;

use codex_analytics_api::HookRunFact;
use codex_analytics_api::build_track_events_context;
use codex_utils_absolute_path::AbsolutePathBuf;
use hooks::HookRuntimeHost;
use hooks::HookRuntimeTurn;
use hooks_api::SharedHookRuntime;
use metrics_api::HOOK_RUN_DURATION_METRIC;
use metrics_api::HOOK_RUN_METRIC;
use protocol::models::ResponseItem;
use protocol::protocol::AskForApproval;
use protocol::protocol::EventMsg;
use protocol::protocol::HookCompletedEvent;
use protocol::protocol::HookEventName;
use protocol::protocol::HookRunStatus;
use protocol::protocol::HookRunSummary;
use protocol::protocol::HookSource;
use protocol::protocol::HookStartedEvent;
use protocol::user_input::UserInput;

use crate::session::session::Session;
use crate::session::turn_context::TurnContext;

impl HookRuntimeTurn for TurnContext {
    fn turn_id(&self) -> &str {
        &self.sub_id
    }

    fn cwd(&self) -> &AbsolutePathBuf {
        #[allow(deprecated)]
        &self.cwd
    }

    fn model_slug(&self) -> &str {
        &self.model_info.slug
    }

    fn approval_policy(&self) -> AskForApproval {
        self.approval_policy.value()
    }
}

impl HookRuntimeHost for Session {
    type Turn = TurnContext;

    fn session_id(&self) -> protocol::ThreadId {
        Session::session_id(self).into()
    }

    fn hooks(&self) -> SharedHookRuntime {
        Session::hooks(self)
    }

    async fn take_pending_session_start_source(&self) -> Option<hooks_api::SessionStartSource> {
        Session::take_pending_session_start_source(self).await
    }

    async fn hook_transcript_path(&self) -> Option<PathBuf> {
        Session::hook_transcript_path(self).await
    }

    async fn emit_hook_started(&self, turn: &Self::Turn, run: HookRunSummary) {
        self.send_event(
            turn,
            EventMsg::HookStarted(HookStartedEvent {
                turn_id: Some(turn.sub_id.clone()),
                run,
            }),
        )
        .await;
    }

    async fn emit_hook_completed(&self, turn: &Self::Turn, completed: HookCompletedEvent) {
        emit_hook_completed_metrics(turn, &completed);
        track_hook_completed_analytics(self, turn, &completed);
        self.send_event(turn, EventMsg::HookCompleted(completed))
            .await;
    }

    async fn record_conversation_items(&self, turn: &Self::Turn, items: Vec<ResponseItem>) {
        Session::record_conversation_items(self, turn, items.as_slice()).await;
    }

    async fn record_model_items_and_emit_display_events(
        &self,
        turn: &Self::Turn,
        items: Vec<ResponseItem>,
    ) {
        Session::record_model_items_and_emit_display_events(self, turn, items.as_slice()).await;
    }

    async fn record_user_prompt_and_emit_turn_item(
        &self,
        turn: &Self::Turn,
        content: Vec<UserInput>,
        response_item: ResponseItem,
    ) {
        Session::record_user_prompt_and_emit_turn_item(
            self,
            turn,
            content.as_slice(),
            response_item,
        )
        .await;
    }
}

fn emit_hook_completed_metrics(turn_context: &TurnContext, completed: &HookCompletedEvent) {
    let tags = hook_run_metric_tags(&completed.run);
    turn_context
        .session_telemetry
        .counter(HOOK_RUN_METRIC, /*inc*/ 1, &tags);
    if let Some(duration_ms) = completed.run.duration_ms
        && let Ok(duration_ms) = u64::try_from(duration_ms)
    {
        turn_context.session_telemetry.record_duration(
            HOOK_RUN_DURATION_METRIC,
            std::time::Duration::from_millis(duration_ms),
            &tags,
        );
    }
}

fn track_hook_completed_analytics(
    sess: &Session,
    turn_context: &TurnContext,
    completed: &HookCompletedEvent,
) {
    let (tracking, hook) =
        hook_run_analytics_payload(sess.conversation_id.to_string(), turn_context, completed);
    sess.services
        .analytics_events_client
        .track_hook_run(tracking, hook);
}

fn hook_run_analytics_payload(
    thread_id: String,
    turn_context: &TurnContext,
    completed: &HookCompletedEvent,
) -> (codex_analytics_api::TrackEventsContext, HookRunFact) {
    (
        build_track_events_context(
            turn_context.model_info.slug.clone(),
            thread_id,
            completed
                .turn_id
                .clone()
                .unwrap_or_else(|| turn_context.sub_id.clone()),
        ),
        HookRunFact {
            event_name: completed.run.event_name,
            hook_source: completed.run.source,
            status: completed.run.status,
        },
    )
}

fn hook_run_metric_tags(run: &HookRunSummary) -> [(&'static str, &'static str); 3] {
    let hook_name = match run.event_name {
        HookEventName::PreToolUse => "PreToolUse",
        HookEventName::PermissionRequest => "PermissionRequest",
        HookEventName::PostToolUse => "PostToolUse",
        HookEventName::PreCompact => "PreCompact",
        HookEventName::PostCompact => "PostCompact",
        HookEventName::SessionStart => "SessionStart",
        HookEventName::UserPromptSubmit => "UserPromptSubmit",
        HookEventName::Stop => "Stop",
    };
    let hook_source = match run.source {
        HookSource::System => "system",
        HookSource::User => "user",
        HookSource::Project => "project",
        HookSource::Mdm => "mdm",
        HookSource::SessionFlags => "session_flags",
        HookSource::Plugin => "plugin",
        HookSource::CloudRequirements => "cloud_requirements",
        HookSource::LegacyManagedConfigFile => "legacy_managed_config_file",
        HookSource::LegacyManagedConfigMdm => "legacy_managed_config_mdm",
        HookSource::Unknown => "unknown",
    };
    let status = match run.status {
        HookRunStatus::Running => "running",
        HookRunStatus::Completed => "completed",
        HookRunStatus::Failed => "failed",
        HookRunStatus::Blocked => "blocked",
        HookRunStatus::Stopped => "stopped",
    };

    [
        ("hook_name", hook_name),
        ("source", hook_source),
        ("status", status),
    ]
}

#[cfg(test)]
mod tests {
    use codex_utils_absolute_path::test_support::PathBufExt;
    use codex_utils_absolute_path::test_support::test_path_buf;
    use pretty_assertions::assert_eq;
    use protocol::protocol::HookCompletedEvent;
    use protocol::protocol::HookEventName;
    use protocol::protocol::HookExecutionMode;
    use protocol::protocol::HookHandlerType;
    use protocol::protocol::HookRunStatus;
    use protocol::protocol::HookRunSummary;
    use protocol::protocol::HookScope;
    use protocol::protocol::HookSource;

    use super::hook_run_analytics_payload;
    use super::hook_run_metric_tags;
    use crate::session::tests::make_session_and_context;

    #[tokio::test]
    async fn hook_run_analytics_payload_uses_completed_turn_id() {
        let (_session, turn_context) = make_session_and_context().await;
        let completed = HookCompletedEvent {
            turn_id: Some("turn-from-hook".to_string()),
            run: sample_hook_run(HookRunStatus::Blocked, HookSource::Project),
        };

        let (tracking, hook) =
            hook_run_analytics_payload("thread-123".to_string(), &turn_context, &completed);

        assert_eq!(tracking.thread_id, "thread-123");
        assert_eq!(tracking.turn_id, "turn-from-hook");
        assert_eq!(tracking.model_slug, turn_context.model_info.slug);
        assert_eq!(hook.event_name, HookEventName::Stop);
        assert_eq!(hook.hook_source, HookSource::Project);
        assert_eq!(hook.status, HookRunStatus::Blocked);
    }

    #[tokio::test]
    async fn hook_run_analytics_payload_falls_back_to_turn_context_id() {
        let (_session, turn_context) = make_session_and_context().await;
        let completed = HookCompletedEvent {
            turn_id: None,
            run: sample_hook_run(HookRunStatus::Failed, HookSource::Unknown),
        };

        let (tracking, hook) =
            hook_run_analytics_payload("thread-123".to_string(), &turn_context, &completed);

        assert_eq!(tracking.turn_id, turn_context.sub_id);
        assert_eq!(hook.hook_source, HookSource::Unknown);
        assert_eq!(hook.status, HookRunStatus::Failed);
    }

    #[test]
    fn hook_run_metric_tags_match_analytics_shape() {
        let run = sample_hook_run(HookRunStatus::Blocked, HookSource::Project);

        assert_eq!(
            hook_run_metric_tags(&run),
            [
                ("hook_name", "Stop"),
                ("source", "project"),
                ("status", "blocked"),
            ]
        );

        let cloud_requirements =
            sample_hook_run(HookRunStatus::Blocked, HookSource::CloudRequirements);

        assert_eq!(
            hook_run_metric_tags(&cloud_requirements),
            [
                ("hook_name", "Stop"),
                ("source", "cloud_requirements"),
                ("status", "blocked"),
            ]
        );
    }

    #[test]
    fn hook_run_metric_tags_include_expanded_hook_sources() {
        let run = sample_hook_run(HookRunStatus::Completed, HookSource::LegacyManagedConfigMdm);

        assert_eq!(
            hook_run_metric_tags(&run),
            [
                ("hook_name", "Stop"),
                ("source", "legacy_managed_config_mdm"),
                ("status", "completed"),
            ]
        );
    }

    fn sample_hook_run(status: HookRunStatus, source: HookSource) -> HookRunSummary {
        HookRunSummary {
            id: "stop:0:/tmp/hooks.json".to_string(),
            event_name: HookEventName::Stop,
            handler_type: HookHandlerType::Command,
            execution_mode: HookExecutionMode::Sync,
            scope: HookScope::Turn,
            source_path: test_path_buf("/tmp/hooks.json").abs(),
            source,
            display_order: 0,
            status,
            status_message: None,
            started_at: 10,
            completed_at: Some(37),
            duration_ms: Some(27),
            entries: Vec::new(),
        }
    }
}
