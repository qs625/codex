use std::sync::Arc;

use tokio::sync::Mutex;

use super::CommandNotificationFilter;
use super::HeadTailBuffer;
use super::MAX_OUTPUT_NOTIFICATION_BYTES;
use super::OutputNotificationAggregator;
use protocol::models::CommandExecutionNotificationKind;
use protocol::models::ResponseItem;

#[test]
fn output_notification_aggregator_combines_fast_chunks() {
    let mut aggregator = OutputNotificationAggregator::default();

    aggregator.push(1, "first".to_string());
    aggregator.push(2, "second".to_string());

    let item = aggregator
        .take_item("call-1")
        .expect("pending output should flush");
    let ResponseItem::CommandExecutionNotification {
        id,
        command_item_id,
        kind,
        message,
        output,
        exit_code,
        ..
    } = item
    else {
        panic!("expected output notification item");
    };
    assert_eq!(id, Some("call-1:notification:output:1-2".to_string()));
    assert_eq!(command_item_id, "call-1");
    assert_eq!(kind, CommandExecutionNotificationKind::Output);
    assert_eq!(message, "Command call-1 produced new output.");
    assert_eq!(output, Some("firstsecond".to_string()));
    assert_eq!(exit_code, None);
    assert!(
        aggregator.take_item("call-1").is_none(),
        "flush should clear pending output"
    );
}

#[test]
fn output_notification_aggregator_keeps_single_sequence_ids() {
    let mut aggregator = OutputNotificationAggregator::default();

    aggregator.push(7, "only".to_string());

    let item = aggregator
        .take_item("call-1")
        .expect("pending output should flush");
    let ResponseItem::CommandExecutionNotification { id, output, .. } = item else {
        panic!("expected output notification item");
    };
    assert_eq!(id, Some("call-1:notification:output:7".to_string()));
    assert_eq!(output, Some("only".to_string()));
}

#[test]
fn output_notification_aggregator_flushes_at_size_limit() {
    let mut aggregator = OutputNotificationAggregator::default();

    aggregator.push(1, "a".repeat(MAX_OUTPUT_NOTIFICATION_BYTES - 1));
    assert!(!aggregator.should_flush_for_size());
    aggregator.push(2, "b".to_string());

    assert!(aggregator.should_flush_for_size());
    let item = aggregator
        .take_item("call-1")
        .expect("size-capped output should flush");
    let ResponseItem::CommandExecutionNotification { id, output, .. } = item else {
        panic!("expected output notification item");
    };
    assert_eq!(id, Some("call-1:notification:output:1-2".to_string()));
    assert_eq!(
        output.as_deref().map(str::len),
        Some(MAX_OUTPUT_NOTIFICATION_BYTES)
    );
}

#[test]
fn output_notification_aggregator_bounds_oversized_flushes() {
    let mut aggregator = OutputNotificationAggregator::default();

    aggregator.push(1, "a".repeat(MAX_OUTPUT_NOTIFICATION_BYTES - 1));
    aggregator.push(2, "bcdef".to_string());

    let item = aggregator
        .take_item("call-1")
        .expect("oversized output should flush");
    let ResponseItem::CommandExecutionNotification { output, .. } = item else {
        panic!("expected output notification item");
    };
    let output = output.expect("output should be present");

    assert!(output.len() <= MAX_OUTPUT_NOTIFICATION_BYTES);
    assert!(output.starts_with('a'));
    assert!(output.ends_with("bcdef"));
}

#[test]
fn exit_notification_item_uses_terminal_fallback_exit_code() {
    let item =
        super::command_exit_notification_item("call-1", Some("finished".to_string()), -1, 1234);
    let ResponseItem::CommandExecutionNotification {
        kind,
        message,
        output,
        exit_code,
        ..
    } = item
    else {
        panic!("expected exit notification item");
    };

    assert_eq!(kind, CommandExecutionNotificationKind::Exit);
    assert_eq!(message, "Command call-1 has exited with code -1.");
    assert_eq!(output, Some("finished".to_string()));
    assert_eq!(exit_code, Some(-1));
}

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
async fn exit_notification_output_is_bounded_for_large_transcript() {
    let transcript = Arc::new(Mutex::new(HeadTailBuffer::new(
        MAX_OUTPUT_NOTIFICATION_BYTES * 2,
    )));
    let exit_notification_output = Arc::new(Mutex::new(HeadTailBuffer::default()));
    {
        let mut guard = transcript.lock().await;
        guard.push_chunk("a".repeat(MAX_OUTPUT_NOTIFICATION_BYTES * 2).into_bytes());
        guard.push_chunk(b"tail-marker".to_vec());
    }

    let output = super::resolve_exit_notification_output(
        &transcript,
        &exit_notification_output,
        CommandNotificationFilter::Exit,
        None,
    )
    .await
    .expect("large transcript should produce bounded notification output");

    assert!(output.len() <= MAX_OUTPUT_NOTIFICATION_BYTES);
    assert!(output.ends_with("tail-marker"));
}

#[test]
fn command_notification_output_bounding_handles_multibyte_boundaries() {
    let output =
        super::bound_command_notification_output("é".repeat(MAX_OUTPUT_NOTIFICATION_BYTES));

    assert!(output.len() <= MAX_OUTPUT_NOTIFICATION_BYTES);
    assert!(!output.is_empty());
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
