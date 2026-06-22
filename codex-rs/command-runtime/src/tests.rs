use super::*;

use pretty_assertions::assert_eq;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::time::Duration;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

fn test_output_handles() -> CommandOutputHandles {
    CommandOutputHandles {
        output_buffer: Arc::new(Mutex::new(HeadTailBuffer::default())),
        output_notify: Arc::new(Notify::new()),
        output_closed: Arc::new(AtomicBool::new(false)),
        output_closed_notify: Arc::new(Notify::new()),
        cancellation_token: CancellationToken::new(),
    }
}

#[test]
fn keeps_prefix_and_suffix_when_over_budget() {
    let mut buf = HeadTailBuffer::new(/*max_bytes*/ 10);

    buf.push_chunk(b"0123456789".to_vec());
    assert_eq!(buf.omitted_bytes(), 0);

    buf.push_chunk(b"ab".to_vec());
    assert!(buf.omitted_bytes() > 0);

    let rendered = String::from_utf8_lossy(&buf.to_bytes()).to_string();
    assert!(rendered.starts_with("01234"));
    assert!(rendered.ends_with("89ab"));
}

#[test]
fn max_bytes_zero_drops_everything() {
    let mut buf = HeadTailBuffer::new(/*max_bytes*/ 0);
    buf.push_chunk(b"abc".to_vec());

    assert_eq!(buf.retained_bytes(), 0);
    assert_eq!(buf.omitted_bytes(), 3);
    assert_eq!(buf.to_bytes(), b"".to_vec());
    assert_eq!(buf.snapshot_chunks(), Vec::<Vec<u8>>::new());
}

#[test]
fn head_budget_zero_keeps_only_last_byte_in_tail() {
    let mut buf = HeadTailBuffer::new(/*max_bytes*/ 1);
    buf.push_chunk(b"abc".to_vec());

    assert_eq!(buf.retained_bytes(), 1);
    assert_eq!(buf.omitted_bytes(), 2);
    assert_eq!(buf.to_bytes(), b"c".to_vec());
}

#[test]
fn draining_resets_state() {
    let mut buf = HeadTailBuffer::new(/*max_bytes*/ 10);
    buf.push_chunk(b"0123456789".to_vec());
    buf.push_chunk(b"ab".to_vec());

    let drained = buf.drain_chunks();
    assert!(!drained.is_empty());

    assert_eq!(buf.retained_bytes(), 0);
    assert_eq!(buf.omitted_bytes(), 0);
    assert_eq!(buf.to_bytes(), b"".to_vec());
}

#[test]
fn chunk_larger_than_tail_budget_keeps_only_tail_end() {
    let mut buf = HeadTailBuffer::new(/*max_bytes*/ 10);
    buf.push_chunk(b"0123456789".to_vec());

    buf.push_chunk(b"ABCDEFGHIJK".to_vec());

    let out = String::from_utf8_lossy(&buf.to_bytes()).to_string();
    assert!(out.starts_with("01234"));
    assert!(out.ends_with("GHIJK"));
    assert!(buf.omitted_bytes() > 0);
}

#[test]
fn fills_head_then_tail_across_multiple_chunks() {
    let mut buf = HeadTailBuffer::new(/*max_bytes*/ 10);

    buf.push_chunk(b"01".to_vec());
    buf.push_chunk(b"234".to_vec());
    assert_eq!(buf.to_bytes(), b"01234".to_vec());

    buf.push_chunk(b"567".to_vec());
    buf.push_chunk(b"89".to_vec());
    assert_eq!(buf.to_bytes(), b"0123456789".to_vec());
    assert_eq!(buf.omitted_bytes(), 0);

    buf.push_chunk(b"a".to_vec());
    assert_eq!(buf.to_bytes(), b"012346789a".to_vec());
    assert_eq!(buf.omitted_bytes(), 1);
}

#[test]
fn process_state_preserves_failure_when_exited() {
    let state = ProcessState::default().failed("failed".to_string());

    assert_eq!(
        state.exited(Some(2)),
        ProcessState {
            has_exited: true,
            exit_code: Some(2),
            failure_message: Some("failed".to_string()),
        }
    );
}

#[tokio::test]
async fn collect_output_drains_available_chunks() {
    let handles = test_output_handles();
    handles
        .output_buffer
        .lock()
        .await
        .push_chunk(b"ready".to_vec());

    let collected =
        collect_output_until_deadline(&handles, None, Instant::now() + Duration::from_millis(1))
            .await;

    assert_eq!(collected, b"ready".to_vec());
}

#[tokio::test]
async fn collect_output_waits_for_notification_before_deadline() {
    let handles = test_output_handles();
    let handles_for_task = handles.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        handles_for_task
            .output_buffer
            .lock()
            .await
            .push_chunk(b"later".to_vec());
        handles_for_task.output_notify.notify_waiters();
    });

    let collected =
        collect_output_until_deadline(&handles, None, Instant::now() + Duration::from_millis(200))
            .await;

    assert_eq!(collected, b"later".to_vec());
}

#[tokio::test]
async fn collect_output_extends_deadline_while_paused() {
    let handles = test_output_handles();
    let handles_for_task = handles.clone();
    let (pause_tx, pause_rx) = tokio::sync::watch::channel(true);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        handles_for_task
            .output_buffer
            .lock()
            .await
            .push_chunk(b"after-pause".to_vec());
        handles_for_task.output_notify.notify_waiters();
        pause_tx.send(false).expect("pause receiver should be open");
    });

    let collected = tokio::time::timeout(
        Duration::from_secs(1),
        collect_output_until_deadline(
            &handles,
            Some(pause_rx),
            Instant::now() + Duration::from_millis(5),
        ),
    )
    .await
    .expect("collector should finish");

    assert_eq!(collected, b"after-pause".to_vec());
}

#[tokio::test]
async fn notification_wait_after_ignores_existing_snapshot_and_returns_next_kind() {
    let state = CommandNotificationState::default();

    state.notify(CommandNotificationKind::Output).await;
    let snapshot = state.snapshot().await;
    state.notify(CommandNotificationKind::Exit).await;

    let kind = tokio::time::timeout(Duration::from_secs(1), state.wait_after(snapshot))
        .await
        .expect("notification should arrive");

    assert_eq!(kind, CommandNotificationKind::Exit);
}

#[test]
fn wait_backoff_advances_to_max_and_resets_to_initial() {
    let mut state = WaitBackoffState::new(Duration::from_millis(50), Duration::from_millis(120));

    assert_eq!(state.current_window(), Duration::from_millis(50));
    state.advance_after_timeout();
    assert_eq!(state.current_window(), Duration::from_millis(100));
    state.advance_after_timeout();
    assert_eq!(state.current_window(), Duration::from_millis(120));
    state.reset_after_event();
    assert_eq!(state.current_window(), Duration::from_millis(50));
}

#[test]
fn clamps_yield_time_to_supported_range() {
    assert_eq!(clamp_yield_time(1), MIN_YIELD_TIME_MS);
    assert_eq!(clamp_yield_time(10_000), 10_000);
    assert_eq!(clamp_yield_time(60_000), MAX_YIELD_TIME_MS);
}

#[test]
fn resolves_default_max_tokens() {
    assert_eq!(resolve_max_tokens(None), DEFAULT_MAX_OUTPUT_TOKENS);
    assert_eq!(resolve_max_tokens(Some(123)), 123);
}

#[test]
fn generates_six_hex_character_chunk_ids() {
    let id = generate_chunk_id();

    assert_eq!(id.len(), 6);
    assert!(id.chars().all(|ch| ch.is_ascii_hexdigit()));
}
