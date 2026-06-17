use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAgentBinding {
    pub agent_id: String,
    pub agent_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Value>,
    #[serde(default)]
    pub options: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRuntimeRequest {
    pub run_id: String,
    pub workflow_id: String,
    pub rpc_id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRuntimeError {
    pub code: String,
    pub message: String,
}

impl WorkflowRuntimeError {
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self {
            code: "unsupported".to_string(),
            message: message.into(),
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_request".to_string(),
            message: message.into(),
        }
    }
}

/// Host implementation for workflow runtime requests.
///
/// Implementations must route requests through Codex runtime primitives so workflow scripts keep
/// the same permission, lifecycle, and typed event semantics as normal tool calls.
pub trait WorkflowRuntimeBridge: Send + Sync {
    fn call(
        &self,
        request: WorkflowRuntimeRequest,
    ) -> Pin<Box<dyn Future<Output = Result<Value, WorkflowRuntimeError>> + Send + '_>>;
}

#[derive(Debug)]
pub(crate) struct RunnerProcessState {
    pub(crate) run_id: String,
    pub(crate) workflow_id: String,
    pub(crate) bindings: BTreeMap<String, WorkflowAgentBinding>,
    events: Vec<Value>,
    output: Option<Value>,
    protocol_error: Option<String>,
}

impl RunnerProcessState {
    pub(crate) fn new(
        run_id: String,
        workflow_id: String,
        bindings: BTreeMap<String, WorkflowAgentBinding>,
    ) -> Self {
        Self {
            run_id,
            workflow_id,
            bindings,
            events: Vec::new(),
            output: None,
            protocol_error: None,
        }
    }

    pub(crate) fn output(&self) -> Value {
        serde_json::json!({
            "result": self.output.clone().unwrap_or(Value::Null),
            "events": self.events.clone(),
            "bindings": self.bindings.clone(),
        })
    }

    pub(crate) fn error_without_stderr(&self) -> String {
        self.protocol_error
            .clone()
            .unwrap_or_else(|| "workflow runner failed without stderr".to_string())
    }

    pub(crate) fn protocol_error(&self) -> Option<String> {
        self.protocol_error.clone()
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum RunnerStdoutFrame {
    Rpc {
        id: u64,
        method: String,
        #[serde(default)]
        params: Value,
    },
    Event {
        #[serde(default)]
        event: Value,
    },
    Output {
        #[serde(default)]
        output: Value,
    },
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum RunnerStdinFrame<'a> {
    RpcResult { id: u64, result: &'a Value },
    RpcError { id: u64, error: &'a WorkflowRuntimeError },
}

pub(crate) async fn read_runner_stdout(
    pipe: Option<impl tokio::io::AsyncRead + Unpin>,
    stdin: Option<impl tokio::io::AsyncWrite + Unpin>,
    state: Arc<Mutex<RunnerProcessState>>,
    bridge: Option<Arc<dyn WorkflowRuntimeBridge>>,
) -> Result<(), String> {
    let Some(pipe) = pipe else {
        return Ok(());
    };
    let mut stdin = stdin;
    let mut reader = BufReader::new(pipe).lines();
    while let Some(line) = reader
        .next_line()
        .await
        .map_err(|err| format!("failed to read workflow runner stdout: {err}"))?
    {
        let frame = serde_json::from_str::<RunnerStdoutFrame>(&line)
            .map_err(|err| format!("invalid workflow runner frame: {err}: {line}"))?;
        match frame {
            RunnerStdoutFrame::Rpc { id, method, params } => {
                let result = handle_runner_rpc(id, method, params, &state, bridge.as_ref()).await;
                let Some(stdin) = stdin.as_mut() else {
                    return Err("workflow runner stdin is unavailable".to_string());
                };
                write_rpc_response(stdin, id, &result).await?;
            }
            RunnerStdoutFrame::Event { event } => {
                state.lock().await.events.push(event);
            }
            RunnerStdoutFrame::Output { output } => {
                state.lock().await.output = Some(output);
            }
        }
    }
    Ok(())
}

async fn handle_runner_rpc(
    rpc_id: u64,
    method: String,
    mut params: Value,
    state: &Arc<Mutex<RunnerProcessState>>,
    bridge: Option<&Arc<dyn WorkflowRuntimeBridge>>,
) -> Result<Value, WorkflowRuntimeError> {
    match method.as_str() {
        "agent.spawn" => {
            let agent_id = string_param(&params, "id")?;
            if let Some(binding) = state.lock().await.bindings.get(&agent_id).cloned() {
                return serde_json::to_value(binding)
                    .map_err(|err| WorkflowRuntimeError::invalid_request(err.to_string()));
            }
            let result = call_bridge(rpc_id, method, params, state, bridge).await?;
            let binding: WorkflowAgentBinding =
                serde_json::from_value(result.clone()).map_err(|err| {
                    WorkflowRuntimeError::invalid_request(format!(
                        "agent.spawn returned invalid binding: {err}"
                    ))
                })?;
            state
                .lock()
                .await
                .bindings
                .insert(binding.agent_id.clone(), binding);
            Ok(result)
        }
        "agent.followup" | "agent.wait" => {
            let agent_id = string_param(&params, "id")?;
            let binding = state
                .lock()
                .await
                .bindings
                .get(&agent_id)
                .cloned()
                .ok_or_else(|| {
                    WorkflowRuntimeError::invalid_request(format!(
                        "workflow agent `{agent_id}` is not bound"
                    ))
                })?;
            params["target"] = Value::String(binding.agent_path);
            call_bridge(rpc_id, method, params, state, bridge).await
        }
        "shell.exec" => call_bridge(rpc_id, method, params, state, bridge).await,
        _ => Err(WorkflowRuntimeError::unsupported(format!(
            "unsupported workflow runtime method `{method}`"
        ))),
    }
}

async fn call_bridge(
    rpc_id: u64,
    method: String,
    params: Value,
    state: &Arc<Mutex<RunnerProcessState>>,
    bridge: Option<&Arc<dyn WorkflowRuntimeBridge>>,
) -> Result<Value, WorkflowRuntimeError> {
    let Some(bridge) = bridge else {
        return Err(WorkflowRuntimeError::unsupported(
            "workflow runtime bridge is not bound in this entrypoint",
        ));
    };
    let (run_id, workflow_id) = {
        let state = state.lock().await;
        (state.run_id.clone(), state.workflow_id.clone())
    };
    bridge
        .call(WorkflowRuntimeRequest {
            run_id,
            workflow_id,
            rpc_id,
            method,
            params,
        })
        .await
}

fn string_param(params: &Value, field: &str) -> Result<String, WorkflowRuntimeError> {
    params
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            WorkflowRuntimeError::invalid_request(format!("workflow RPC missing `{field}`"))
        })
}

async fn write_rpc_response(
    stdin: &mut (impl tokio::io::AsyncWrite + Unpin),
    id: u64,
    result: &Result<Value, WorkflowRuntimeError>,
) -> Result<(), String> {
    let line = match result {
        Ok(result) => serde_json::to_string(&RunnerStdinFrame::RpcResult { id, result }),
        Err(error) => serde_json::to_string(&RunnerStdinFrame::RpcError { id, error }),
    }
    .map_err(|err| format!("failed to serialize workflow RPC response: {err}"))?;
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|err| format!("failed to write workflow RPC response: {err}"))?;
    stdin
        .write_all(b"\n")
        .await
        .map_err(|err| format!("failed to write workflow RPC response newline: {err}"))?;
    stdin
        .flush()
        .await
        .map_err(|err| format!("failed to flush workflow RPC response: {err}"))
}

pub(crate) async fn write_workflow_shim(snapshot_dir: &std::path::Path) -> Result<(), String> {
    tokio::fs::create_dir_all(snapshot_dir.join("node_modules/@codex/workflow"))
        .await
        .map_err(|err| format!("failed to create workflow shim directory: {err}"))?;
    tokio::fs::write(
        snapshot_dir.join("node_modules/@codex/workflow/package.json"),
        r#"{"type":"module","main":"index.js","types":"index.d.ts"}"#,
    )
    .await
    .map_err(|err| format!("failed to write workflow shim package: {err}"))?;
    tokio::fs::write(
        snapshot_dir.join("node_modules/@codex/workflow/index.js"),
        "export function defineWorkflow(definition) { return definition; }\n",
    )
    .await
    .map_err(|err| format!("failed to write workflow shim: {err}"))?;
    tokio::fs::write(
        snapshot_dir.join("node_modules/@codex/workflow/index.d.ts"),
        WORKFLOW_SHIM_TYPES,
    )
    .await
    .map_err(|err| format!("failed to write workflow shim types: {err}"))
}

pub(crate) async fn write_bootstrap(path: &std::path::Path) -> Result<(), String> {
    tokio::fs::write(path, BOOTSTRAP_SOURCE)
        .await
        .map_err(|err| format!("failed to write workflow runner bootstrap: {err}"))
}

const BOOTSTRAP_SOURCE: &str = r#"
import readline from "node:readline";

const runInput = JSON.parse(process.argv[2] ?? "{}");
const originalConsole = globalThis.console;
globalThis.console = {
  ...originalConsole,
  log: (...args) => originalConsole.error(...args),
  info: (...args) => originalConsole.error(...args),
  warn: (...args) => originalConsole.error(...args),
  error: (...args) => originalConsole.error(...args),
};

let nextRpcId = 1;
const pending = new Map();
const rl = readline.createInterface({ input: process.stdin });
rl.on("line", (line) => {
  let frame;
  try {
    frame = JSON.parse(line);
  } catch (error) {
    originalConsole.error(`invalid workflow host frame: ${error.message}`);
    return;
  }
  const waiter = pending.get(frame.id);
  if (!waiter) {
    originalConsole.error(`received workflow host frame for unknown rpc id ${frame.id}`);
    return;
  }
  pending.delete(frame.id);
  if (frame.type === "rpc_result") {
    waiter.resolve(frame.result);
  } else if (frame.type === "rpc_error") {
    const error = new Error(frame.error?.message ?? "workflow runtime RPC failed");
    error.code = frame.error?.code ?? "error";
    waiter.reject(error);
  } else {
    waiter.reject(new Error(`unsupported workflow host frame type ${frame.type}`));
  }
});

function sendFrame(frame) {
  process.stdout.write(`${JSON.stringify(frame)}\n`);
}

function rpc(method, params) {
  const id = nextRpcId;
  nextRpcId += 1;
  sendFrame({ type: "rpc", id, method, params });
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
  });
}

const module = await import(new URL(runInput.entry, import.meta.url).href);
const definition = module.default;

if (!definition || typeof definition.run !== "function") {
  throw new Error("workflow entry must default-export defineWorkflow({ run })");
}

const runtimeEvents = [];
const wf = {
  inputs: runInput.inputs ?? null,
  runId: runInput.runId,
  workflowId: runInput.workflowId,
  mode: runInput.mode,
  revision: runInput.revision,
  emit(event) {
    runtimeEvents.push(event);
    sendFrame({ type: "event", event });
  },
  async Agent(id, options) {
    const binding = await rpc("agent.spawn", { id, options: options ?? {} });
    const agent = {
      id,
      options,
      binding,
      async wait() {
        return await rpc("agent.wait", { id });
      },
      async followup(message) {
        return await rpc("agent.followup", { id, message });
      }
    };
    runtimeEvents.push({ type: "agentBound", id, options, binding });
    sendFrame({ type: "event", event: { type: "agentBound", id, options, binding } });
    return agent;
  },
  async shell(command) {
    return await rpc("shell.exec", { command });
  }
};

const result = await definition.run(wf);
sendFrame({
  type: "output",
  output: {
    result: result ?? null,
    events: runtimeEvents,
    staticGraph: definition.staticGraph ?? null
  }
});
rl.close();
"#;

const WORKFLOW_SHIM_TYPES: &str = r#"
export interface WorkflowDefinition {
  id?: string;
  version?: string;
  staticGraph?: unknown;
  run(runtime: WorkflowRuntime): unknown | Promise<unknown>;
}

export interface WorkflowRuntime {
  inputs: unknown;
  runId: string;
  workflowId: string;
  mode: "start" | "resume";
  revision: number;
  emit(event: unknown): void;
  Agent(id: string, options: WorkflowAgentOptions): Promise<WorkflowAgent>;
  shell(command: unknown): Promise<WorkflowShellResult>;
}

export interface WorkflowAgentOptions {
  parent?: string;
  type?: string;
  agent_type?: string;
  cwd?: string;
  message: string;
  model?: string;
  reasoningEffort?: string;
  reasoning_effort?: string;
  serviceTier?: string;
  service_tier?: string;
  agentMode?: string;
  agent_mode?: string;
  forkTurns?: string;
  fork_turns?: string;
}

export interface WorkflowAgent {
  id: string;
  options: WorkflowAgentOptions;
  binding: WorkflowAgentBinding;
  wait(): Promise<unknown>;
  followup(message: string): Promise<unknown>;
}

export interface WorkflowAgentBinding {
  agentId: string;
  agentPath: string;
  workflowId?: string;
  runId?: string;
  stageId?: string;
  threadId?: string;
  status?: unknown;
  options: unknown;
}

export interface WorkflowShellResult {
  status?: string;
  output?: unknown;
}

export function defineWorkflow<T extends WorkflowDefinition>(definition: T): T;
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_agent_binding_deserializes_legacy_snapshot_without_metadata() {
        let binding: WorkflowAgentBinding = serde_json::from_value(serde_json::json!({
            "agentId": "owner",
            "agentPath": "/root/workflow_wf_1_owner",
            "options": { "message": "implement" }
        }))
        .expect("legacy binding should deserialize");

        assert_eq!(binding.agent_id, "owner");
        assert_eq!(binding.agent_path, "/root/workflow_wf_1_owner");
        assert_eq!(binding.workflow_id, None);
        assert_eq!(binding.run_id, None);
        assert_eq!(binding.stage_id, None);
    }
}
