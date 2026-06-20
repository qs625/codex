pub use codex_exec_server_api::ExecProcessEvent;
pub use codex_exec_server_api::ExecProcessEventLog;
pub use codex_exec_server_api::ExecProcessEventReceiver;

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use tokio::time::Duration;
    use tokio::time::timeout;

    use crate::protocol::ExecOutputStream;
    use crate::protocol::ProcessOutputChunk;
    use codex_exec_server_api::ExecProcessEvent;
    use codex_exec_server_api::ExecProcessEventLog;

    #[tokio::test]
    async fn event_history_replay_is_bounded_by_retained_bytes() {
        let log = ExecProcessEventLog::new(/*event_capacity*/ 8, /*byte_capacity*/ 3);

        log.publish(ExecProcessEvent::Output(ProcessOutputChunk {
            seq: 1,
            stream: ExecOutputStream::Stdout,
            chunk: b"large".to_vec().into(),
        }));
        log.publish(ExecProcessEvent::Exited {
            seq: 2,
            exit_code: 0,
        });
        log.publish(ExecProcessEvent::Closed { seq: 3 });

        let mut events = log.subscribe();
        let replay = vec![
            timeout(Duration::from_secs(1), events.recv())
                .await
                .expect("exit event replay should not time out")
                .expect("exit event replay should be available"),
            timeout(Duration::from_secs(1), events.recv())
                .await
                .expect("closed event replay should not time out")
                .expect("closed event replay should be available"),
        ];

        assert_eq!(
            replay,
            vec![
                ExecProcessEvent::Exited {
                    seq: 2,
                    exit_code: 0,
                },
                ExecProcessEvent::Closed { seq: 3 },
            ]
        );
    }
}
