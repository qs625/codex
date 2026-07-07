use super::*;

use pretty_assertions::assert_eq;
use tokio::time::Duration;
use tokio::time::Instant;

fn test_output_handles() -> CommandOutputHandles {
    CommandOutputRuntime::new().handles()
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
    // "é" is 2 bytes in UTF-8. With a max of 3 bytes, emit only 1 char.
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
async fn output_runtime_pumps_broadcast_chunks_into_shared_buffer() {
    let runtime = CommandOutputRuntime::new();
    let handles = runtime.handles();
    let (tx, rx) = tokio::sync::broadcast::channel(4);
    let task = tokio::spawn(runtime.pump_broadcast_receiver(rx));

    tx.send(b"chunk".to_vec()).expect("receiver should be open");
    drop(tx);
    task.await.expect("pump should finish");

    let collected =
        collect_output_until_deadline(&handles, None, Instant::now() + Duration::from_millis(1))
            .await;
    assert_eq!(collected, b"chunk".to_vec());
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
        .expect("notification should arrive")
        .0;

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

#[test]
fn deterministic_process_ids_start_at_one_thousand_and_advance() {
    let mut allocator = CommandProcessIdAllocator::default();

    assert_eq!(allocator.reserve_next(/*deterministic*/ true), 1000);
    assert_eq!(allocator.reserve_next(/*deterministic*/ true), 1001);
}

#[test]
fn released_process_id_can_be_reserved_again() {
    let mut allocator = CommandProcessIdAllocator::default();
    let first = allocator.reserve_next(/*deterministic*/ true);
    allocator.release_reservation(first);

    assert_eq!(allocator.reserve_next(/*deterministic*/ true), first);
}

#[test]
fn completed_process_id_is_not_reused_until_pruned() {
    let mut allocator = CommandProcessIdAllocator::new(/*max_completed_processes*/ 1);
    let first = allocator.reserve_next(/*deterministic*/ true);
    allocator.release_reservation(first);
    allocator.mark_completed(first, Some(7));

    assert_eq!(
        allocator.completed_process(first),
        Some(&CompletedCommandProcess {
            exit_code: Some(7),
            completed_at: allocator
                .completed_process(first)
                .expect("completed process should exist")
                .completed_at,
        })
    );
    assert_eq!(allocator.reserve_next(/*deterministic*/ true), first + 1);
}

#[test]
fn completed_process_history_prunes_oldest_entry() {
    let mut allocator = CommandProcessIdAllocator::new(/*max_completed_processes*/ 1);
    allocator.mark_completed(1000, Some(0));
    std::thread::sleep(Duration::from_millis(1));
    allocator.mark_completed(1001, Some(1));

    assert!(allocator.completed_process(1000).is_none());
    assert_eq!(
        allocator
            .completed_process(1001)
            .map(|entry| entry.exit_code),
        Some(Some(1))
    );
}

fn prune_meta(
    process_id: i32,
    last_used: tokio::time::Instant,
    has_exited: bool,
) -> CommandProcessPruneMeta {
    CommandProcessPruneMeta {
        process_id,
        last_used,
        has_exited,
    }
}

#[test]
fn pruning_prefers_exited_processes_outside_recently_used() {
    let now = tokio::time::Instant::now();
    let meta = vec![
        prune_meta(1, now - Duration::from_secs(40), false),
        prune_meta(2, now - Duration::from_secs(30), true),
        prune_meta(3, now - Duration::from_secs(20), false),
        prune_meta(4, now - Duration::from_secs(19), false),
        prune_meta(5, now - Duration::from_secs(18), false),
        prune_meta(6, now - Duration::from_secs(17), false),
        prune_meta(7, now - Duration::from_secs(16), false),
        prune_meta(8, now - Duration::from_secs(15), false),
        prune_meta(9, now - Duration::from_secs(14), false),
        prune_meta(10, now - Duration::from_secs(13), false),
    ];

    assert_eq!(command_process_id_to_prune(&meta), Some(2));
}

#[test]
fn pruning_falls_back_to_lru_when_no_exited() {
    let now = tokio::time::Instant::now();
    let meta = vec![
        prune_meta(1, now - Duration::from_secs(40), false),
        prune_meta(2, now - Duration::from_secs(30), false),
        prune_meta(3, now - Duration::from_secs(20), false),
        prune_meta(4, now - Duration::from_secs(19), false),
        prune_meta(5, now - Duration::from_secs(18), false),
        prune_meta(6, now - Duration::from_secs(17), false),
        prune_meta(7, now - Duration::from_secs(16), false),
        prune_meta(8, now - Duration::from_secs(15), false),
        prune_meta(9, now - Duration::from_secs(14), false),
        prune_meta(10, now - Duration::from_secs(13), false),
    ];

    assert_eq!(command_process_id_to_prune(&meta), Some(1));
}

#[test]
fn pruning_protects_recent_processes_even_if_exited() {
    let now = tokio::time::Instant::now();
    let meta = vec![
        prune_meta(1, now - Duration::from_secs(40), false),
        prune_meta(2, now - Duration::from_secs(30), false),
        prune_meta(3, now - Duration::from_secs(20), true),
        prune_meta(4, now - Duration::from_secs(19), false),
        prune_meta(5, now - Duration::from_secs(18), false),
        prune_meta(6, now - Duration::from_secs(17), false),
        prune_meta(7, now - Duration::from_secs(16), false),
        prune_meta(8, now - Duration::from_secs(15), false),
        prune_meta(9, now - Duration::from_secs(14), false),
        prune_meta(10, now - Duration::from_secs(13), true),
    ];

    assert_eq!(command_process_id_to_prune(&meta), Some(1));
}
