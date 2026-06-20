use std::sync::Arc;

use codex_exec_server_api::ExecRuntimeError;
use futures::FutureExt;
use futures::future::BoxFuture;
use tokio::sync::watch;
use tracing::trace;

use crate::ExecBackend;
use crate::ExecProcess;
use crate::ExecProcessEventReceiver;
use crate::StartedExecProcess;
use crate::client::LazyRemoteExecServerClient;
use crate::client::Session;
use crate::protocol::ExecParams;
use crate::protocol::ReadResponse;
use crate::protocol::WriteResponse;

#[derive(Clone)]
pub(crate) struct RemoteProcess {
    client: LazyRemoteExecServerClient,
}

struct RemoteExecProcess {
    session: Session,
}

impl RemoteProcess {
    pub(crate) fn new(client: LazyRemoteExecServerClient) -> Self {
        trace!("remote process new");
        Self { client }
    }
}

impl ExecBackend for RemoteProcess {
    fn start(
        &self,
        params: ExecParams,
    ) -> BoxFuture<'_, Result<StartedExecProcess, ExecRuntimeError>> {
        async move {
            let process_id = params.process_id.clone();
            let client = self.client.get().await.map_err(ExecRuntimeError::from)?;
            let session = client
                .register_session(&process_id)
                .await
                .map_err(ExecRuntimeError::from)?;
            if let Err(err) = client.exec(params).await {
                session.unregister().await;
                return Err(ExecRuntimeError::from(err));
            }

            Ok(StartedExecProcess {
                process: Arc::new(RemoteExecProcess { session }),
            })
        }
        .boxed()
    }
}

impl ExecProcess for RemoteExecProcess {
    fn process_id(&self) -> &crate::ProcessId {
        self.session.process_id()
    }

    fn subscribe_wake(&self) -> watch::Receiver<u64> {
        self.session.subscribe_wake()
    }

    fn subscribe_events(&self) -> ExecProcessEventReceiver {
        self.session.subscribe_events()
    }

    fn read(
        &self,
        after_seq: Option<u64>,
        max_bytes: Option<usize>,
        wait_ms: Option<u64>,
    ) -> BoxFuture<'_, Result<ReadResponse, ExecRuntimeError>> {
        async move {
            self.session
                .read(after_seq, max_bytes, wait_ms)
                .await
                .map_err(ExecRuntimeError::from)
        }
        .boxed()
    }

    fn write(&self, chunk: Vec<u8>) -> BoxFuture<'_, Result<WriteResponse, ExecRuntimeError>> {
        async move {
            trace!("exec process write");
            self.session
                .write(chunk)
                .await
                .map_err(ExecRuntimeError::from)
        }
        .boxed()
    }

    fn terminate(&self) -> BoxFuture<'_, Result<(), ExecRuntimeError>> {
        async move {
            trace!("exec process terminate");
            self.session
                .terminate()
                .await
                .map_err(ExecRuntimeError::from)
        }
        .boxed()
    }
}

impl Drop for RemoteExecProcess {
    fn drop(&mut self) {
        let session = self.session.clone();
        tokio::spawn(async move {
            session.unregister().await;
        });
    }
}
