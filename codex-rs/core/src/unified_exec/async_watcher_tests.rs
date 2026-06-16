use super::process_chunk;
use super::split_valid_utf8_prefix_with_max;

use crate::unified_exec::CommandNotificationFilter;
use crate::unified_exec::CommandNotificationState;
use crate::unified_exec::HeadTailBuffer;
use codex_protocol::protocol::EventMsg;
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
}
