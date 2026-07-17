use std::sync::Arc;

use tokio::sync::Mutex;

use super::CommandNotificationFilter;
use super::HeadTailBuffer;

#[tokio::test]
async fn output_notify_exit_uses_only_unnotified_residual_output() {
    let transcript = Arc::new(Mutex::new(HeadTailBuffer::new(10)));
    let exit_notification_output = Arc::new(Mutex::new(HeadTailBuffer::new(10)));
    {
        let mut guard = transcript.lock().await;
        guard.push_chunk(b"already-notified".to_vec());
        guard.push_chunk(b"residual".to_vec());
    }
    {
        let mut guard = exit_notification_output.lock().await;
        guard.push_chunk(b"residual".to_vec());
    }

    let output = super::resolve_exit_notification_output(
        &transcript,
        &exit_notification_output,
        CommandNotificationFilter::Output,
        None,
    )
    .await
    .expect("residual output should produce notification output");

    assert_eq!(output, "residual");
}

#[tokio::test]
async fn exit_notify_exit_uses_bounded_full_transcript() {
    let transcript = Arc::new(Mutex::new(HeadTailBuffer::new(10)));
    let exit_notification_output = Arc::new(Mutex::new(HeadTailBuffer::default()));
    {
        let mut guard = transcript.lock().await;
        guard.push_chunk(b"0123456789".to_vec());
        guard.push_chunk(b"abcdef".to_vec());
    }

    let output = super::resolve_exit_notification_output(
        &transcript,
        &exit_notification_output,
        CommandNotificationFilter::Exit,
        None,
    )
    .await
    .expect("non-empty transcript should produce notification output");

    assert_eq!(output.len(), 10);
    assert!(output.starts_with("01234"));
    assert!(output.ends_with("bcdef"));
}

#[tokio::test]
async fn exit_notification_output_is_none_for_empty_transcript() {
    let transcript = Arc::new(Mutex::new(HeadTailBuffer::default()));
    let exit_notification_output = Arc::new(Mutex::new(HeadTailBuffer::default()));

    let output = super::resolve_exit_notification_output(
        &transcript,
        &exit_notification_output,
        CommandNotificationFilter::Exit,
        None,
    )
    .await;

    assert_eq!(output, None);
}

#[tokio::test]
async fn failed_exit_notification_output_includes_failure_message() {
    let transcript = Arc::new(Mutex::new(HeadTailBuffer::new(32)));
    let exit_notification_output = Arc::new(Mutex::new(HeadTailBuffer::default()));
    {
        let mut guard = transcript.lock().await;
        guard.push_chunk(b"stdout before failure".to_vec());
    }

    let output = super::resolve_exit_notification_output(
        &transcript,
        &exit_notification_output,
        CommandNotificationFilter::Exit,
        Some("spawn failed"),
    )
    .await
    .expect("failure message should produce notification output");

    assert_eq!(output, "stdout before failure\nspawn failed");
}

#[tokio::test]
async fn failed_exit_notification_output_uses_failure_message_when_stdout_is_empty() {
    let transcript = Arc::new(Mutex::new(HeadTailBuffer::default()));
    let exit_notification_output = Arc::new(Mutex::new(HeadTailBuffer::default()));

    let output = super::resolve_exit_notification_output(
        &transcript,
        &exit_notification_output,
        CommandNotificationFilter::Exit,
        Some("spawn failed"),
    )
    .await
    .expect("failure message should produce notification output");

    assert_eq!(output, "spawn failed");
}
