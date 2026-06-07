use std::sync::Arc;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::ChildStdin;
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::Notify;
use tokio::sync::mpsc;
use tokio::sync::oneshot;

const EVENT_COMMAND_STDIN_READY_TIMEOUT: Duration = Duration::from_secs(2);
const EVENT_COMMAND_STDIN_WRITE_QUEUE_SIZE: usize = 8;

struct EventCommandStdinWrite {
    bytes: Vec<u8>,
    result_tx: oneshot::Sender<Result<(), String>>,
}

#[derive(Clone)]
pub(crate) struct EventCommandRuntime {
    writer_tx: Arc<AsyncMutex<Option<mpsc::Sender<EventCommandStdinWrite>>>>,
    writer_ready: Arc<Notify>,
}

impl EventCommandRuntime {
    pub(crate) fn new() -> Self {
        Self {
            writer_tx: Arc::new(AsyncMutex::new(None)),
            writer_ready: Arc::new(Notify::new()),
        }
    }

    pub(crate) async fn set_stdin(&self, stdin: Option<ChildStdin>) {
        let writer_tx = stdin.map(start_event_command_stdin_writer);
        *self.writer_tx.lock().await = writer_tx;
        self.writer_ready.notify_waiters();
    }

    pub(crate) async fn write_stdin(
        &self,
        subscription_id: &str,
        chars: &str,
    ) -> Result<(), String> {
        let writer_tx = loop {
            let notified = self.writer_ready.notified();
            if let Some(writer_tx) = self.writer_tx.lock().await.as_ref().cloned() {
                break writer_tx;
            }
            tokio::time::timeout(EVENT_COMMAND_STDIN_READY_TIMEOUT, notified)
                .await
                .map_err(|_| format!("event command stdin unavailable: {subscription_id}"))?;
        };
        let (result_tx, result_rx) = oneshot::channel();
        writer_tx
            .send(EventCommandStdinWrite {
                bytes: chars.as_bytes().to_vec(),
                result_tx,
            })
            .await
            .map_err(|_| format!("event command stdin unavailable: {subscription_id}"))?;
        result_rx
            .await
            .map_err(|_| format!("event command stdin unavailable: {subscription_id}"))?
    }
}

fn start_event_command_stdin_writer(mut stdin: ChildStdin) -> mpsc::Sender<EventCommandStdinWrite> {
    let (tx, mut rx) =
        mpsc::channel::<EventCommandStdinWrite>(EVENT_COMMAND_STDIN_WRITE_QUEUE_SIZE);
    tokio::spawn(async move {
        while let Some(request) = rx.recv().await {
            let result = async {
                stdin
                    .write_all(&request.bytes)
                    .await
                    .map_err(|err| format!("failed to write event command stdin: {err}"))?;
                stdin
                    .flush()
                    .await
                    .map_err(|err| format!("failed to flush event command stdin: {err}"))
            }
            .await;
            let _ = request.result_tx.send(result);
        }
    });
    tx
}
