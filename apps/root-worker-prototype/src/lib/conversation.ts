import type {
  ConversationCell,
  ConversationEntry,
  ResponseItem,
  Thread,
  ThreadItem,
} from "../types";
import {
  buildConversationCells,
  type ConversationCellBuildOptions,
} from "./conversationCompact";
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

const AGENT_STATUS_PREVIEW_MAX_CHARS = 120;
const STRUCTURED_TOOL_OUTPUT_NAMES = new Set([
  "command_wait",
  "command_write_stdin",
  "wait_agent",
]);

export function buildConversationEntries(
  thread: Thread | null,
  options?: ConversationCellBuildOptions,
): ConversationEntry[] {
  return buildConversationState(thread, undefined, options).entries;
}

export function buildConversationState(
  thread: Thread | null,
  previous?: ConversationBuildState | null,
  options?: ConversationCellBuildOptions,
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
  const commandLookup = buildCommandLookup(thread);
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
      const rebuiltEntries =
        previousFlatItem &&
        previousFlatItem.id === item.id &&
        previousFlatItem.item === item &&
        previousFlatItem.timestamp === timestamp
          ? previousFlatItem.entries
          : buildConversationItemEntries(item, {
              author,
              timestamp,
              commandLookup,
            }).map(
              (entry) => ({
                ...entry,
                turnId: turn.id,
              }),
            );

      flatItems.push({
        id: item.id,
        item,
        timestamp,
        entries: rebuiltEntries,
      });
      entries.push(...rebuiltEntries);
      if (item.type === "contextCompaction") {
        entries.length = 0;
      }
      flatItemIndex += 1;
    }
  }
  return {
    threadId: thread.id,
    author,
    flatItems,
    entries,
    cells: buildConversationCells(entries, previous?.cells, options),
  };
}

function buildConversationItemEntries(
  item: ThreadItem,
  {
    author,
    timestamp,
    commandLookup,
  }: {
    author: string;
    timestamp: string;
    commandLookup: Map<string, string>;
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

  if (item.type === "commandExecutionNotification") {
    return [
      {
        id: item.id,
        kind: "event" as const,
        author,
        role: "system" as const,
        text: summarizeCommandExecutionNotification(item, commandLookup),
        timestamp,
        attachments: [],
      },
    ];
  }

  if (item.type === "commandWait") {
    return [
      {
        id: item.id,
        kind: "event" as const,
        author,
        role: "system" as const,
        text: summarizeCommandWait(item),
        timestamp,
        attachments: [],
      },
    ];
  }

  if (item.type === "commandWriteStdin") {
    return [
      {
        id: item.id,
        kind: "event" as const,
        author,
        role: "system" as const,
        text: summarizeCommandWriteStdin(item),
        timestamp,
        attachments: [],
      },
    ];
  }

  if (item.type === "builtinToolCall") {
    return [
      {
        id: item.id,
        kind: "tool" as const,
        author,
        role: "system" as const,
        text: summarizeBuiltinToolCall(item),
        timestamp,
        attachments: [],
        toolName: item.tool,
        toolStatus: item.status,
        toolDetails: formatStructuredToolDetails(item.arguments, item.output),
        toolCategory: toolCategoryForName(item.tool),
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
      buildCollabAgentMessageEntry(item, {
        id: item.id,
        author,
        timestamp,
      }),
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
        toolCategory: "subagentNotification",
      },
    ];
  }

  if (item.type === "contextCompaction") {
    return [];
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
    return [
      {
        id: item.id,
        kind: "event" as const,
        author,
        role: "system" as const,
        text: text || "Reasoning item received.",
        timestamp,
        attachments: [],
      },
    ];
  }

  if (item.type === "eventCommandCall") {
    return [
      {
        id: item.id,
        kind: "tool" as const,
        author,
        role: "system" as const,
        text: summarizeEventCommandCall(item),
        timestamp,
        attachments: [],
        toolName: item.label?.trim() || item.command,
        toolStatus: item.status,
        toolDetails: formatEventCommandCallDetails(item),
        toolCategory: "eventDrivenSubscription",
      },
    ];
  }

  if (item.type === "eventCommandEvent") {
    return [
      {
        id: item.id,
        kind: "event" as const,
        author,
        role: "system" as const,
        text: summarizeEventCommandEvent(item),
        timestamp,
        attachments: [],
      },
    ];
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
              : toolCategoryForName(item.tool, item.namespace),
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

  if (item.type === "workflowRunProgress") {
    return [
      {
        id: item.id,
        kind: "tool" as const,
        author,
        role: "system" as const,
        text: summarizeWorkflowRunProgress(item),
        timestamp,
        attachments: [],
        toolName: `Workflow · ${item.event.workflowId}`,
        toolStatus: workflowRunProgressStatus(item),
        toolDetails: formatWorkflowRunProgressDetails(item),
        toolCategory: "workflow",
      },
    ];
  }

  if (item.type === "threadGoalUpdate") {
    return [
      {
        id: item.id,
        kind: "event" as const,
        author,
        role: "system" as const,
        text: summarizeThreadGoalUpdate(item),
        timestamp,
        attachments: [],
        toolName: "Goal",
        toolStatus: item.goal.status,
        toolDetails: formatThreadGoalUpdateDetails(item),
        toolCategory: "goal",
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

  return [
    {
      id: item.id,
      kind: "event" as const,
      author,
      role: "system" as const,
      text: `Unsupported thread item: ${item.type}`,
      timestamp,
      attachments: [],
    },
  ];
}

function formatItemTimestamp(item: ThreadItem) {
  if (
    (item.type === "commandExecutionNotification" ||
      item.type === "commandWait" ||
      item.type === "commandWriteStdin") &&
    Number.isFinite(item.createdAtMs)
  ) {
    return formatClockTime(item.createdAtMs / 1000);
  }

  if (
    item.type === "workflowRunProgress" &&
    Number.isFinite(item.event.updatedAt)
  ) {
    return formatClockTime(item.event.updatedAt);
  }

  if (item.type === "eventCommandEvent" && Number.isFinite(item.createdAt)) {
    return formatClockTime(item.createdAt);
  }

  const timestampMs = item.completedAtMs ?? item.startedAtMs;
  return timestampMs === null || timestampMs === undefined
    ? null
    : formatClockTime(timestampMs / 1000);
}

function buildReplacementHistoryEntries(
  items: ResponseItem[],
  {
    author,
    timestamp,
    parentId,
  }: {
    author: string;
    timestamp: string;
    parentId: string;
  },
): ConversationEntry[] {
  const hiddenFunctionOutputCallIds = collectStructuredToolOutputCallIds(items);
  const hiddenFunctionCallIds = collectStructuredToolCallIdsWithTypedDisplay(items);
  return items.flatMap((item, index) => {
    if (
      item.type === "function_call" &&
      hiddenFunctionCallIds.has(stringOrFallback(item.call_id, ""))
    ) {
      return [];
    }
    if (
      item.type === "function_call_output" &&
      hiddenFunctionOutputCallIds.has(stringOrFallback(item.call_id, ""))
    ) {
      return [];
    }
    const entry = buildReplacementHistoryEntry(item, {
      author,
      timestamp,
      id: `${parentId}:replacement:${index}`,
      index,
    });
    return entry
      ? [
          {
            ...entry,
            isReplacementHistory: true,
          },
        ]
      : [];
  });
}

function buildReplacementHistoryEntry(
  item: ResponseItem,
  {
    author,
    timestamp,
    id,
    index,
  }: {
    author: string;
    timestamp: string;
    id: string;
    index: number;
  },
): ConversationEntry | null {
  switch (item.type) {
    case "message": {
      const role = stringOrFallback(item.role, "message");
      const text = extractResponseContentText(item.content);
      return {
        id,
        kind: "message",
        author: formatReplacementMessageAuthor(role, author),
        role: role === "user" ? "user" : role === "assistant" ? "agent" : "system",
        text:
          text || `Replacement history ${index + 1}: empty ${role} message.`,
        timestamp,
        attachments: [],
      };
    }
    case "reasoning":
      return {
        id,
        kind: "event",
        author,
        role: "system",
        text:
          extractReasoningText(item.summary) ||
          extractReasoningText(item.content) ||
          "Reasoning item included in compacted model context.",
        timestamp,
        attachments: [],
      };
    case "function_call":
      if (isStructuredToolOutputName(item.name)) {
        return structuredToolCallReplacementEntry(item, {
          id,
          author,
          timestamp,
          index,
        });
      }
      return replacementToolEntry({
        id,
        author,
        timestamp,
        toolName: formatFunctionToolName(item),
        text: `Function call ${stringOrFallback(item.call_id, `#${index + 1}`)}`,
        details: formatResponseItemDetails([
          ["Type", item.type],
          ["Call ID", item.call_id],
          ["Name", item.name],
          ["Namespace", item.namespace],
          ["Arguments", parseMaybeJsonString(item.arguments)],
        ]),
      });
    case "function_call_output":
      return replacementToolEntry({
        id,
        author,
        timestamp,
        toolName: "function output",
        text: `Function output ${stringOrFallback(item.call_id, `#${index + 1}`)}`,
        details: formatResponseItemDetails([
          ["Type", item.type],
          ["Call ID", item.call_id],
          ["Output", item.output],
        ]),
      });
    case "command_wait":
      return replacementToolEntry({
        id,
        author,
        timestamp,
        toolName: "command wait",
        text: `Command wait ${stringOrFallback(item.status, `#${index + 1}`)}`,
        details: formatResponseItemDetails([
          ["Type", item.type],
          ["Command ID", item.command_id],
          ["Status", item.status],
          ["Notification", item.notification],
          ["Exit code", item.exit_code],
          ["Wall time", formatSecondsDuration(Number(item.wall_time_seconds))],
          [
            "Wait timeout",
            formatMillisecondsDuration(Number(item.wait_timeout_ms)),
          ],
        ]),
      });
    case "command_write_stdin":
      return replacementToolEntry({
        id,
        author,
        timestamp,
        toolName: "command stdin",
        text: `Command stdin ${stringOrFallback(item.command_id, `#${index + 1}`)}`,
        details: formatResponseItemDetails([
          ["Type", item.type],
          ["Command ID", item.command_id],
          ["Bytes written", item.bytes_written],
          ["Contains newline", item.contains_newline],
        ]),
      });
    case "custom_tool_call":
      return replacementToolEntry({
        id,
        author,
        timestamp,
        toolName: stringOrFallback(item.name, "custom tool"),
        text: `Custom tool call ${stringOrFallback(item.call_id, `#${index + 1}`)}`,
        details: formatResponseItemDetails([
          ["Type", item.type],
          ["Status", item.status],
          ["Call ID", item.call_id],
          ["Input", item.input],
        ]),
      });
    case "custom_tool_call_output":
      return replacementToolEntry({
        id,
        author,
        timestamp,
        toolName: stringOrFallback(item.name, "custom tool output"),
        text: `Custom tool output ${stringOrFallback(item.call_id, `#${index + 1}`)}`,
        details: formatResponseItemDetails([
          ["Type", item.type],
          ["Call ID", item.call_id],
          ["Output", item.output],
        ]),
      });
    case "local_shell_call":
      return replacementToolEntry({
        id,
        author,
        timestamp,
        toolName: "local shell",
        text: `Local shell call ${stringOrFallback(item.call_id, `#${index + 1}`)}`,
        details: formatResponseItemDetails([
          ["Type", item.type],
          ["Status", item.status],
          ["Action", item.action],
        ]),
      });
    case "tool_search_call":
    case "tool_search_output":
    case "web_search_call":
    case "image_generation_call":
      return replacementToolEntry({
        id,
        author,
        timestamp,
        toolName: formatResponseItemType(item.type),
        text: summarizeResponseToolItem(item, index),
        details: formatRawJson(item),
      });
    case "compaction":
      return {
        id,
        kind: "event",
        author,
        role: "system",
        text: "Compaction summary item included in compacted model context.",
        timestamp,
        attachments: [],
      };
    case "context_compaction":
      return {
        id,
        kind: "compact",
        author,
        role: "system",
        text: "Nested context compaction item included in compacted model context.",
        timestamp,
        attachments: [],
        replacementHistoryEntries: null,
        replacementHistoryStatus: "missing",
        replacementHistoryCount: null,
      };
    default:
      return replacementToolEntry({
        id,
        author,
        timestamp,
        toolName: `unsupported ${formatResponseItemType(item.type)}`,
        text: "Unsupported replacement history item. Raw data is preserved below.",
        details: formatRawJson(item),
      });
  }
}

function collectStructuredToolOutputCallIds(items: ResponseItem[]) {
  const callIds = new Set<string>();
  for (const item of items) {
    if (item.type !== "function_call") {
      continue;
    }
    const name = stringOrNull(item.name);
    const callId = stringOrNull(item.call_id);
    if (name && callId && STRUCTURED_TOOL_OUTPUT_NAMES.has(name)) {
      callIds.add(callId);
    }
  }
  return callIds;
}

function collectStructuredToolCallIdsWithTypedDisplay(items: ResponseItem[]) {
  const typedCommandWaitIds = new Set(
    items
      .filter((item) => item.type === "command_wait")
      .map((item) => stringOrNumberId(item.command_id))
      .filter((id): id is string => id !== null),
  );
  const typedCommandWriteStdinIds = new Set(
    items
      .filter((item) => item.type === "command_write_stdin")
      .map((item) => stringOrNumberId(item.command_id))
      .filter((id): id is string => id !== null),
  );
  const callIds = new Set<string>();
  for (const item of items) {
    if (item.type !== "function_call") {
      continue;
    }
    const name = stringOrNull(item.name);
    const callId = stringOrNull(item.call_id);
    if (!callId) {
      continue;
    }
    const commandId = commandIdFromFunctionCall(item);
    if (
      name === "command_wait" &&
      commandId &&
      typedCommandWaitIds.has(commandId)
    ) {
      callIds.add(callId);
    }
    if (
      name === "command_write_stdin" &&
      commandId &&
      typedCommandWriteStdinIds.has(commandId)
    ) {
      callIds.add(callId);
    }
  }
  return callIds;
}

function commandIdFromFunctionCall(item: ResponseItem) {
  const args = parseMaybeJsonString(item.arguments);
  if (!args || typeof args !== "object" || Array.isArray(args)) {
    return null;
  }
  return stringOrNumberId((args as Record<string, unknown>).command_id);
}

function isStructuredToolOutputName(name: unknown) {
  const value = stringOrNull(name);
  return value !== null && STRUCTURED_TOOL_OUTPUT_NAMES.has(value);
}

function structuredToolCallReplacementEntry(
  item: ResponseItem,
  {
    id,
    author,
    timestamp,
    index,
  }: {
    id: string;
    author: string;
    timestamp: string;
    index: number;
  },
) {
  const name = stringOrFallback(item.name, "wait tool");
  const args = parseMaybeJsonString(item.arguments);
  const argsRecord =
    args && typeof args === "object" && !Array.isArray(args)
      ? (args as Record<string, unknown>)
      : {};
  const target = argsRecord.target ?? argsRecord.command_id;
  const targetText = stringOrNumberId(target);
  return replacementToolEntry({
    id,
    author,
    timestamp,
    toolName: name.replaceAll("_", " "),
    text: `${formatResponseItemType(name)} ${targetText ?? `#${index + 1}`}`,
    details: formatResponseItemDetails([
      ["Tool", name],
      ["Target", target],
      ["Call ID", item.call_id],
    ]),
  });
}

function buildCollabAgentMessageEntry(
  item: Extract<ThreadItem, { type: "collabAgentMessage" }>,
  {
    id,
    author,
    timestamp,
  }: {
    id: string;
    author: string;
    timestamp: string;
  },
): ConversationEntry {
  return {
    id,
    kind: "tool",
    author,
    role: "system",
    text: summarizeCollabAgentMessage(item),
    timestamp,
    attachments: [],
    toolName: formatCollabAgentMessageTitle(item),
    toolStatus: "completed",
    toolDetails: formatCollabAgentMessageDetails(item),
    toolCategory: formatCollabAgentMessageCategory(item),
  };
}

function replacementToolEntry({
  id,
  author,
  timestamp,
  toolName,
  text,
  details,
}: {
  id: string;
  author: string;
  timestamp: string;
  toolName: string;
  text: string;
  details: string;
}): ConversationEntry {
  return {
    id,
    kind: "tool",
    author,
    role: "system",
    text,
    timestamp,
    attachments: [],
    toolName,
    toolStatus: "context",
    toolDetails: details,
    toolCategory: "context",
  };
}

function formatReplacementMessageAuthor(role: string, author: string) {
  if (role === "user") {
    return "You";
  }
  if (role === "assistant") {
    return author;
  }
  return `${role} context`;
}

function extractResponseContentText(content: unknown) {
  if (!Array.isArray(content)) {
    return "";
  }
  return content
    .map((item) => {
      if (!item || typeof item !== "object") {
        return "";
      }
      const record = item as Record<string, unknown>;
      return stringOrNull(record.text) ?? stringOrNull(record.content) ?? "";
    })
    .filter((text) => text.trim().length > 0)
    .join("\n")
    .trim();
}

function extractReasoningText(value: unknown) {
  if (!Array.isArray(value)) {
    return "";
  }
  return value
    .map((item) => {
      if (typeof item === "string") {
        return item;
      }
      if (!item || typeof item !== "object") {
        return "";
      }
      const record = item as Record<string, unknown>;
      return stringOrNull(record.text) ?? stringOrNull(record.summary) ?? "";
    })
    .filter((text) => text.trim().length > 0)
    .join("\n")
    .trim();
}

function formatFunctionToolName(item: ResponseItem) {
  const name = stringOrFallback(item.name, "function");
  const namespace = stringOrNull(item.namespace);
  return namespace ? `${namespace}/${name}` : name;
}

function summarizeResponseToolItem(item: ResponseItem, index: number) {
  if (item.type === "web_search_call") {
    const action = item.action;
    if (action && typeof action === "object" && !Array.isArray(action)) {
      const query = stringOrNull((action as Record<string, unknown>).query);
      if (query) {
        return `Web search for ${query}`;
      }
    }
  }
  return `Replacement history ${index + 1}: ${formatResponseItemType(item.type)}`;
}

function formatResponseItemType(type: string) {
  return type.replaceAll("_", " ");
}

function formatResponseItemDetails(sections: Array<[string, unknown]>) {
  return sections
    .filter(([, value]) => value !== null && value !== undefined && value !== "")
    .map(([label, value]) => `${label}\n${formatUnknownValue(value)}`)
    .join("\n\n");
}

function parseMaybeJsonString(value: unknown) {
  if (typeof value !== "string") {
    return value;
  }
  try {
    return JSON.parse(value);
  } catch {
    return value;
  }
}

function formatRawJson(value: unknown) {
  return `Raw item\n${formatUnknownValue(value)}`;
}

function formatUnknownValue(value: unknown) {
  if (typeof value === "string") {
    return value;
  }
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function arrayOfStrings(value: unknown) {
  return Array.isArray(value)
    ? value.map((item) => stringOrFallback(item, "unknown"))
    : [];
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

function summarizeBuiltinToolCall(
  item: Extract<ThreadItem, { type: "builtinToolCall" }>,
) {
  if (item.tool === "poll_event") {
    const output =
      item.output && typeof item.output === "object" && !Array.isArray(item.output)
        ? (item.output as Record<string, unknown>)
        : null;
    const error = stringOrNull(output?.error);
    const sourceHint = stringOrNull(output?.sourceHint);
    if (item.status === "failed" || error) {
      return error ? `poll_event • failed: ${error}` : "poll_event • failed";
    }
    if (output?.timedOut === true) {
      return "poll_event • timeout";
    }
    if (sourceHint) {
      return `poll_event • ${sourceHint}`;
    }
    return "poll_event • woke";
  }
  const details = extractEventDrivenSummaryDetails(item.tool, item.arguments);
  return details ? `${item.tool} • ${details}` : item.tool;
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
    case "listAgents":
    case "list_agents":
      return item.receiverPaths.length > 0
        ? `listed ${item.receiverPaths.length} agents`
        : "list_agents";
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

function summarizeEventCommandCall(
  item: Extract<ThreadItem, { type: "eventCommandCall" }>,
) {
  const label = stringOrNull(item.label);
  return label ? `${label} • ${item.command}` : item.command;
}

function summarizeEventDrivenTool(
  item: Extract<ThreadItem, { type: "eventDrivenTool" }>,
) {
  const text = stringOrNull(item.text);
  if (!text) {
    return item.title;
  }

  const details = parseEventDrivenToolDetails(text);
  return details.capturedOutput !== null
    ? firstNonEmptyLine(details.summary) || item.title
    : text;
}

function formatEventDrivenToolDetails(
  item: Extract<ThreadItem, { type: "eventDrivenTool" }>,
) {
  const details = parseEventDrivenToolDetails(item.text);
  const sections = [
    `Tool\n${item.tool}`,
    `Event\n${item.title}`,
    `Details\n${details.summary}`,
  ];

  if (details.capturedOutput !== null && details.capturedOutput.length > 0) {
    sections.push(
      `Captured output\n${previewCapturedOutput(details.capturedOutput)}`,
    );
  }

  return sections.join("\n\n");
}

function summarizeThreadGoalUpdate(
  item: Extract<ThreadItem, { type: "threadGoalUpdate" }>,
) {
  const objective = firstNonEmptyLine(item.goal.objective) ?? "thread goal";
  switch (item.action) {
    case "created":
      return `Goal created: ${objective}`;
    case "updated":
      return `Goal updated: ${objective}`;
    case "paused":
      return `Goal paused: ${objective}`;
    case "resumed":
      return `Goal resumed: ${objective}`;
    case "budgetLimited":
      return `Goal budget reached: ${objective}`;
    case "completed":
      return `Goal completed: ${objective}`;
    default:
      return `Goal changed: ${objective}`;
  }
}

function summarizeWorkflowRunProgress(
  item: Extract<ThreadItem, { type: "workflowRunProgress" }>,
) {
  return (
    firstNonEmptyLine(item.event.message) ??
    formatWorkflowRunProgressKind(item.event.kind)
  );
}

function workflowRunProgressStatus(
  item: Extract<ThreadItem, { type: "workflowRunProgress" }>,
) {
  switch (item.event.kind) {
    case "started":
    case "resumed":
      return "running";
    case "completed":
      return "completed";
    case "failed":
      return "failed";
    case "aborted":
      return "aborted";
    default:
      return "unknown";
  }
}

function formatWorkflowRunProgressDetails(
  item: Extract<ThreadItem, { type: "workflowRunProgress" }>,
) {
  return formatResponseItemDetails([
    ["Workflow", item.event.workflowId],
    ["Run", item.event.runId],
    ["Progress", formatWorkflowRunProgressKind(item.event.kind)],
    ["Runner Status", item.event.runnerStatus],
    ["Message", item.event.message],
    ["Run Status", item.event.status],
    ["Graph", "No graph details in this update."],
  ]);
}

function formatWorkflowRunProgressKind(kind: string) {
  switch (kind) {
    case "started":
      return "Workflow started";
    case "resumed":
      return "Workflow resumed";
    case "completed":
      return "Workflow completed";
    case "failed":
      return "Workflow failed";
    case "aborted":
      return "Workflow aborted";
    default:
      return "Workflow updated";
  }
}

function formatThreadGoalUpdateDetails(
  item: Extract<ThreadItem, { type: "threadGoalUpdate" }>,
) {
  const sections = [
    `Objective\n${item.goal.objective}`,
    `Status\n${formatThreadGoalStatus(item.goal.status)}`,
    `Source\n${formatThreadGoalUpdateSource(item.source)}`,
  ];

  if (item.previousStatus) {
    sections.push(`Previous Status\n${formatThreadGoalStatus(item.previousStatus)}`);
  }

  if (item.goal.tokenBudget !== null && item.goal.tokenBudget !== undefined) {
    sections.push(
      `Token Usage\n${formatThreadGoalNumber(item.goal.tokensUsed)} / ${formatThreadGoalNumber(item.goal.tokenBudget)}`,
    );
  } else if (item.goal.tokensUsed > 0) {
    sections.push(`Token Usage\n${formatThreadGoalNumber(item.goal.tokensUsed)}`);
  }

  if (item.goal.timeUsedSeconds > 0) {
    sections.push(`Time Used\n${formatThreadGoalDuration(item.goal.timeUsedSeconds)}`);
  }

  return sections.join("\n\n");
}

function formatThreadGoalStatus(status: string) {
  switch (status) {
    case "active":
      return "Active";
    case "paused":
      return "Paused";
    case "budgetLimited":
      return "Budget limited";
    case "complete":
      return "Complete";
    default:
      return status;
  }
}

function formatThreadGoalUpdateSource(source: string) {
  switch (source) {
    case "modelTool":
      return "Model tool";
    case "client":
      return "Client";
    case "system":
      return "System";
    default:
      return source;
  }
}

function formatThreadGoalNumber(value: number) {
  return new Intl.NumberFormat("en-US").format(value);
}

function formatThreadGoalDuration(totalSeconds: number) {
  const seconds = Math.max(0, Math.floor(totalSeconds));
  const minutes = Math.floor(seconds / 60);
  const remainingSeconds = seconds % 60;
  if (minutes <= 0) {
    return `${remainingSeconds}s`;
  }
  const hours = Math.floor(minutes / 60);
  const remainingMinutes = minutes % 60;
  if (hours <= 0) {
    return `${minutes}m ${remainingSeconds}s`;
  }
  return `${hours}h ${remainingMinutes}m`;
}

const CAPTURED_OUTPUT_MARKER = "\nCaptured output:\n";
const CAPTURED_OUTPUT_PREVIEW_MAX_LINES = 12;
const CAPTURED_OUTPUT_PREVIEW_MAX_CHARS = 2000;

function parseEventDrivenToolDetails(text: string) {
  const markerIndex = text.indexOf(CAPTURED_OUTPUT_MARKER);
  if (markerIndex === -1) {
    return {
      summary: text,
      capturedOutput: null,
    };
  }

  return {
    summary: text.slice(0, markerIndex).trim() || "Event completed.",
    capturedOutput: text
      .slice(markerIndex + CAPTURED_OUTPUT_MARKER.length)
      .trim(),
  };
}

function firstNonEmptyLine(text: string) {
  return text
    .split(/\r?\n/u)
    .map((line) => line.trim())
    .find((line) => line.length > 0);
}

function previewCapturedOutput(output: string) {
  const lines = output.split(/\r?\n/u);
  let preview = lines.slice(0, CAPTURED_OUTPUT_PREVIEW_MAX_LINES).join("\n");
  let omitted = lines.length > CAPTURED_OUTPUT_PREVIEW_MAX_LINES;

  if (preview.length > CAPTURED_OUTPUT_PREVIEW_MAX_CHARS) {
    preview = preview.slice(0, CAPTURED_OUTPUT_PREVIEW_MAX_CHARS).trimEnd();
    omitted = true;
  }

  return omitted
    ? `${preview}\n… omitted additional captured output`
    : preview;
}

function formatCollabAgentToolName(
  tool: Extract<ThreadItem, { type: "collabAgentToolCall" }>["tool"],
) {
  switch (tool) {
    case "spawnAgent":
      return "spawn_agent";
    case "sendInput":
    case "resumeAgent":
      return "followup_task";
    case "wait":
      return "wait_agent";
    case "listAgents":
    case "list_agents":
      return "list_agents";
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
    case "resumeAgent":
      return "followup task";
    case "wait":
      return "wait for agent";
    case "listAgents":
    case "list_agents":
      return "list agents";
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
    case "send_message":
    case "followupTask":
      return `Received follow-up from ${senderPath}.`;
    case "childCompletion": {
      const preview = previewInlineText(
        item.content,
        AGENT_STATUS_PREVIEW_MAX_CHARS,
      );
      return preview
        ? `Received child completion from ${senderPath}: ${preview}`
        : `Received child completion from ${senderPath}.`;
    }
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

function formatCollabAgentMessageCategory(
  item: Extract<ThreadItem, { type: "collabAgentMessage" }>,
): ConversationEntry["toolCategory"] {
  return item.operation === "childCompletion" ? "childCompletion" : "multiAgent";
}

function formatCollabAgentMessageDetails(
  item: Extract<ThreadItem, { type: "collabAgentMessage" }>,
) {
  const message = stringOrNull(item.content) ?? "…";
  const sections = [
    `Operation\n${formatCollabAgentMessageOperation(item.operation)}`,
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

function formatCollabAgentMessageOperation(operation: string) {
  return operation === "sendMessage" || operation === "send_message"
    ? "followupTask"
    : operation;
}

function summarizeCollabAgentStatusUpdate(
  item: Extract<ThreadItem, { type: "collabAgentStatusUpdate" }>,
) {
  const agentPath =
    stringOrNull(item.status.path) ??
    stringOrNull(item.senderPath) ??
    "unknown";
  const message = previewInlineText(
    item.status.message,
    AGENT_STATUS_PREVIEW_MAX_CHARS,
  );
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

  if (item.initialWaitMs !== null && item.initialWaitMs !== undefined) {
    sections.push(`Initial Wait\n${item.initialWaitMs} ms`);
  }

  if (item.notifyOn) {
    sections.push(`Notify On\n${item.notifyOn}`);
  }

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

function summarizeCommandExecutionNotification(
  item: Extract<ThreadItem, { type: "commandExecutionNotification" }>,
  commandLookup: Map<string, string>,
) {
  const commandLabel = commandLookup.get(item.commandItemId) ?? item.commandItemId;
  if (item.kind === "output") {
    const output = stringOrNull(item.output);
    return output
      ? `Command output notification received for ${commandLabel}: ${output}`
      : `Command output notification received for ${commandLabel}.`;
  }

  if (item.kind === "exit") {
    const exitCode =
      item.exitCode === null || item.exitCode === undefined
        ? "unknown exit"
        : `exit ${item.exitCode}`;
    return `Command exit notification received for ${commandLabel}: ${exitCode}.`;
  }

  return item.message || `Command notification received for ${commandLabel}.`;
}

function buildCommandLookup(thread: Thread) {
  const commandLookup = new Map<string, string>();
  for (const turn of thread.turns) {
    for (const item of turn.items) {
      if (item.type === "commandExecution") {
        commandLookup.set(item.id, item.command);
      }
    }
  }
  return commandLookup;
}

function summarizeCommandWait(item: Extract<ThreadItem, { type: "commandWait" }>) {
  const notification = item.notification
    ? ` after ${item.notification} notification`
    : "";
  const exitCode =
    item.exitCode === null || item.exitCode === undefined
      ? ""
      : `, exit ${item.exitCode}`;
  const seconds = Number.isFinite(item.wallTimeSeconds)
    ? ` in ${formatSecondsDuration(item.wallTimeSeconds)}`
    : "";
  const timeout = Number.isFinite(item.waitTimeoutMs)
    ? ` with timeout ${formatMillisecondsDuration(item.waitTimeoutMs)}`
    : "";
  return `Waited for command ${item.commandId}${timeout}${notification}: ${item.status}${exitCode}${seconds}.`;
}

function formatSecondsDuration(totalSeconds: number) {
  if (!Number.isFinite(totalSeconds) || totalSeconds <= 0) {
    return "0s";
  }

  const totalMilliseconds = Math.round(totalSeconds * 1000);
  return formatMillisecondsDuration(totalMilliseconds);
}

function formatMillisecondsDuration(totalMilliseconds: number) {
  if (!Number.isFinite(totalMilliseconds) || totalMilliseconds <= 0) {
    return "0ms";
  }

  if (totalMilliseconds < 1000) {
    return `${Math.round(totalMilliseconds)}ms`;
  }

  const roundedSeconds = Math.round(totalMilliseconds / 1000);
  if (roundedSeconds >= 60) {
    const minutes = Math.floor(roundedSeconds / 60);
    const remainingSeconds = roundedSeconds % 60;
    if (remainingSeconds === 0) {
      return `${minutes}m`;
    }
    return `${minutes}m ${remainingSeconds}s`;
  }

  const seconds = totalMilliseconds / 1000;
  return `${formatDurationNumber(seconds)}s`;
}

function formatDurationNumber(value: number) {
  if (Number.isInteger(value)) {
    return value.toString();
  }
  return value.toFixed(2).replace(/\.?0+$/u, "");
}

function summarizeCommandWriteStdin(
  item: Extract<ThreadItem, { type: "commandWriteStdin" }>,
) {
  const suffix = item.containsNewline ? " including newline" : "";
  return `Wrote ${item.bytesWritten} bytes to command ${item.commandId}${suffix}.`;
}

function summarizeEventCommandEvent(
  item: Extract<ThreadItem, { type: "eventCommandEvent" }>,
) {
  const label = stringOrNull(item.label) ?? item.command;
  switch (item.kind) {
    case "output":
      return item.line ? `${label}: ${item.line}` : `${label}: output received.`;
    case "exited": {
      if (item.signal) {
        return `${label}: signal ${item.signal}.`;
      }
      const exitCode =
        item.exitCode === null || item.exitCode === undefined
          ? "unknown exit"
          : `exit ${item.exitCode}`;
      return `${label}: ${exitCode}.`;
    }
    case "cancelled":
      return `${label}: cancelled.`;
    case "failedToStart":
      return item.message
        ? `${label}: failed to start. ${item.message}`
        : `${label}: failed to start.`;
    default:
      return item.message ? `${label}: ${item.message}` : `${label}: ${item.kind}.`;
  }
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

function formatEventCommandCallDetails(
  item: Extract<ThreadItem, { type: "eventCommandCall" }>,
) {
  const sections = [`Command\n${item.command}`];

  const cwd = stringOrNull(item.cwd);
  if (cwd) {
    sections.push(`Directory\n${cwd}`);
  }

  const label = stringOrNull(item.label);
  if (label) {
    sections.push(`Label\n${label}`);
  }

  sections.push(`Subscription\n${item.subscriptionId}`);

  if (item.output !== null && item.output !== undefined) {
    sections.push(`Output\n${safeJson(item.output)}`);
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

function toolCategoryForName(tool: string, namespace?: string | null) {
  if (
    tool === "process_exit_subscribe" ||
    tool === "fs_subscribe" ||
    tool === "schedule_subscribe" ||
    tool === "process_exit_unsubscribe" ||
    tool === "fs_unsubscribe" ||
    tool === "schedule_unsubscribe"
  ) {
    return "eventDrivenSubscription" as const;
  }
  void namespace;
  return "external" as const;
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

function previewInlineText(value: unknown, maxChars: number) {
  const text = stringOrNull(value)?.replace(/\s+/g, " ") ?? null;
  if (!text) {
    return null;
  }
  const chars = Array.from(text);
  return chars.length > maxChars
    ? `${chars.slice(0, maxChars).join("").trimEnd()}…`
    : text;
}

function stringOrNull(value: unknown) {
  return typeof value === "string" && value.trim().length > 0
    ? value.trim()
    : null;
}

function stringOrNumberId(value: unknown) {
  if (typeof value === "number" && Number.isFinite(value)) {
    return String(value);
  }
  return stringOrNull(value);
}

function stringOrFallback(value: unknown, fallback: string) {
  return stringOrNull(value) ?? fallback;
}
