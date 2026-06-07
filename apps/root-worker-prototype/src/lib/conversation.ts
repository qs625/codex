import type {
  ConversationCell,
  ConversationEntry,
  ResponseItem,
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

const AGENT_STATUS_PREVIEW_MAX_CHARS = 120;

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
    const collabMessage = parseCollabEnvelopeText(item.text, item);
    if (collabMessage) {
      return [
        {
          id: item.id,
          kind: "tool" as const,
          author,
          role: "system" as const,
          text: summarizeCollabAgentMessage(collabMessage),
          timestamp,
          attachments: [],
          toolName: formatCollabAgentMessageTitle(collabMessage),
          toolStatus: "completed",
          toolDetails: formatCollabAgentMessageDetails(collabMessage),
          toolCategory: formatCollabAgentMessageCategory(collabMessage),
        },
      ];
    }

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
        toolCategory: formatCollabAgentMessageCategory(item),
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
        toolCategory: "subagentNotification",
      },
    ];
  }

  if (item.type === "contextCompaction") {
    const replacementHistory = item.replacementHistory;
    const replacementHistoryEntries = Array.isArray(replacementHistory)
      ? buildReplacementHistoryEntries(replacementHistory, {
          author,
          timestamp,
          parentId: item.id,
        })
      : null;
    return [
      {
        id: item.id,
        kind: "compact" as const,
        author,
        role: "system" as const,
        text: "Earlier conversation was replaced with compacted model context.",
        timestamp,
        attachments: [],
        replacementHistoryEntries,
        replacementHistoryStatus: Array.isArray(replacementHistory)
          ? replacementHistory.length > 0
            ? "available"
            : "empty"
          : "missing",
        replacementHistoryCount: Array.isArray(replacementHistory)
          ? replacementHistory.length
          : null,
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
    item.type === "eventDrivenToolCall" ||
    item.type === "eventCommandCall"
  ) {
    const details =
      item.type === "dynamicToolCall"
        ? formatStructuredToolDetails(item.arguments, item.contentItems)
        : item.type === "eventCommandCall"
          ? formatStructuredToolDetails(
              {
                subscriptionId: item.subscriptionId,
                command: item.command,
                cwd: item.cwd,
                label: item.label,
              },
              item.output,
            )
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
        text:
          item.type === "eventCommandCall"
            ? summarizeEventCommandCall(item)
            : summarizeToolCall(item),
        timestamp,
        attachments: [],
        toolName:
          item.type === "eventCommandCall" ? "event_command_subscribe" : item.tool,
        toolStatus: item.status,
        toolDetails: details,
        toolCategory:
          item.type === "eventCommandCall"
            ? "eventDrivenSubscription"
            : item.type === "eventDrivenToolCall"
            ? "eventDrivenSubscription"
            : item.type === "mcpToolCall"
              ? "external"
              : "external",
      },
    ];
  }

  if (item.type === "eventCommandEvent") {
    return [
      {
        id: item.id,
        kind: "tool" as const,
        author,
        role: "system" as const,
        text: summarizeEventCommandEvent(item),
        timestamp,
        attachments: [],
        toolName: eventCommandEventTitle(item),
        toolStatus: "completed",
        toolDetails: formatEventCommandEventDetails(item),
        toolCategory: "eventDrivenEvent",
      },
    ];
  }

  if (item.type === "eventDrivenTool") {
    const collabMessage = parseCollabEnvelopeText(item.text, item);
    if (collabMessage) {
      return [
        {
          id: item.id,
          kind: "tool" as const,
          author,
          role: "system" as const,
          text: summarizeCollabAgentMessage(collabMessage),
          timestamp,
          attachments: [],
          toolName: formatCollabAgentMessageTitle(collabMessage),
          toolStatus: "completed",
          toolDetails: formatCollabAgentMessageDetails(collabMessage),
          toolCategory: formatCollabAgentMessageCategory(collabMessage),
        },
      ];
    }

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
  return items.map((item, index) =>
    ({
      ...buildReplacementHistoryEntry(item, {
        author,
        timestamp,
        id: `${parentId}:replacement:${index}`,
        index,
      }),
      isReplacementHistory: true,
    }),
  );
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
): ConversationEntry {
  switch (item.type) {
    case "message": {
      const role = stringOrFallback(item.role, "message");
      return {
        id,
        kind: "message",
        author: formatReplacementMessageAuthor(role, author),
        role: role === "user" ? "user" : role === "assistant" ? "agent" : "system",
        text:
          extractResponseContentText(item.content) ||
          `Replacement history ${index + 1}: empty ${role} message.`,
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

function parseCollabEnvelopeText(
  text: string,
  item: Pick<ThreadItem, "id" | "startedAtMs" | "completedAtMs">,
): Extract<ThreadItem, { type: "collabAgentMessage" }> | null {
  const envelope = parseRecord(text);
  if (!isChildCompletionEnvelope(envelope)) {
    return null;
  }

  return {
    type: "collabAgentMessage",
    id: item.id,
    operation: "childCompletion",
    senderThreadId: stringOrNull(envelope.sender_thread_id),
    senderPath: stringOrFallback(envelope.author, "unknown"),
    recipientThreadId: stringOrNull(envelope.recipient_thread_id),
    recipientPath: stringOrFallback(envelope.recipient, "unknown"),
    otherRecipientPaths: arrayOfStrings(envelope.other_recipients),
    content: extractCollabEnvelopeContent(envelope),
    triggerTurn: envelope.trigger_turn === true,
    startedAtMs: item.startedAtMs,
    completedAtMs: item.completedAtMs,
  };
}

function isChildCompletionEnvelope(
  envelope: Record<string, unknown> | null,
): envelope is Record<string, unknown> & {
  author: string;
  recipient: string;
  operation: "childCompletion";
  sender_thread_id: string;
  recipient_thread_id: string;
} {
  return (
    envelope?.operation === "childCompletion" &&
    typeof envelope.author === "string" &&
    typeof envelope.recipient === "string" &&
    typeof envelope.sender_thread_id === "string" &&
    typeof envelope.recipient_thread_id === "string"
  );
}

function extractCollabEnvelopeContent(envelope: Record<string, unknown>) {
  const statusMessage = extractCollabEnvelopeStatusMessage(envelope.status);
  if (statusMessage) {
    return statusMessage;
  }

  const taggedContent = extractSubagentNotificationContent(envelope.content);
  return taggedContent ?? stringOrFallback(envelope.content, "…");
}

function extractCollabEnvelopeStatusMessage(status: unknown) {
  if (!status || typeof status !== "object" || Array.isArray(status)) {
    return null;
  }
  const statusRecord = status as Record<string, unknown>;
  return stringOrNull(statusRecord.completed);
}

function extractSubagentNotificationContent(content: unknown) {
  const text = stringOrNull(content);
  if (!text) {
    return null;
  }

  const match = text.match(
    /<subagent_notification>\s*([\s\S]*?)\s*<\/subagent_notification>/u,
  );
  const notification = match ? parseRecord(match[1]) : null;
  if (!notification) {
    return null;
  }

  return extractCollabEnvelopeStatusMessage(notification.status);
}

function parseRecord(text: unknown): Record<string, unknown> | null {
  const json = stringOrNull(text);
  if (!json) {
    return null;
  }

  try {
    const parsed = JSON.parse(json);
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}

function arrayOfStrings(value: unknown) {
  return Array.isArray(value)
    ? value.map((item) => stringOrFallback(item, "unknown"))
    : [];
}

export function buildConversationCells(
  entries: ConversationEntry[],
  previousCells?: ConversationCell[] | null,
): ConversationCell[] {
  const cells: ConversationCell[] = [];
  let segmentEntries: ConversationEntry[] = [];

  for (const entry of entries) {
    if (entry.kind !== "compact") {
      segmentEntries.push(entry);
      continue;
    }

    cells.push(...buildConversationCellsForSegment(segmentEntries));
    segmentEntries = [];

    if (cells.length > 0) {
      const archivedCell = buildArchivedHistoryCell(entry, [...cells]);
      cells.length = 0;
      cells.push(archivedCell);
    }

    cells.push({
      id: entry.id,
      kind: "compact",
      entries: [entry],
    });

    segmentEntries = [...(entry.replacementHistoryEntries ?? [])];
  }

  cells.push(...buildConversationCellsForSegment(segmentEntries));

  return reuseConversationCells(cells, previousCells);
}

function buildConversationCellsForSegment(
  entries: ConversationEntry[],
): ConversationCell[] {
  const cells: ConversationCell[] = [];
  let entryIndex = 0;

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

    cells.push({
      id: nextCellEntries[0].id,
      kind: nextCellEntries[0].kind,
      entries: nextCellEntries,
    });

    entryIndex += nextCellEntries.length;
  }

  return cells;
}

function buildArchivedHistoryCell(
  compactEntry: ConversationEntry,
  archivedCells: ConversationCell[],
): ConversationCell {
  const archivedEntryCount = archivedCells.reduce(
    (count, cell) =>
      count +
      cell.entries.reduce(
        (entryCount, entry) =>
          entryCount +
          (entry.kind === "archive"
            ? (entry.archivedEntryCount ?? 0)
            : 1),
        0,
      ),
    0,
  );
  return {
    id: `${compactEntry.id}:archive`,
    kind: "archive",
    entries: [
      {
        id: `${compactEntry.id}:archive`,
        kind: "archive",
        author: compactEntry.author,
        role: "system",
        text: "Previous conversation is no longer the active model context.",
        timestamp: compactEntry.timestamp,
        attachments: [],
        archivedCells,
        archivedEntryCount,
      },
    ],
  };
}

function reuseConversationCells(
  cells: ConversationCell[],
  previousCells?: ConversationCell[] | null,
): ConversationCell[] {
  if (!previousCells) {
    return cells;
  }

  const previousCellsByKey = new Map(
    previousCells.map((cell) => [conversationCellReuseKey(cell), cell]),
  );

  return cells.map((cell) => {
    const existingCell = previousCellsByKey.get(conversationCellReuseKey(cell));
    if (
      existingCell &&
      existingCell.entries.length === cell.entries.length &&
      existingCell.entries.every(
        (entry, entryIndex) => entry === cell.entries[entryIndex],
      )
    ) {
      return existingCell;
    }
    return cell;
  });
}

function conversationCellReuseKey(cell: ConversationCell) {
  return `${cell.kind}:${cell.id}`;
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
    if (isStandaloneNotificationEntry(previousEntry) || isStandaloneNotificationEntry(nextEntry)) {
      return false;
    }
    if (previousEntry.isReplacementHistory !== nextEntry.isReplacementHistory) {
      return false;
    }
    return previousEntry.toolCategory === nextEntry.toolCategory;
  }

  if (
    cell.kind === "message" &&
    nextEntry.kind === "message" &&
    previousEntry.role === "agent" &&
    nextEntry.role === "agent"
  ) {
    if (previousEntry.isReplacementHistory !== nextEntry.isReplacementHistory) {
      return false;
    }
    return true;
  }

  return false;
}

function isStandaloneNotificationEntry(entry: ConversationEntry) {
  return (
    entry.toolCategory === "childCompletion" ||
    entry.toolCategory === "subagentNotification"
  );
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

function summarizeEventCommandCall(
  item: Extract<ThreadItem, { type: "eventCommandCall" }>,
) {
  return item.label ? `${item.label} • ${item.command}` : item.command;
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

function eventCommandEventTitle(
  item: Extract<ThreadItem, { type: "eventCommandEvent" }>,
) {
  switch (item.kind) {
    case "output":
      return "EventCommand output";
    case "exited":
      return "EventCommand exited";
    case "cancelled":
      return "EventCommand cancelled";
    case "failedToStart":
      return "EventCommand failed";
    default:
      return "EventCommand";
  }
}

function summarizeEventCommandEvent(
  item: Extract<ThreadItem, { type: "eventCommandEvent" }>,
) {
  if (item.kind === "output") {
    return item.line || item.label || item.command;
  }

  if (item.kind === "exited") {
    return item.exitCode === null
      ? `${item.command} exited`
      : `${item.command} exited with code ${item.exitCode}`;
  }

  return item.message || `${item.command} ${item.kind}`;
}

function formatEventCommandEventDetails(
  item: Extract<ThreadItem, { type: "eventCommandEvent" }>,
) {
  const sections = [
    `Subscription\n${item.subscriptionId}`,
    `Command\n${item.command}`,
    `Kind\n${item.kind}`,
  ];

  if (item.cwd) {
    sections.push(`Working directory\n${item.cwd}`);
  }
  if (item.label) {
    sections.push(`Label\n${item.label}`);
  }
  if (item.line) {
    sections.push(`Output\n${item.line}`);
  }
  if (item.message) {
    sections.push(`Message\n${item.message}`);
  }
  if (item.exitCode !== null) {
    sections.push(`Exit code\n${item.exitCode}`);
  }

  return sections.join("\n\n");
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

function stringOrFallback(value: unknown, fallback: string) {
  return stringOrNull(value) ?? fallback;
}
