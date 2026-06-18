use super::emit_exec_end_for_unified_exec;
use super::process_chunk;
use super::split_valid_utf8_prefix_with_max;

use crate::unified_exec::CommandNotificationFilter;
use crate::unified_exec::CommandNotificationState;
use crate::unified_exec::HeadTailBuffer;
use codex_protocol::models::CommandExecutionNotificationKind;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ExecCommandNotifyOn;
use pretty_assertions::assert_eq;
use std::sync::Arc;
use tokio::time::Duration;

#[test]
fn split_valid_utf8_prefix_respects_max_bytes_for_ascii() {
    let mut buf = b"hello word!".to_vec();

    let first =
        split_valid_utf8_prefix_with_max(&mut buf, /*max_bytes*/ 5).expect("expected prefix");
    assert_eq!(first, b"hello".to_vec());
    assert_eq!(buf, b" word!".to_vec());

    let second =
        split_valid_utf8_prefix_with_max(&mut buf, /*max_bytes*/ 5).expect("expected prefix");
    assert_eq!(second, b" word".to_vec());
    assert_eq!(buf, b"!".to_vec());
}

#[test]
fn split_valid_utf8_prefix_avoids_splitting_utf8_codepoints() {
    // "é" is 2 bytes in UTF-8. With a max of 3 bytes, we should only emit 1 char (2 bytes).
    let mut buf = "ééé".as_bytes().to_vec();

    let first =
        split_valid_utf8_prefix_with_max(&mut buf, /*max_bytes*/ 3).expect("expected prefix");
    assert_eq!(std::str::from_utf8(&first).unwrap(), "é");
    assert_eq!(buf, "éé".as_bytes().to_vec());
}

#[test]
fn split_valid_utf8_prefix_makes_progress_on_invalid_utf8() {
    let mut buf = vec![0xff, b'a', b'b'];

    let first =
        split_valid_utf8_prefix_with_max(&mut buf, /*max_bytes*/ 2).expect("expected prefix");
    assert_eq!(first, vec![0xff]);
    assert_eq!(buf, b"ab".to_vec());
}

#[tokio::test]
async fn output_delta_generates_notification_only_after_background_session_activation() {
    let (session, turn, rx_event) = crate::session::tests::make_session_and_context_with_rx().await;
    let transcript = Arc::new(tokio::sync::Mutex::new(HeadTailBuffer::default()));
    let notification_state = Arc::new(CommandNotificationState::default());
    let mut pending = Vec::new();
    let mut emitted_deltas = 0;

    process_chunk(
        &mut pending,
        &transcript,
        "call-output",
        &session,
        &turn,
        &mut emitted_deltas,
        CommandNotificationFilter::Output,
        &notification_state,
        b"inline".to_vec(),
    )
    .await;

    let event = tokio::time::timeout(Duration::from_secs(1), rx_event.recv())
        .await
        .expect("timed out waiting for inline output delta")
        .expect("event channel closed");
    let EventMsg::ExecCommandOutputDelta(delta) = event.msg else {
        panic!("expected ExecCommandOutputDelta");
    };
    assert_eq!(delta.generates_notification, false);

    notification_state.activate_background_session();
    process_chunk(
        &mut pending,
        &transcript,
        "call-output",
        &session,
        &turn,
        &mut emitted_deltas,
        CommandNotificationFilter::Output,
        &notification_state,
        b"background".to_vec(),
    )
    .await;

    let event = tokio::time::timeout(Duration::from_secs(1), rx_event.recv())
        .await
        .expect("timed out waiting for background output delta")
        .expect("event channel closed");
    let EventMsg::ExecCommandOutputDelta(delta) = event.msg else {
        panic!("expected ExecCommandOutputDelta");
    };
    assert_eq!(delta.generates_notification, true);

    let EventMsg::CommandExecutionNotificationCompleted(notification) =
        recv_matching_event(&rx_event, |event| {
            matches!(event, EventMsg::CommandExecutionNotificationCompleted(_))
        })
        .await
    else {
        panic!("expected CommandExecutionNotificationCompleted");
    };
    assert_eq!(notification.command_item_id, "call-output");
    assert_eq!(notification.kind, CommandExecutionNotificationKind::Output);
    assert_eq!(notification.output, Some("background".to_string()));
    assert_eq!(notification.exit_code, None);
}

#[tokio::test]
async fn unified_exec_end_records_exit_notification_for_background_process() {
    let (session, turn, rx_event) = crate::session::tests::make_session_and_context_with_rx().await;
    let transcript = Arc::new(tokio::sync::Mutex::new(HeadTailBuffer::default()));
    transcript.lock().await.push_chunk(b"finished\n".to_vec());
    let cwd = std::env::current_dir()
        .expect("current dir")
        .try_into()
        .expect("absolute path");

    emit_exec_end_for_unified_exec(
        session,
        turn,
        "call-exit".to_string(),
        vec!["echo".to_string(), "finished".to_string()],
        cwd,
        Some("123".to_string()),
        transcript,
        String::new(),
        7,
        Duration::from_millis(5),
        1_000,
        ExecCommandNotifyOn::Exit,
    )
    .await;

    let EventMsg::ExecCommandEnd(end) = recv_matching_event(&rx_event, |event| {
        matches!(event, EventMsg::ExecCommandEnd(_))
    })
    .await
    else {
        panic!("expected ExecCommandEnd");
    };
    assert_eq!(end.call_id, "call-exit");
    assert_eq!(end.process_id, Some("123".to_string()));
    assert_eq!(end.exit_code, 7);

    let EventMsg::CommandExecutionNotificationCompleted(notification) =
        recv_matching_event(&rx_event, |event| {
            matches!(event, EventMsg::CommandExecutionNotificationCompleted(_))
        })
        .await
    else {
        panic!("expected CommandExecutionNotificationCompleted");
    };
    assert_eq!(notification.command_item_id, "call-exit");
    assert_eq!(notification.kind, CommandExecutionNotificationKind::Exit);
    assert_eq!(notification.output, Some("finished\n".to_string()));
    assert_eq!(notification.exit_code, Some(7));
}

async fn recv_matching_event(
    rx_event: &async_channel::Receiver<codex_protocol::protocol::Event>,
    predicate: impl Fn(&EventMsg) -> bool,
) -> EventMsg {
    for _ in 0..8 {
        let event = tokio::time::timeout(Duration::from_secs(1), rx_event.recv())
            .await
            .expect("timed out waiting for matching event")
            .expect("event channel closed");
        if predicate(&event.msg) {
            return event.msg;
        }
    }
    panic!("matching event not received");
}
