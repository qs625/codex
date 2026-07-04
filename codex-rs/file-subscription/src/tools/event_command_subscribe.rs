use std::sync::Arc;

use codex_extension_api::ExtensionToolOutput;
use codex_extension_api::FunctionCallError;
use codex_extension_api::JsonToolOutput;
use codex_extension_api::ToolCall;
use codex_extension_api::ToolExecutor;
use codex_extension_api::ToolName;
use codex_extension_api::ToolSpec;
use protocol::ThreadId;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::registry::FsSubscriptionRegistry;

use super::parse_args;
use super::subscription_function_tool;

const TOOL_NAME: &str = "event_command_subscribe";
const TOOL_DESCRIPTION: &str = "Run a shell command in the background and inject an event \
whenever stdout data is read from the command. Each stdout read chunk becomes a separate event and \
may contain multiple lines. Use this \
for file monitors, long-running command monitors, command-exit notifications, or server log \
watchers. The command is restarted automatically when the thread resumes from persisted monitor \
metadata.\n\n\
Parameters:\n\
- `command`: shell command to run. Each stdout read chunk becomes an event.\n\
- `cwd`: optional working directory for the command. When omitted, the command inherits the \
server process working directory.\n\
- `label`: optional short name included in active monitor lists and emitted events.\n\n\
Use this when:\n\
- You need to watch files or directories and wake the model only after meaningful changes. Add \
debounce in the command so rapid changes collapse into one event.\n\
- You need to wait for a long command to finish and receive its exit code or a short stdout \
summary as an event instead of polling.\n\
- You need to confirm that a very long-running command is still active; have the monitor command \
emit heartbeat or progress lines instead of repeatedly checking status from outside.\n\
- You need to keep a server or watcher running and only wake the model on readiness, errors, \
crashes, or other important log lines.\n\n\
Command-writing guidance:\n\
- Keep monitored commands quiet: redirect noisy output and only print data that should wake the \
model. A single output event can contain multiple stdout lines.\n\
- `stderr` does not emit events unless the command redirects it to `stdout`, for example with \
`2>&1`.\n\
- Prefer a small shell pipeline first; if the logic is awkward in shell, embed a short `python` \
or `node` script inside the command to express debounce, parsing, or summarization.\n\
- If you need liveness for a long-running command, print heartbeat or progress lines from inside \
the monitor command itself.\n\
- Runtime completion is tied to the main process exiting, not to every inherited `stdout` handle \
closing. If your command starts child processes and you want the monitor to stay active until they \
finish, make the script `wait` for them before it exits.\n\
- Do not fall back to frequent polling, `sleep` loops, or repeated status checks when a monitor \
command can wait and emit the relevant event.\n\n\
Example monitor commands:\n\
- File watch with debounce: `while inotifywait -qq -e close_write src/lib.rs; do now=$(date \
+%s); if [ \"${last:-0}\" -ne \"$now\" ]; then last=$now; echo \"src/lib.rs changed\"; fi; \
done`\n\
- Long command exit with status + summary: `log=$(mktemp); cargo test -p codex-tui >\"$log\" \
2>&1; status=$?; tail -n 20 \"$log\"; echo \"EXIT:$status\"`\n\
- Long command with heartbeat + exit summary: `log=$(mktemp); (while sleep 30; do echo \
\"heartbeat: cargo test still running\"; done) & hb=$!; cargo test -p codex-tui >\"$log\" 2>&1; \
status=$?; kill \"$hb\"; tail -n 20 \"$log\"; echo \"EXIT:$status\"`\n\
- Server readiness/error watcher: `npm run dev 2>&1 | while IFS= read -r line; do case \"$line\" \
in *\"ready\"*|*\"ERROR\"*|*\"panic\"*|*\"crash\"*) echo \"$line\";; esac; done`\n\
- Python log filter helper: `npm run dev 2>&1 | python - <<'PY'\n\
import re, sys\n\
pattern = re.compile(r'ready|error|panic|crash', re.IGNORECASE)\n\
for line in sys.stdin:\n\
    if pattern.search(line):\n\
        print(line.rstrip(), flush=True)\n\
PY`\n\
- Node log filter: `node -e \"process.stdin.setEncoding('utf8'); let buf=''; \
process.stdin.on('data', chunk => { buf += chunk; let lines = buf.split(/\\n/); buf = lines.pop(); \
for (const line of lines) { if (/ready|error|crash/i.test(line)) console.log(line); } });\" < \
server.log`\n\n\
Example requests:\n\
- \"Watch this file and tell me once the writes settle down.\"\n\
- \"Wait for this build to finish and wake me with the exit code.\"\n\
- \"Keep this long job under watch and emit a heartbeat every 30 seconds until it exits.\"\n\
- \"Keep an eye on the dev server and notify me when it is ready or if it crashes.\"\n\n\
Use `event_command_unsubscribe` to cancel a running monitor.";

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EventCommandSubscribeArgs {
    /// Shell command to run in the background. Each stdout read chunk becomes an event.
    command: String,
    /// Optional working directory for the command.
    cwd: Option<String>,
    /// Optional short label included in the active monitor and emitted events.
    label: Option<String>,
}

#[derive(Serialize)]
struct EventCommandSubscribeResult {
    subscription_id: String,
    command: String,
    cwd: Option<String>,
    label: Option<String>,
}

pub(crate) struct EventCommandSubscribeTool {
    pub(crate) thread_id: ThreadId,
    pub(crate) registry: Arc<FsSubscriptionRegistry>,
}

impl ToolExecutor<ToolCall> for EventCommandSubscribeTool {
    type Output = ExtensionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(subscription_function_tool::<EventCommandSubscribeArgs>(
            TOOL_NAME,
            TOOL_DESCRIPTION,
        ))
    }

    fn handle<'a>(
        &'a self, call: ToolCall,
    ) -> codex_extension_api::ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
                    let args: EventCommandSubscribeArgs = parse_args(&call)?;
                    if args.command.trim().is_empty() {
                        return Err(FunctionCallError::RespondToModel(
                            "command must not be empty".to_string(),
                        ));
                    }
                    let subscription_id = Uuid::now_v7().to_string();
                    self.registry
                        .subscribe_event_command(
                            self.thread_id,
                            args.command.clone(),
                            args.cwd.clone(),
                            args.label.clone(),
                            subscription_id.clone(),
                        )
                        .await;
                    Ok(JsonToolOutput::new(json!(EventCommandSubscribeResult {
                        subscription_id,
                        command: args.command,
                        cwd: args.cwd,
                        label: args.label,
                    })))
            })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use codex_extension_api::ResponsesApiTool;
    use codex_extension_api::ToolSpec;
    use codex_file_watcher::FileWatcher;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::runtime::UnavailableFileSubscriptionThreadRuntime;

    fn test_tool() -> EventCommandSubscribeTool {
        EventCommandSubscribeTool {
            thread_id: ThreadId::new(),
            registry: Arc::new(FsSubscriptionRegistry::new(
                Arc::new(FileWatcher::noop()),
                Arc::new(UnavailableFileSubscriptionThreadRuntime),
                None,
            )),
        }
    }

    #[test]
    fn spec_includes_monitoring_guidance_examples() {
        let spec = test_tool().spec().expect("tool spec");
        let ToolSpec::Function(ResponsesApiTool { description, .. }) = spec else {
            panic!("expected function tool spec");
        };

        assert_eq!(description, TOOL_DESCRIPTION.to_string());
        assert!(description.contains("File watch with debounce"));
        assert!(description.contains("EXIT:$status"));
        assert!(description.contains("heartbeat: cargo test still running"));
        assert!(description.contains("ready or if it crashes"));
        assert!(description.contains("short `python` or `node` script"));
        assert!(description.contains("heartbeat or progress lines"));
        assert!(description.contains("main process exiting"));
        assert!(description.contains("make the script `wait` for them"));
        assert!(description.contains("Do not fall back to frequent polling"));
        assert!(description.contains("`stderr` does not emit events"));
        assert!(description.contains("event_command_unsubscribe"));
    }
}
