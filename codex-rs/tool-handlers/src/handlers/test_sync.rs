use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolExecutor;
use codex_tool_types::ToolExecutorFuture;
use codex_tool_types::ToolName;
use codex_tool_types::ToolPayload;
use codex_tool_types::ToolSpec;
use serde::Deserialize;
use tokio::sync::Barrier;
use tokio::time::sleep;

use crate::FunctionToolOutput;
use codex_tool_planning::create_test_sync_tool;
use codex_tool_runtime::ToolHandler;
use codex_tool_runtime::ToolInvocationView;

pub struct TestSyncHandler<Invocation = ()> {
    _marker: PhantomData<fn(Invocation)>,
}

impl<Invocation> Default for TestSyncHandler<Invocation> {
    fn default() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

impl<Invocation> TestSyncHandler<Invocation> {
    pub fn new() -> Self {
        Self::default()
    }
}

const DEFAULT_TIMEOUT_MS: u64 = 1_000;

static BARRIERS: OnceLock<tokio::sync::Mutex<HashMap<String, BarrierState>>> = OnceLock::new();

struct BarrierState {
    barrier: Arc<Barrier>,
    participants: usize,
}

#[derive(Debug, Deserialize)]
struct BarrierArgs {
    id: String,
    participants: usize,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
}

#[derive(Debug, Deserialize)]
struct TestSyncArgs {
    #[serde(default)]
    sleep_before_ms: Option<u64>,
    #[serde(default)]
    sleep_after_ms: Option<u64>,
    #[serde(default)]
    barrier: Option<BarrierArgs>,
}

fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}

fn barrier_map() -> &'static tokio::sync::Mutex<HashMap<String, BarrierState>> {
    BARRIERS.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

impl<Invocation> ToolExecutor<Invocation> for TestSyncHandler<Invocation>
where
    Invocation: ToolInvocationView + Send,
{
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("test_sync_tool")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_test_sync_tool())
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

    fn handle<'a>(&'a self, invocation: Invocation) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
        Invocation: 'a,
    {
        Box::pin(async move {
            let ToolPayload::Function { arguments } = invocation.payload() else {
                return Err(FunctionCallError::RespondToModel(
                    "test_sync_tool handler received unsupported payload".to_string(),
                ));
            };

            let args: TestSyncArgs = serde_json::from_str(arguments).map_err(|err| {
                FunctionCallError::RespondToModel(format!(
                    "failed to parse function arguments: {err}"
                ))
            })?;

            if let Some(delay) = args.sleep_before_ms
                && delay > 0
            {
                sleep(Duration::from_millis(delay)).await;
            }

            if let Some(barrier) = args.barrier {
                wait_on_barrier(barrier).await?;
            }

            if let Some(delay) = args.sleep_after_ms
                && delay > 0
            {
                sleep(Duration::from_millis(delay)).await;
            }

            Ok(FunctionToolOutput::from_text("ok".to_string(), Some(true)))
        })
    }
}

impl<Invocation, DiffContext> ToolHandler<Invocation, DiffContext> for TestSyncHandler<Invocation> where
    Invocation: ToolInvocationView + Send
{
}

async fn wait_on_barrier(args: BarrierArgs) -> Result<(), FunctionCallError> {
    if args.participants == 0 {
        return Err(FunctionCallError::RespondToModel(
            "barrier participants must be greater than zero".to_string(),
        ));
    }

    if args.timeout_ms == 0 {
        return Err(FunctionCallError::RespondToModel(
            "barrier timeout must be greater than zero".to_string(),
        ));
    }

    let barrier_id = args.id.clone();
    let barrier = {
        let mut map = barrier_map().lock().await;
        match map.entry(barrier_id.clone()) {
            Entry::Occupied(entry) => {
                let state = entry.get();
                if state.participants != args.participants {
                    let existing = state.participants;
                    return Err(FunctionCallError::RespondToModel(format!(
                        "barrier {barrier_id} already registered with {existing} participants"
                    )));
                }
                state.barrier.clone()
            }
            Entry::Vacant(entry) => {
                let barrier = Arc::new(Barrier::new(args.participants));
                entry.insert(BarrierState {
                    barrier: barrier.clone(),
                    participants: args.participants,
                });
                barrier
            }
        }
    };

    let timeout = Duration::from_millis(args.timeout_ms);
    let wait_result = tokio::time::timeout(timeout, barrier.wait())
        .await
        .map_err(|_| {
            FunctionCallError::RespondToModel("test_sync_tool barrier wait timed out".to_string())
        })?;

    if wait_result.is_leader() {
        let mut map = barrier_map().lock().await;
        if let Some(state) = map.get(&barrier_id)
            && Arc::ptr_eq(&state.barrier, &barrier)
        {
            map.remove(&barrier_id);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use codex_tool_runtime::ToolInvocation;
    use codex_tool_types::ToolCallSource;
    use codex_tool_types::ToolExecutor;
    use codex_tool_types::ToolInvocationMetadata;
    use codex_tool_types::ToolName;
    use codex_tool_types::ToolOutput;
    use codex_tool_types::ToolPayload;
    use serde_json::json;

    use super::TestSyncHandler;

    #[tokio::test]
    async fn handles_empty_function_args() {
        let handler = TestSyncHandler::new();
        let invocation = ToolInvocation {
            session: (),
            turn: (),
            cancellation_token: tokio_util::sync::CancellationToken::new(),
            tracker: (),
            metadata: ToolInvocationMetadata {
                call_id: "call-sync".to_string(),
                tool_name: ToolName::plain("test_sync_tool"),
                source: ToolCallSource::Direct,
                payload: ToolPayload::Function {
                    arguments: json!({}).to_string(),
                },
            },
        };

        let output = handler.handle(invocation).await.expect("sync output");
        assert_eq!(output.log_preview(), "ok");
    }
}
