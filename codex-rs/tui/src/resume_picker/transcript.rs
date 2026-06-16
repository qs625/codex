use std::sync::Arc;

use crate::app_server_session::AppServerSession;
use crate::git_action_directives::parse_assistant_markdown;
use crate::history_cell::AgentMarkdownCell;
use crate::history_cell::HistoryCell;
use crate::history_cell::PlainHistoryCell;
use crate::history_cell::ReasoningSummaryCell;
use crate::history_cell::UserHistoryCell;
use codex_app_server_protocol::Thread;
use codex_app_server_protocol::ThreadItem;
use codex_protocol::ThreadId;
use codex_protocol::items::UserMessageItem;
use ratatui::style::Stylize as _;
use ratatui::text::Line;

pub(crate) type TranscriptCells = Vec<Arc<dyn HistoryCell>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RawReasoningVisibility {
    Hidden,
    Visible,
}

pub(crate) async fn load_session_transcript(
    app_server: &mut AppServerSession,
    thread_id: ThreadId,
    raw_reasoning_visibility: RawReasoningVisibility,
) -> std::io::Result<TranscriptCells> {
    let thread = app_server
        .thread_read(thread_id, /*include_turns*/ true)
        .await
        .map_err(std::io::Error::other)?;
    Ok(thread_to_transcript_cells(
        &thread,
        raw_reasoning_visibility,
    ))
}

pub(crate) fn thread_to_transcript_cells(
    thread: &Thread,
    raw_reasoning_visibility: RawReasoningVisibility,
) -> TranscriptCells {
    let cwd = thread.cwd.as_path();
    let mut cells: TranscriptCells = Vec::new();
    for item in thread.turns.iter().flat_map(|turn| turn.items.iter()) {
        match item {
            ThreadItem::UserMessage { id, content } => {
                let item = UserMessageItem {
                    id: id.clone(),
                    content: content
                        .iter()
                        .cloned()
                        .map(codex_app_server_protocol::UserInput::into_core)
                        .collect(),
                };
                cells.push(Arc::new(UserHistoryCell {
                    message: item.message(),
                    text_elements: item.text_elements(),
                    local_image_paths: item.local_image_paths(),
                    remote_image_urls: item.image_urls(),
                }));
            }
            ThreadItem::AgentMessage { text, .. } => {
                let parsed = parse_assistant_markdown(text);
                if !parsed.visible_markdown.trim().is_empty() {
                    cells.push(Arc::new(AgentMarkdownCell::new(
                        parsed.visible_markdown,
                        cwd,
                    )));
                }
            }
            ThreadItem::Plan { text, .. } => {
                if !text.trim().is_empty() {
                    cells.push(Arc::new(crate::history_cell::new_proposed_plan(
                        text.clone(),
                        cwd,
                    )));
                }
            }
            ThreadItem::Reasoning {
                summary, content, ..
            } => {
                let text = if matches!(raw_reasoning_visibility, RawReasoningVisibility::Visible)
                    && !content.is_empty()
                {
                    content.join("\n\n")
                } else {
                    summary.join("\n\n")
                };
                if !text.trim().is_empty() {
                    cells.push(Arc::new(ReasoningSummaryCell::new(
                        "Reasoning".to_string(),
                        text,
                        cwd,
                        /*transcript_only*/ false,
                    )));
                }
            }
            other => {
                if let Some(cell) = fallback_transcript_cell(other) {
                    cells.push(Arc::new(cell));
                }
            }
        }
    }
    if cells.is_empty() {
        cells.push(Arc::new(PlainHistoryCell::new(vec![
            "No transcript content available".italic().dim().into(),
        ])));
    }
    cells
}

fn fallback_transcript_cell(item: &ThreadItem) -> Option<PlainHistoryCell> {
    let lines = match item {
        ThreadItem::HookPrompt { fragments, .. } => fragments
            .iter()
            .map(|fragment| {
                vec![
                    "hook prompt: ".dim(),
                    fragment.text.trim().to_string().into(),
                ]
                .into()
            })
            .collect::<Vec<_>>(),
        ThreadItem::CommandExecution {
            command,
            status,
            aggregated_output,
            exit_code,
            ..
        } => {
            let mut lines: Vec<Line<'static>> =
                vec![vec!["$ ".dim(), command.clone().into()].into()];
            lines.push(
                format!(
                    "status: {status:?}{}",
                    exit_code
                        .map(|code| format!(" · exit {code}"))
                        .unwrap_or_default()
                )
                .dim()
                .into(),
            );
            if let Some(output) = aggregated_output.as_deref()
                && !output.trim().is_empty()
            {
                lines.extend(
                    output
                        .lines()
                        .map(|line| vec!["  ".dim(), line.trim_end().to_string().dim()].into()),
                );
            }
            lines
        }
        ThreadItem::CommandExecutionNotification {
            kind,
            message,
            output,
            exit_code,
            ..
        } => {
            let mut lines: Vec<Line<'static>> = vec![
                format!("command notification: {kind:?} · {message}")
                    .dim()
                    .into(),
            ];
            if let Some(output) = output.as_deref()
                && !output.trim().is_empty()
            {
                lines.extend(
                    output
                        .lines()
                        .map(|line| vec!["  ".dim(), line.trim_end().to_string().dim()].into()),
                );
            }
            if let Some(exit_code) = exit_code {
                lines.push(format!("  exit {exit_code}").dim().into());
            }
            lines
        }
        ThreadItem::CommandWait {
            status,
            notification,
            exit_code,
            wall_time_seconds,
            ..
        } => {
            let mut details = vec![format!("{wall_time_seconds:.1}s")];
            if let Some(notification) = notification {
                details.push(format!("{notification:?}"));
            }
            if let Some(exit_code) = exit_code {
                details.push(format!("exit {exit_code}"));
            }
            vec![
                format!("command wait: {status:?} · {}", details.join(" · "))
                    .dim()
                    .into(),
            ]
        }
        ThreadItem::CommandWriteStdin {
            bytes_written,
            contains_newline,
            ..
        } => {
            let suffix = if *contains_newline {
                " with newline"
            } else {
                ""
            };
            vec![
                format!("command stdin: wrote {bytes_written} bytes{suffix}")
                    .dim()
                    .into(),
            ]
        }
        ThreadItem::FileChange {
            changes, status, ..
        } => vec![
            format!("file changes: {status:?} · {} changes", changes.len())
                .dim()
                .into(),
        ],
        ThreadItem::McpToolCall {
            server,
            tool,
            status,
            ..
        } => vec![
            format!("mcp tool: {server}/{tool} · {status:?}")
                .dim()
                .into(),
        ],
        ThreadItem::DynamicToolCall {
            namespace,
            tool,
            status,
            ..
        } => {
            let name = namespace
                .as_ref()
                .map(|namespace| format!("{namespace}/{tool}"))
                .unwrap_or_else(|| tool.clone());
            vec![format!("tool: {name} · {status:?}").dim().into()]
        }
        ThreadItem::EventDrivenToolCall { tool, status, .. } => {
            vec![
                format!("event-driven tool call: {tool} · {status:?}")
                    .dim()
                    .into(),
            ]
        }
        ThreadItem::EventDrivenTool {
            tool, title, text, ..
        } => {
            vec![
                format!("event-driven tool: {tool} · {title}").dim().into(),
                vec!["  ".dim(), text.clone().dim()].into(),
            ]
        }
        ThreadItem::EventCommandCall {
            command,
            label,
            status,
            ..
        } => {
            let title = label.as_deref().unwrap_or(command);
            vec![format!("event command: {title} · {status:?}").dim().into()]
        }
        ThreadItem::EventCommandEvent {
            command,
            kind,
            label,
            line,
            exit_code,
            signal,
            message,
            truncated,
            ..
        } => {
            let title = label.as_deref().unwrap_or(command);
            let mut lines: Vec<Line<'static>> = vec![
                format!("event command event: {title} · {kind:?}")
                    .dim()
                    .into(),
            ];
            if let Some(line) = line.as_deref()
                && !line.trim().is_empty()
            {
                lines.push(vec!["  ".dim(), line.trim_end().to_string().dim()].into());
            }
            if let Some(message) = message.as_deref()
                && !message.trim().is_empty()
            {
                lines.push(vec!["  ".dim(), message.trim().to_string().dim()].into());
            }
            let mut details = Vec::new();
            if let Some(exit_code) = exit_code {
                details.push(format!("exit {exit_code}"));
            }
            if let Some(signal) = signal.as_deref() {
                details.push(format!("signal {signal}"));
            }
            if *truncated {
                details.push("truncated".to_string());
            }
            if !details.is_empty() {
                lines.push(format!("  {}", details.join(" · ")).dim().into());
            }
            lines
        }
        ThreadItem::InjectedContext { title, preview, .. } => vec![
            format!("injected context: {title} · {}", preview.trim())
                .dim()
                .into(),
        ],
        ThreadItem::CollabAgentMessage {
            operation,
            recipient_path,
            ..
        } => vec![
            format!("agent message: {operation:?} · {recipient_path}")
                .dim()
                .into(),
        ],
        ThreadItem::CollabAgentToolCall { tool, status, .. } => {
            vec![format!("agent tool: {tool:?} · {status:?}").dim().into()]
        }
        ThreadItem::CollabAgentStatusUpdate {
            recipient_path,
            status,
            ..
        } => vec![
            format!("agent status: {} · {:?}", recipient_path, status.status)
                .dim()
                .into(),
        ],
        ThreadItem::WorkflowRunProgress { event, .. } => vec![
            format!(
                "workflow: {} · {} · {:?}",
                event.workflow_id, event.runner_status, event.kind
            )
            .dim()
            .into(),
        ],
        ThreadItem::ThreadGoalUpdate { action, goal, .. } => {
            vec![format!("goal: {:?} · {}", action, goal.objective).dim().into()]
        }
        ThreadItem::WebSearch { query, .. } => {
            vec![vec!["web search: ".dim(), query.clone().into()].into()]
        }
        ThreadItem::ImageView { path, .. } => {
            vec![format!("image: {}", path.as_path().display()).dim().into()]
        }
        ThreadItem::ImageGeneration {
            status, saved_path, ..
        } => {
            let saved = saved_path
                .as_ref()
                .map(|path| format!(" · {}", path.as_path().display()))
                .unwrap_or_default();
            vec![format!("image generation: {status}{saved}").dim().into()]
        }
        ThreadItem::EnteredReviewMode { review, .. } => {
            vec![vec!["review started: ".dim(), review.clone().into()].into()]
        }
        ThreadItem::ExitedReviewMode { review, .. } => {
            vec![vec!["review finished: ".dim(), review.clone().into()].into()]
        }
        ThreadItem::ContextCompaction { .. } => vec!["context locally compacted".dim().into()],
        ThreadItem::UserMessage { .. }
        | ThreadItem::AgentMessage { .. }
        | ThreadItem::Plan { .. }
        | ThreadItem::Reasoning { .. } => return None,
    };
    (!lines.is_empty()).then(|| PlainHistoryCell::new(lines))
}
