use super::*;
use crate::guardian::GuardianNetworkAccessTrigger;
use crate::tool_approval_support::ToolError;
use codex_permissions_runtime::NetworkApprovalOutcome;
use codex_protocol::models::SandboxPermissions;
use core_test_support::test_path_buf;
use core_test_support::PathBufExt;
use pretty_assertions::assert_eq;
use std::sync::Arc;
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;

fn default_shell_trigger() -> GuardianNetworkAccessTrigger {
    GuardianNetworkAccessTrigger {
        call_id: "call-1".to_string(),
        tool_name: "exec_command".to_string(),
        command: vec!["curl".to_string(), "https://example.com".to_string()],
        cwd: test_path_buf("/tmp").abs(),
        sandbox_permissions: SandboxPermissions::UseDefault,
        additional_permissions: None,
        justification: None,
        tty: None,
    }
}

async fn register_call_with_default_shell_trigger(
    service: &NetworkApprovalService,
    registration_id: &str,
) -> CancellationToken {
    let cancellation_token = CancellationToken::new();
    service
        .register_call(
            registration_id.to_string(),
            "turn-1".to_string(),
            default_shell_trigger(),
            "curl https://example.com".to_string(),
            cancellation_token.clone(),
        )
        .await;
    cancellation_token
}

#[tokio::test]
async fn active_call_preserves_triggering_command_context() {
    let service = NetworkApprovalService::default();
    let expected = GuardianNetworkAccessTrigger {
        call_id: "call-1".to_string(),
        tool_name: "exec_command".to_string(),
        command: vec!["curl".to_string(), "https://example.com".to_string()],
        cwd: test_path_buf("/repo").abs(),
        sandbox_permissions: SandboxPermissions::UseDefault,
        additional_permissions: None,
        justification: Some("fetch release metadata".to_string()),
        tty: None,
    };

    service
        .register_call(
            "registration-1".to_string(),
            "turn-1".to_string(),
            expected.clone(),
            "curl https://example.com".to_string(),
            CancellationToken::new(),
        )
        .await;

    let call = service
        .resolve_single_active_call()
        .await
        .expect("single active call should resolve");

    assert_eq!(&call.trigger, &expected);
    assert_eq!(call.command, "curl https://example.com");
}

#[tokio::test]
async fn finish_call_returns_denial_and_unregisters_active_call() {
    let service = NetworkApprovalService::default();
    register_call_with_default_shell_trigger(&service, "registration-1").await;

    service
        .record_call_outcome(
            "registration-1",
            NetworkApprovalOutcome::DeniedByPolicy("network denied".to_string()),
        )
        .await;

    let err = service
        .finish_call("registration-1")
        .await
        .expect_err("denial should be returned");

    assert!(matches!(err, ToolError::Rejected(message) if message == "network denied"));
    assert!(service.resolve_single_active_call().await.is_none());
}

#[tokio::test]
async fn deferred_finish_reuses_denial_result_after_first_consumer() {
    let service = NetworkApprovalService::default();
    let cancellation_token =
        register_call_with_default_shell_trigger(&service, "registration-1").await;
    let deferred = DeferredNetworkApproval {
        registration_id: "registration-1".to_string(),
        cancellation_token,
        finish_outcome: Arc::new(OnceCell::new()),
    };
    service
        .record_call_outcome(
            "registration-1",
            NetworkApprovalOutcome::DeniedByPolicy("network denied".to_string()),
        )
        .await;

    let first = deferred
        .finish(&service)
        .await
        .expect_err("first consumer should see denial");
    let second = deferred
        .finish(&service)
        .await
        .expect_err("second consumer should reuse denial");

    assert!(matches!(first, ToolError::Rejected(message) if message == "network denied"));
    assert!(matches!(second, ToolError::Rejected(message) if message == "network denied"));
}
