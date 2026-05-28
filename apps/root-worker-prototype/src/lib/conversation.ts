import type { ConversationCell, ConversationEntry, Thread, ThreadItem } from "../types";
import { formatClockTime, getThreadLabel, trimPath, trimThreadId } from "./thread";

export function buildConversationEntries(thread: Thread | null): ConversationEntry[] {
  if (!thread) {
    return [];
  }

  const author = getThreadLabel(thread);

  return thread.turns.flatMap((turn) =>
    turn.items.flatMap<ConversationEntry>((item) => {
      const timestamp = formatClockTime(turn.completedAt ?? turn.startedAt ?? thread.updatedAt);

      if (item.type === "userMessage") {
        const text = item.content
          .filter((content) => content.type === "text")
          .map((content) => content.text ?? "")
          .join("\n")
          .trim();
        const skillAttachments = item.content
          .filter((content) => content.type === "skill")
          .map((content) => ({
            kind: "file" as const,
            label: `/${content.name ?? "skill"}`,
            path: content.path,
          }));
        const imageAttachments = item.content
          .filter((content) => content.type === "image")
          .map((content, index) => ({
            kind: "image" as const,
            label: content.name ?? `Image ${index + 1}`,
            url: content.image_url,
          }));
        return [
          {
            id: item.id,
            kind: "message" as const,
            author: "You",
            role: "user" as const,
            text:
              text ||
              (skillAttachments.length > 0
                ? `Activated ${skillAttachments.length} skill${skillAttachments.length === 1 ? "" : "s"}.`
                : "") ||
              (imageAttachments.length > 0
                ? `Attached ${imageAttachments.length} image${imageAttachments.length === 1 ? "" : "s"}.`
                : ""),
            timestamp,
            attachments: [...skillAttachments, ...imageAttachments],
          },
        ];
      }

      if (item.type === "agentMessage") {
        return [
          {
            id: item.id,
            kind: "message" as const,
            author,
            role: "agent" as const,
            text: item.text || "…",
            timestamp,
            attachments: [],
          },
        ];
      }

      if (item.type === "injectedContext") {
        return [
          {
            id: item.id,
            kind: "tool" as const,
            author,
            role: "system" as const,
            text: item.preview,
            timestamp,
            attachments: [],
            toolName: item.title,
            toolStatus: "completed",
            toolDetails: formatInjectedContextDetails(item),
            toolCategory: "context",
          },
        ];
      }

      if (item.type === "fileChange") {
        return [
          {
            id: item.id,
            kind: "message" as const,
            author,
            role: "agent" as const,
            text: summarizeFileChanges(item),
            timestamp,
            attachments: item.changes.map((change) => ({
              kind: "file" as const,
              label: `${change.path} ${formatDeltaKind(change.kind)}`,
            })),
          },
        ];
      }

      if (item.type === "commandExecution") {
        return [
          {
            id: item.id,
            kind: "tool" as const,
            author,
            role: "system" as const,
            text: summarizeCommandExecution(item),
            timestamp,
            attachments: [],
            toolName: item.command,
            toolStatus: item.status,
            toolDetails: formatCommandExecutionDetails(item),
            toolCategory: "command",
          },
        ];
      }

      if (item.type === "collabAgentToolCall") {
        return [
          {
            id: item.id,
            kind: "tool" as const,
            author,
            role: "system" as const,
            text: summarizeCollabAgentToolCall(item),
            timestamp,
            attachments: [],
            toolName: formatCollabAgentToolTitle(item),
            toolStatus: item.status,
            toolDetails: formatCollabAgentToolDetails(item),
            toolCategory: "multiAgent",
          },
        ];
      }

      if (item.type === "collabAgentMessage") {
        return [
          {
            id: item.id,
            kind: "tool" as const,
            author,
            role: "system" as const,
            text: summarizeCollabAgentMessage(item),
            timestamp,
            attachments: [],
            toolName: formatCollabAgentMessageTitle(item),
            toolStatus: "completed",
            toolDetails: formatCollabAgentMessageDetails(item),
            toolCategory: "multiAgent",
          },
        ];
      }

      if (item.type === "collabAgentStatusUpdate") {
        return [
          {
            id: item.id,
            kind: "tool" as const,
            author,
            role: "system" as const,
            text: summarizeCollabAgentStatusUpdate(item),
            timestamp,
            attachments: [],
            toolName: formatCollabAgentStatusUpdateTitle(item),
            toolStatus: "completed",
            toolDetails: formatCollabAgentStatusUpdateDetails(item),
            toolCategory: "multiAgent",
          },
        ];
      }

      if (item.type === "plan") {
        return [
          {
            id: item.id,
            kind: "event" as const,
            author,
            role: "system" as const,
            text: item.text,
            timestamp,
            attachments: [],
          },
        ];
      }

      if (item.type === "reasoning") {
        const text = item.summary.join("\n").trim() || item.content.join("\n").trim();
        return text
          ? [
              {
                id: item.id,
                kind: "event" as const,
                author,
                role: "system" as const,
                text,
                timestamp,
                attachments: [],
              },
            ]
          : [];
      }

      if (
        item.type === "dynamicToolCall" ||
        item.type === "mcpToolCall" ||
        item.type === "builtinToolCall"
      ) {
        const details =
          item.type === "dynamicToolCall"
            ? formatStructuredToolDetails(item.arguments, item.contentItems)
            : item.type === "builtinToolCall"
              ? formatStructuredToolDetails(item.arguments, item.output)
              : formatStructuredToolDetails(item.arguments, item.result ?? item.error);
        return [
          {
            id: item.id,
            kind: "tool" as const,
            author,
            role: "system" as const,
            text: summarizeToolCall(item),
            timestamp,
            attachments: [],
            toolName: item.tool,
            toolStatus: item.status,
            toolDetails: details,
            toolCategory:
              item.type === "builtinToolCall"
                ? "builtin"
                : item.type === "mcpToolCall"
                  ? "external"
                  : "external",
          },
        ];
      }

      if (item.type === "webSearch") {
        return [
          {
            id: item.id,
            kind: "event" as const,
            author,
            role: "system" as const,
            text: `Searched for ${item.query}`,
            timestamp,
            attachments: [],
          },
        ];
      }

      if (item.type === "imageGeneration") {
        const label = item.savedPath ? basename(item.savedPath) : "Generated image";
        const promptText = item.revisedPrompt?.trim();
        const captionText =
          promptText && promptText.length > 0
            ? promptText
            : item.savedPath
              ? `Generated ${label}.`
              : "Generating image…";
        return [
          {
            id: item.id,
            kind: "message" as const,
            author,
            role: "agent" as const,
            text: captionText,
            timestamp,
            attachments: item.savedPath
              ? [
                  {
                    kind: "image" as const,
                    label,
                    path: item.savedPath,
                  },
                ]
              : [],
          },
        ];
      }

      if (item.type === "imageView") {
        const label = item.path ? basename(item.path) : "Viewed image";
        return [
          {
            id: item.id,
            kind: "message" as const,
            author,
            role: "agent" as const,
            text: `Viewed ${label}.`,
            timestamp,
            attachments: item.path
              ? [
                  {
                    kind: "image" as const,
                    label,
                    path: item.path,
                  },
                ]
              : [],
          },
        ];
      }

      if (item.type === "enteredReviewMode" || item.type === "exitedReviewMode") {
        return [
          {
            id: item.id,
            kind: "event" as const,
            author,
            role: "system" as const,
            text: item.review,
            timestamp,
            attachments: [],
          },
        ];
      }

      return [];
    }),
  );
}

export function buildConversationCells(entries: ConversationEntry[]): ConversationCell[] {
  return entries.reduce<ConversationCell[]>((cells, entry) => {
    const previousCell = cells.at(-1);
    if (previousCell && shouldMergeConversationEntry(previousCell, entry)) {
      previousCell.entries.push(entry);
      return cells;
    }

    cells.push({
      id: entry.id,
      kind: entry.kind,
      entries: [entry],
    });
    return cells;
  }, []);
}

function shouldMergeConversationEntry(cell: ConversationCell, nextEntry: ConversationEntry) {
  const previousEntry = cell.entries.at(-1);
  if (!previousEntry) {
    return false;
  }

  if (cell.kind === "tool" && nextEntry.kind === "tool") {
    return previousEntry.toolCategory === nextEntry.toolCategory;
  }

  if (
    cell.kind === "message" &&
    nextEntry.kind === "message" &&
    previousEntry.role === "agent" &&
    nextEntry.role === "agent"
  ) {
    return true;
  }

  return false;
}

function summarizeFileChanges(item: Extract<ThreadItem, { type: "fileChange" }>) {
  const count = item.changes.length;
  if (count === 0) {
    return "Applied file changes.";
  }
  if (count === 1) {
    return `Updated ${trimPath(item.changes[0].path)}.`;
  }
  return `Updated ${count} files.`;
}

function formatDeltaKind(kind: string) {
  if (kind === "added") {
    return "+0";
  }
  if (kind === "deleted") {
    return "-0";
  }
  return "edited";
}

function summarizeToolCall(
  item: Extract<ThreadItem, { type: "dynamicToolCall" | "mcpToolCall" | "builtinToolCall" }>,
) {
  if (item.type === "mcpToolCall") {
    return `${item.server}/${item.tool}`;
  }
  if (item.type === "builtinToolCall") {
    return item.tool;
  }
  if (item.namespace) {
    return `${item.namespace}/${item.tool}`;
  }
  return item.tool;
}

function summarizeCollabAgentToolCall(
  item: Extract<ThreadItem, { type: "collabAgentToolCall" }>,
) {
  const receiverLabel =
    item.receiverPaths.length === 1
      ? item.receiverPaths[0]
      : `${item.receiverPaths.length} workers`;
  const prompt = item.prompt?.trim();

  switch (item.tool) {
    case "spawnAgent":
      return prompt || `Spawned ${receiverLabel}.`;
    case "sendInput":
      return prompt || `Sent message to ${receiverLabel}.`;
    case "resumeAgent":
      return prompt || `Queued follow-up for ${receiverLabel}.`;
    case "wait":
      return `Waiting on ${receiverLabel}.`;
    case "closeAgent":
      return `Closed ${receiverLabel}.`;
    default:
      return prompt || `${item.tool} for ${receiverLabel}.`;
  }
}

function formatCollabAgentToolName(tool: Extract<ThreadItem, { type: "collabAgentToolCall" }>["tool"]) {
  switch (tool) {
    case "spawnAgent":
      return "spawn_agent";
    case "sendInput":
      return "send_message";
    case "resumeAgent":
      return "followup_task";
    case "wait":
      return "wait_agent";
    case "closeAgent":
      return "close_agent";
    default:
      return tool;
  }
}

function formatCollabAgentToolTitle(item: Extract<ThreadItem, { type: "collabAgentToolCall" }>) {
  switch (item.tool) {
    case "spawnAgent":
      return "spawn agent";
    case "sendInput":
      return "send message";
    case "resumeAgent":
      return "followup task";
    case "wait":
      return "wait for agent";
    case "closeAgent":
      return "close agent";
    default:
      return formatCollabAgentToolName(item.tool);
  }
}

function formatCollabAgentToolDetails(
  item: Extract<ThreadItem, { type: "collabAgentToolCall" }>,
) {
  const sections = [
    `Tool\n${formatCollabAgentToolName(item.tool)}`,
    `Sender\n${item.senderPath}`,
  ];

  if (item.receiverPaths.length > 0) {
    sections.push(
      `Receivers\n${item.receiverPaths.join("\n")}`,
    );
  }

  if (item.prompt?.trim()) {
    sections.push(`Prompt\n${item.prompt.trim()}`);
  }

  if (item.model) {
    sections.push(`Model\n${item.model}`);
  }

  if (item.reasoningEffort) {
    sections.push(`Reasoning\n${item.reasoningEffort}`);
  }

  const agentStates = Object.entries(item.agentsStates ?? {});
  if (agentStates.length > 0) {
    sections.push(
      `Agent States\n${agentStates
        .map(([threadId, state]) =>
          [state.path?.trim() || trimThreadId(threadId), state.status, state.message?.trim()]
            .filter((value) => value && value.length > 0)
            .join(" • "),
        )
        .join("\n")}`,
    );
  }

  return sections.join("\n\n");
}

function summarizeCollabAgentMessage(item: Extract<ThreadItem, { type: "collabAgentMessage" }>) {
  switch (item.operation) {
    case "spawnAgent":
      return `Received initial task from ${item.senderPath}.`;
    case "sendMessage":
      return `Received message from ${item.senderPath}.`;
    case "followupTask":
      return `Received follow-up from ${item.senderPath}.`;
    case "childCompletion":
      return `Received child completion from ${item.senderPath}.`;
    default:
      return `Received agent message from ${item.senderPath}.`;
  }
}

function formatCollabAgentMessageTitle(item: Extract<ThreadItem, { type: "collabAgentMessage" }>) {
  if (item.operation === "childCompletion") {
    return `${resolveAgentPath(item.senderPath, item.recipientPath)} subagent completion`;
  }
  return `received from ${resolveAgentPath(item.senderPath, item.recipientPath)}`;
}

function formatCollabAgentMessageDetails(
  item: Extract<ThreadItem, { type: "collabAgentMessage" }>,
) {
  const sections = [
    `Operation\n${item.operation}`,
    `From\n${item.senderPath}`,
    `To\n${item.recipientPath}`,
    `Message\n${item.content.trim() || "…"}`,
    `Trigger Turn\n${item.triggerTurn ? "true" : "false"}`,
  ];

  if (item.otherRecipientPaths.length > 0) {
    sections.push(`Other Recipients\n${item.otherRecipientPaths.join("\n")}`);
  }

  return sections.join("\n\n");
}

function summarizeCollabAgentStatusUpdate(
  item: Extract<ThreadItem, { type: "collabAgentStatusUpdate" }>,
) {
  const agentPath = item.status.path?.trim() || item.senderPath;
  const message = item.status.message?.trim();
  return [agentPath, item.status.status, message].filter((value) => value && value.length > 0).join(" • ");
}

function formatCollabAgentStatusUpdateTitle(
  item: Extract<ThreadItem, { type: "collabAgentStatusUpdate" }>,
) {
  const agentPath = resolveAgentPath(item.status.path, item.senderPath);
  return item.status.status === "completed"
    ? `${agentPath} subagent completion`
    : `status from ${agentPath}`;
}

function formatCollabAgentStatusUpdateDetails(
  item: Extract<ThreadItem, { type: "collabAgentStatusUpdate" }>,
) {
  const sections = [
    `From\n${item.senderPath}`,
    `To\n${item.recipientPath}`,
    `Status\n${item.status.status}`,
  ];

  if (item.status.path?.trim()) {
    sections.push(`Agent\n${item.status.path.trim()}`);
  }

  if (item.status.message?.trim()) {
    sections.push(`Message\n${item.status.message.trim()}`);
  }

  return sections.join("\n\n");
}

function summarizeCommandExecution(item: Extract<ThreadItem, { type: "commandExecution" }>) {
  const cwd = trimPath(item.cwd);
  const exitCode =
    item.exitCode === null || item.exitCode === undefined ? "running" : `exit ${item.exitCode}`;
  return `${cwd} • ${exitCode}`;
}

function formatCommandExecutionDetails(item: Extract<ThreadItem, { type: "commandExecution" }>) {
  const sections = [`Command\n${item.command}`, `Cwd\n${item.cwd}`, `Status\n${item.status}`];

  if (item.durationMs !== null && item.durationMs !== undefined) {
    sections.push(`Duration\n${item.durationMs} ms`);
  }

  if (item.exitCode !== null && item.exitCode !== undefined) {
    sections.push(`Exit Code\n${item.exitCode}`);
  }

  if (item.aggregatedOutput?.trim()) {
    sections.push(`Output\n${item.aggregatedOutput.trim()}`);
  }

  return sections.join("\n\n");
}

function formatStructuredToolDetails(input: unknown, output: unknown) {
  const sections: string[] = [];

  if (input !== null && input !== undefined) {
    sections.push(`Arguments\n${safeJson(input)}`);
  }

  if (output !== null && output !== undefined) {
    sections.push(`Output\n${safeJson(output)}`);
  }

  return sections.join("\n\n");
}

function formatInjectedContextDetails(item: Extract<ThreadItem, { type: "injectedContext" }>) {
  return item.sections
    .map((section) => `${section.label}\n${section.text.trim()}`)
    .join("\n\n");
}

function basename(filePath: string) {
  const normalized = filePath.replace(/\\/g, "/").replace(/\/+$/u, "");
  const lastSlash = normalized.lastIndexOf("/");
  return lastSlash === -1 ? normalized : normalized.slice(lastSlash + 1) || normalized;
}

function safeJson(value: unknown) {
  if (typeof value === "string") {
    return value;
  }

  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function resolveAgentPath(...paths: Array<string | null | undefined>) {
  return paths.map((path) => path?.trim()).find((path) => path && path.length > 0) ?? "unknown";
}
