//! Thread targeting helpers for app-server requests and notifications.

use app_server_protocol::ServerNotification;
use app_server_protocol::ServerRequest;
use protocol::ThreadId;

pub(super) fn server_request_thread_id(request: &ServerRequest) -> Option<ThreadId> {
    match request {
        ServerRequest::CommandExecutionRequestApproval { params, .. } => {
            ThreadId::from_string(&params.thread_id).ok()
        }
        ServerRequest::FileChangeRequestApproval { params, .. } => {
            ThreadId::from_string(&params.thread_id).ok()
        }
        ServerRequest::ToolRequestUserInput { params, .. } => {
            ThreadId::from_string(&params.thread_id).ok()
        }
        ServerRequest::McpServerElicitationRequest { params, .. } => {
            ThreadId::from_string(&params.thread_id).ok()
        }
        ServerRequest::PermissionsRequestApproval { params, .. } => {
            ThreadId::from_string(&params.thread_id).ok()
        }
        ServerRequest::DynamicToolCall { params, .. } => {
            ThreadId::from_string(&params.thread_id).ok()
        }
        ServerRequest::ChatgptAuthTokensRefresh { .. }
        | ServerRequest::AttestationGenerate { .. }
        | ServerRequest::ApplyPatchApproval { .. }
        | ServerRequest::ExecCommandApproval { .. } => None,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ServerNotificationThreadTarget {
    Thread(ThreadId),
    InvalidThreadId(String),
    Global,
}

pub(super) fn server_notification_thread_target(
    notification: &ServerNotification,
) -> ServerNotificationThreadTarget {
    let thread_id = match notification {
        ServerNotification::Error(notification) => Some(notification.thread_id.as_str()),
        ServerNotification::ThreadStarted(notification) => Some(notification.thread.id.as_str()),
        ServerNotification::ThreadStatusChanged(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::ThreadArchived(notification) => Some(notification.thread_id.as_str()),
        ServerNotification::ThreadUnarchived(notification) => Some(notification.thread_id.as_str()),
        ServerNotification::ThreadClosed(notification) => Some(notification.thread_id.as_str()),
        ServerNotification::ThreadNameUpdated(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::ThreadTokenUsageUpdated(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::ThreadContextUsageUpdated(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::ThreadSkillsUpdated(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::ThreadGoalUpdated(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::ThreadGoalCleared(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::WorkflowRunUpdated(_) => None,
        ServerNotification::TurnStarted(notification) => Some(notification.thread_id.as_str()),
        ServerNotification::HookStarted(notification) => Some(notification.thread_id.as_str()),
        ServerNotification::TurnCompleted(notification) => Some(notification.thread_id.as_str()),
        ServerNotification::HookCompleted(notification) => Some(notification.thread_id.as_str()),
        ServerNotification::TurnDiffUpdated(notification) => Some(notification.thread_id.as_str()),
        ServerNotification::TurnPlanUpdated(notification) => Some(notification.thread_id.as_str()),
        ServerNotification::ItemStarted(notification) => Some(notification.thread_id.as_str()),
        ServerNotification::ItemGuardianApprovalReviewStarted(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::ItemGuardianApprovalReviewCompleted(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::ItemCompleted(notification) => Some(notification.thread_id.as_str()),
        ServerNotification::AgentMessageDelta(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::PlanDelta(notification) => Some(notification.thread_id.as_str()),
        ServerNotification::CommandExecutionOutputDelta(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::TerminalInteraction(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::FileChangeOutputDelta(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::FileChangePatchUpdated(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::ServerRequestResolved(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::McpToolCallProgress(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::ReasoningSummaryTextDelta(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::ReasoningSummaryPartAdded(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::ReasoningTextDelta(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::ContextCompacted(notification) => Some(notification.thread_id.as_str()),
        ServerNotification::ModelRerouted(notification) => Some(notification.thread_id.as_str()),
        ServerNotification::ModelVerification(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::ThreadRealtimeStarted(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::ThreadRealtimeItemAdded(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::ThreadRealtimeTranscriptDelta(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::ThreadRealtimeTranscriptDone(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::ThreadRealtimeOutputAudioDelta(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::ThreadRealtimeSdp(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::ThreadRealtimeError(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::ThreadRealtimeClosed(notification) => {
            Some(notification.thread_id.as_str())
        }
        ServerNotification::Warning(notification) => notification.thread_id.as_deref(),
        ServerNotification::GuardianWarning(notification) => Some(notification.thread_id.as_str()),
        ServerNotification::SkillsChanged(_)
        | ServerNotification::McpServerStatusUpdated(_)
        | ServerNotification::McpServerOauthLoginCompleted(_)
        | ServerNotification::AccountUpdated(_)
        | ServerNotification::AccountRateLimitsUpdated(_)
        | ServerNotification::AppListUpdated(_)
        | ServerNotification::RemoteControlStatusChanged(_)
        | ServerNotification::ExternalAgentConfigImportCompleted(_)
        | ServerNotification::DeprecationNotice(_)
        | ServerNotification::ConfigWarning(_)
        | ServerNotification::FuzzyFileSearchSessionUpdated(_)
        | ServerNotification::FuzzyFileSearchSessionCompleted(_)
        | ServerNotification::CommandExecOutputDelta(_)
        | ServerNotification::ProcessOutputDelta(_)
        | ServerNotification::ProcessExited(_)
        | ServerNotification::FsChanged(_)
        | ServerNotification::WindowsWorldWritableWarning(_)
        | ServerNotification::WindowsSandboxSetupCompleted(_)
        | ServerNotification::AccountLoginCompleted(_) => None,
    };

    match thread_id {
        Some(thread_id) => match ThreadId::from_string(thread_id) {
            Ok(thread_id) => ServerNotificationThreadTarget::Thread(thread_id),
            Err(_) => ServerNotificationThreadTarget::InvalidThreadId(thread_id.to_string()),
        },
        None => ServerNotificationThreadTarget::Global,
    }
}

#[cfg(test)]
mod tests {
    use super::ServerNotificationThreadTarget;
    use super::server_notification_thread_target;
    use app_server_protocol::GuardianWarningNotification;
    use app_server_protocol::ServerNotification;
    use app_server_protocol::WarningNotification;
    use app_server_protocol::WorkflowRun;
    use app_server_protocol::WorkflowRunStatus;
    use app_server_protocol::WorkflowRunUpdatedNotification;
    use app_server_protocol::WorkflowSource;
    use app_server_protocol::WorkflowSummary;
    use pretty_assertions::assert_eq;
    use protocol::ThreadId;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn warning_notifications_without_threads_are_global() {
        let notification = ServerNotification::Warning(WarningNotification {
            thread_id: None,
            message: "warning".to_string(),
        });

        let target = server_notification_thread_target(&notification);

        assert_eq!(target, ServerNotificationThreadTarget::Global);
    }

    #[test]
    fn warning_notifications_route_to_threads_when_thread_id_is_present() {
        let thread_id = ThreadId::new();
        let notification = ServerNotification::Warning(WarningNotification {
            thread_id: Some(thread_id.to_string()),
            message: "warning".to_string(),
        });

        let target = server_notification_thread_target(&notification);

        assert_eq!(target, ServerNotificationThreadTarget::Thread(thread_id));
    }

    #[test]
    fn guardian_warning_notifications_route_to_threads() {
        let thread_id = ThreadId::new();
        let notification = ServerNotification::GuardianWarning(GuardianWarningNotification {
            thread_id: thread_id.to_string(),
            message: "warning".to_string(),
        });

        let target = server_notification_thread_target(&notification);

        assert_eq!(target, ServerNotificationThreadTarget::Thread(thread_id));
    }

    #[test]
    fn workflow_run_updated_notifications_are_global() {
        let notification = ServerNotification::WorkflowRunUpdated(WorkflowRunUpdatedNotification {
            run: WorkflowRun {
                run_id: "wf_run_1".to_string(),
                workflow: WorkflowSummary {
                    id: "feature-dev".to_string(),
                    name: "Feature Development".to_string(),
                    description: "按调研、实现、review/fix、验证流程开发功能".to_string(),
                    source: WorkflowSource::Project,
                    path: ".codex/workflows/feature-dev".to_string(),
                    entry: "workflow.ts".to_string(),
                    version: Some("0.1.0".to_string()),
                    when_to_use: Vec::new(),
                    inputs: BTreeMap::new(),
                },
                status: WorkflowRunStatus::Running,
                runner_status: "control_plane_started".to_string(),
                inputs: json!({}),
                created_at: 1,
                updated_at: 1,
                revision: 1,
                message: "started".to_string(),
                abort_reason: None,
                output: None,
                error: None,
                snapshot_path: None,
            },
        });

        let target = server_notification_thread_target(&notification);

        assert_eq!(target, ServerNotificationThreadTarget::Global);
    }
}
