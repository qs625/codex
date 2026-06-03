import type {
  ConversationCell,
  ConversationEntry,
  Thread,
  ThreadItem,
} from "../types";
import {
  formatClockTime,
  getThreadLabel,
  trimPath,
  trimThreadId,
} from "./thread";

export type ConversationBuildState = {
  threadId: string | null;
  author: string | null;
  flatItems: ConversationFlatItemState[];
  entries: ConversationEntry[];
  cells: ConversationCell[];
};

type ConversationFlatItemState = {
  id: string;
  item: ThreadItem;
  timestamp: string;
  entries: ConversationEntry[];
};

export function buildConversationEntries(
  thread: Thread | null,
): ConversationEntry[] {
  return buildConversationState(thread).entries;
}

export function buildConversationState(
  thread: Thread | null,
  previous?: ConversationBuildState | null,
): ConversationBuildState {
  if (!thread) {
    return {
      threadId: null,
      author: null,
      flatItems: [],
      entries: [],
      cells: [],
    };
  }

  const author = getThreadLabel(thread);
  const canReusePrevious =
    previous?.threadId === thread.id && previous.author === author;
  const flatItems: ConversationFlatItemState[] = [];
  const entries: ConversationEntry[] = [];
  let flatItemIndex = 0;

  for (const turn of thread.turns) {
    const turnTimestamp = formatClockTime(
      turn.completedAt ?? turn.startedAt ?? thread.updatedAt,
    );

    for (const item of turn.items) {
      const timestamp = formatItemTimestamp(item) ?? turnTimestamp;
      const previousFlatItem = canReusePrevious
        ? previous.flatItems[flatItemIndex]
        : undefined;
      const nextEntries =
        previousFlatItem &&
        previousFlatItem.id === item.id &&
        previousFlatItem.item === item &&
        previousFlatItem.timestamp === timestamp
          ? previousFlatItem.entries
          : buildConversationItemEntries(item, { author, timestamp });

      flatItems.push({
        id: item.id,
        item,
        timestamp,
        entries: nextEntries,
      });
      entries.push(...nextEntries);
      flatItemIndex += 1;
    }
  }

  return {
    threadId: thread.id,
    author,
    flatItems,
    entries,
    cells: buildConversationCells(entries, previous?.cells),
  };
}

function buildConversationItemEntries(
  item: ThreadItem,
  {
    author,
    timestamp,
  }: {
    author: string;
    timestamp: string;
  },
): ConversationEntry[] {
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

  if (item.type === "contextCompaction") {
    return [
      {
        id: item.id,
        kind: "tool" as const,
        author,
        role: "system" as const,
        text: "Conversation history compacted.",
        timestamp,
        attachments: [],
        toolName: "compact context",
        toolStatus: "completed",
        toolDetails:
          "Codex compacted the conversation history for this thread to reduce context usage.",
        toolCategory: "context",
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
    const text =
      item.summary.join("\n").trim() || item.content.join("\n").trim();
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
    item.type === "eventDrivenToolCall"
  ) {
    const details =
      item.type === "dynamicToolCall"
        ? formatStructuredToolDetails(item.arguments, item.contentItems)
        : item.type === "eventDrivenToolCall"
          ? formatStructuredToolDetails(item.arguments, item.output)
          : formatStructuredToolDetails(
              item.arguments,
              item.result ?? item.error,
            );
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
          item.type === "eventDrivenToolCall"
            ? "eventDrivenSubscription"
            : item.type === "mcpToolCall"
              ? "external"
              : "external",
      },
    ];
  }

  if (item.type === "eventDrivenTool") {
    return [
      {
        id: item.id,
        kind: "tool" as const,
        author,
        role: "system" as const,
        text: summarizeEventDrivenTool(item),
        timestamp,
        attachments: [],
        toolName: item.title,
        toolStatus: "completed",
        toolDetails: formatEventDrivenToolDetails(item),
        toolCategory: "eventDrivenEvent",
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
}

function formatItemTimestamp(item: ThreadItem) {
  const timestampMs = item.completedAtMs ?? item.startedAtMs;
  return timestampMs === null || timestampMs === undefined
    ? null
    : formatClockTime(timestampMs / 1000);
}

export function buildConversationCells(
  entries: ConversationEntry[],
  previousCells?: ConversationCell[] | null,
): ConversationCell[] {
  const cells: ConversationCell[] = [];
  let entryIndex = 0;
  let cellIndex = 0;

  while (entryIndex < entries.length) {
    const nextCellEntries = [entries[entryIndex]];
    while (
      entryIndex + nextCellEntries.length < entries.length &&
      shouldMergeConversationEntry(
        {
          id: nextCellEntries[0].id,
          kind: nextCellEntries[0].kind,
          entries: nextCellEntries,
        },
        entries[entryIndex + nextCellEntries.length],
      )
    ) {
      nextCellEntries.push(entries[entryIndex + nextCellEntries.length]);
    }

    const existingCell = previousCells?.[cellIndex];
    if (
      existingCell &&
      existingCell.id === nextCellEntries[0].id &&
      existingCell.kind === nextCellEntries[0].kind &&
      existingCell.entries.length === nextCellEntries.length &&
      existingCell.entries.every(
        (entry, index) => entry === nextCellEntries[index],
      )
    ) {
      cells.push(existingCell);
    } else {
      cells.push({
        id: nextCellEntries[0].id,
        kind: nextCellEntries[0].kind,
        entries: nextCellEntries,
      });
    }

    entryIndex += nextCellEntries.length;
    cellIndex += 1;
  }

  return cells;
}

function shouldMergeConversationEntry(
  cell: ConversationCell,
  nextEntry: ConversationEntry,
) {
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

function summarizeFileChanges(
  item: Extract<ThreadItem, { type: "fileChange" }>,
) {
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
  item: Extract<
    ThreadItem,
    { type: "dynamicToolCall" | "mcpToolCall" | "eventDrivenToolCall" }
  >,
) {
  if (item.type === "mcpToolCall") {
    return `${item.server}/${item.tool}`;
  }
  if (item.type === "eventDrivenToolCall") {
    return summarizeEventDrivenToolCall(item);
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
  const senderLabel = resolveAgentPath(item.senderPath);

  switch (item.tool) {
    case "spawnAgent":
      return `${senderLabel} -> ${receiverLabel}`;
    case "sendInput":
      return `${senderLabel} -> ${receiverLabel}`;
    case "resumeAgent":
      return `${senderLabel} -> ${receiverLabel}`;
    case "wait":
      if (item.receiverPaths.length > 0 && item.timeoutMs !== null && item.timeoutMs !== undefined) {
        return `wait on ${receiverLabel} for ${formatWaitTimeout(item.timeoutMs)}`;
      }
      if (item.receiverPaths.length > 0) {
        return `wait on ${receiverLabel}`;
      }
      return "wait_agent";
    case "closeAgent":
      return `Closed ${receiverLabel}.`;
    default:
      return `${item.tool} for ${receiverLabel}.`;
  }
}

function summarizeEventDrivenToolCall(
  item: Extract<ThreadItem, { type: "eventDrivenToolCall" }>,
) {
  const details = extractEventDrivenSummaryDetails(item.tool, item.arguments);
  return details ? `${item.tool} • ${details}` : item.tool;
}

function summarizeEventDrivenTool(
  item: Extract<ThreadItem, { type: "eventDrivenTool" }>,
) {
  return stringOrNull(item.text) ?? item.title;
}

function formatEventDrivenToolDetails(
  item: Extract<ThreadItem, { type: "eventDrivenTool" }>,
) {
  return [`Tool\n${item.tool}`, `Event\n${item.title}`, `Details\n${item.text}`].join(
    "\n\n",
  );
}

function formatCollabAgentToolName(
  tool: Extract<ThreadItem, { type: "collabAgentToolCall" }>["tool"],
) {
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

function formatCollabAgentToolTitle(
  item: Extract<ThreadItem, { type: "collabAgentToolCall" }>,
) {
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
    `Sender\n${stringOrFallback(item.senderPath, "unknown")}`,
  ];

  if (item.receiverPaths.length > 0) {
    sections.push(
      `Receivers\n${item.receiverPaths
        .map((path) => stringOrFallback(path, "unknown"))
        .join("\n")}`,
    );
  }

  if (item.timeoutMs !== null && item.timeoutMs !== undefined) {
    sections.push(`Timeout\n${formatWaitTimeout(item.timeoutMs)}`);
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
          [
            stringOrNull(state.path) ?? trimThreadId(threadId),
            state.status,
            stringOrNull(state.message),
          ]
            .filter((value) => value && value.length > 0)
            .join(" • "),
        )
        .join("\n")}`,
    );
  }

  return sections.join("\n\n");
}

function formatWaitTimeout(timeoutMs: number) {
  if (timeoutMs % 1000 !== 0) {
    return `${timeoutMs}ms`;
  }

  const totalSeconds = timeoutMs / 1000;
  if (totalSeconds % 60 !== 0) {
    return `${totalSeconds}s`;
  }

  const totalMinutes = totalSeconds / 60;
  if (totalMinutes % 60 !== 0) {
    return `${totalMinutes}m`;
  }

  const totalHours = totalMinutes / 60;
  return `${totalHours}h`;
}

function summarizeCollabAgentMessage(
  item: Extract<ThreadItem, { type: "collabAgentMessage" }>,
) {
  const senderPath = stringOrFallback(item.senderPath, "unknown");
  switch (item.operation) {
    case "spawnAgent":
      return `Received initial task from ${senderPath}.`;
    case "sendMessage":
      return `Received message from ${senderPath}.`;
    case "followupTask":
      return `Received follow-up from ${senderPath}.`;
    case "childCompletion":
      return `Received child completion from ${senderPath}.`;
    default:
      return `Received agent message from ${senderPath}.`;
  }
}

function formatCollabAgentMessageTitle(
  item: Extract<ThreadItem, { type: "collabAgentMessage" }>,
) {
  if (item.operation === "childCompletion") {
    return `${resolveAgentPath(item.senderPath, item.recipientPath)} subagent completion`;
  }
  return `received from ${resolveAgentPath(item.senderPath, item.recipientPath)}`;
}

function formatCollabAgentMessageDetails(
  item: Extract<ThreadItem, { type: "collabAgentMessage" }>,
) {
  const message = stringOrNull(item.content) ?? "…";
  const sections = [
    `Operation\n${item.operation}`,
    `From\n${stringOrFallback(item.senderPath, "unknown")}`,
    `To\n${stringOrFallback(item.recipientPath, "unknown")}`,
    `Message\n${message}`,
    `Trigger Turn\n${item.triggerTurn ? "true" : "false"}`,
  ];

  if (item.otherRecipientPaths.length > 0) {
    sections.push(
      `Other Recipients\n${item.otherRecipientPaths
        .map((path) => stringOrFallback(path, "unknown"))
        .join("\n")}`,
    );
  }

  return sections.join("\n\n");
}

function summarizeCollabAgentStatusUpdate(
  item: Extract<ThreadItem, { type: "collabAgentStatusUpdate" }>,
) {
  const agentPath =
    stringOrNull(item.status.path) ??
    stringOrNull(item.senderPath) ??
    "unknown";
  const message = stringOrNull(item.status.message);
  return [agentPath, item.status.status, message]
    .filter((value) => value && value.length > 0)
    .join(" • ");
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
    `From\n${stringOrFallback(item.senderPath, "unknown")}`,
    `To\n${stringOrFallback(item.recipientPath, "unknown")}`,
    `Status\n${item.status.status}`,
  ];

  const statusPath = stringOrNull(item.status.path);
  if (statusPath) {
    sections.push(`Agent\n${statusPath}`);
  }

  const statusMessage = stringOrNull(item.status.message);
  if (statusMessage) {
    sections.push(`Message\n${statusMessage}`);
  }

  return sections.join("\n\n");
}

function summarizeCommandExecution(
  item: Extract<ThreadItem, { type: "commandExecution" }>,
) {
  const cwd = trimPath(item.cwd);
  const exitCode =
    item.exitCode === null || item.exitCode === undefined
      ? "running"
      : `exit ${item.exitCode}`;
  return `${cwd} • ${exitCode}`;
}

function formatCommandExecutionDetails(
  item: Extract<ThreadItem, { type: "commandExecution" }>,
) {
  const sections = [
    `Command\n${item.command}`,
    `Cwd\n${item.cwd}`,
    `Status\n${item.status}`,
  ];

  if (item.durationMs !== null && item.durationMs !== undefined) {
    sections.push(`Duration\n${item.durationMs} ms`);
  }

  if (item.exitCode !== null && item.exitCode !== undefined) {
    sections.push(`Exit Code\n${item.exitCode}`);
  }

  const aggregatedOutput = stringOrNull(item.aggregatedOutput);
  if (aggregatedOutput) {
    sections.push(`Output\n${aggregatedOutput}`);
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

function extractEventDrivenSummaryDetails(tool: string, args: unknown) {
  if (!args || typeof args !== "object" || Array.isArray(args)) {
    return null;
  }

  const record = args as Record<string, unknown>;
  const label =
    typeof record.label === "string" && record.label.trim().length > 0
      ? record.label.trim()
      : null;

  switch (tool) {
    case "process_exit_subscribe":
      if (label) {
        return `label ${label}`;
      }
      if (typeof record.session_id === "number") {
        return `session ${record.session_id}`;
      }
      return null;
    case "fs_subscribe":
      if (label) {
        return `${label} • ${stringOrNull(record.path) ?? "watch"}`;
      }
      return stringOrNull(record.path);
    case "schedule_subscribe":
      if (label) {
        return `${label} • ${stringOrNull(record.schedule) ?? "schedule"}`;
      }
      return stringOrNull(record.schedule);
    default:
      return label;
  }
}

function formatInjectedContextDetails(
  item: Extract<ThreadItem, { type: "injectedContext" }>,
) {
  return item.sections
    .map((section) => `${section.label}\n${stringOrFallback(section.text, "")}`)
    .join("\n\n");
}

function basename(filePath: string) {
  const normalized = filePath.replace(/\\/g, "/").replace(/\/+$/u, "");
  const lastSlash = normalized.lastIndexOf("/");
  return lastSlash === -1
    ? normalized
    : normalized.slice(lastSlash + 1) || normalized;
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

function resolveAgentPath(...paths: Array<unknown>) {
  return paths.map(stringOrNull).find((path) => path && path.length > 0) ?? "unknown";
}

function stringOrNull(value: unknown) {
  return typeof value === "string" && value.trim().length > 0
    ? value.trim()
    : null;
}

function stringOrFallback(value: unknown, fallback: string) {
  return stringOrNull(value) ?? fallback;
}
