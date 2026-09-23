import type {
  ConversationArtifactSource,
  ConversationCell,
  ConversationEntry,
  Thread,
  ThreadItem,
  ThreadLifecycleStatus,
} from "../types";
import {
  buildActiveCommandConversationTail,
  type ConversationFlatItemState,
} from "./conversationActiveCommands";
import { selectRunningActiveCommandItems } from "./activeCommands";
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
import { formatScheduleArgument } from "./scheduleDisplay";
import {
  formatMillisecondsDuration,
  formatResponseItemDetails,
  numberOrNull,
  objectOrNull,
  previewInlineText,
  stringOrFallback,
  stringOrNull,
} from "./conversationFormatting";
import {
  attachmentsFromUserInput,
  formatUserInputContent,
} from "./conversationUserInput";
import {
  buildCommandLookup,
  commandExecutionNotificationOutput,
  extractEventDrivenSummaryDetails,
  formatCommandExecutionDetails,
  formatCommandExecutionNotificationDetails,
  formatEventCommandCallDetails,
  formatStructuredToolDetails,
  statusForCommandExecutionNotification,
  summarizeCommandExecution,
  summarizeCommandExecutionNotification,
  summarizeCommandWait,
  summarizeCommandWriteStdin,
  summarizeEventCommandCall,
  summarizeEventCommandEvent,
  toolCategoryForName,
} from "./conversationToolPresentation";

export type ConversationBuildState = {
  threadId: string | null;
  author: string | null;
  flatItems: ConversationFlatItemState[];
  entries: ConversationEntry[];
  cells: ConversationCell[];
};

const AGENT_STATUS_PREVIEW_MAX_CHARS = 120;
const ARTIFACT_SUMMARY_MAX_CHARS = 96;

type CollabAgentStateView = {
  path?: string | null;
  agentNickname?: string | null;
  agentRole?: string | null;
  lifecycleStatus: ThreadLifecycleStatus;
  message?: string | null;
};

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
  const historyItemIds = new Set<string>();
  const activeCommandItemsById = new Map(
    selectRunningActiveCommandItems(thread).map((item) => [item.id, item]),
  );
  let flatItemIndex = 0;
  let flatItemSequence = 0;
  let latestCompactOrderKeyMs: number | null = null;
  const latestCompactItemId = latestContextCompactionItemId(thread);

  for (const turn of thread.turns) {
    const turnTimestampSeconds = turn.completedAt ?? turn.startedAt;
    const fallbackTimestampSeconds = turnTimestampSeconds ?? thread.updatedAt;
    const turnTimestamp = formatClockTime(fallbackTimestampSeconds);
    const turnOrderKeyMs = timestampMsFromSeconds(
      turnTimestampSeconds ?? thread.updatedAt,
    );

    for (const item of turn.items) {
      historyItemIds.add(item.id);
      const displayItem = liveCommandDisplayItem(item, activeCommandItemsById);
      const timestamp = formatItemTimestamp(displayItem) ?? turnTimestamp;
      const orderKeyMs =
        itemOrderKeyMs(displayItem) ?? turnOrderKeyMs ?? flatItemSequence;
      const previousFlatItem = canReusePrevious
        ? previous.flatItems[flatItemIndex]
        : undefined;
      const rebuiltEntries =
        displayItem.type !== "contextCompaction" &&
        previousFlatItem &&
        previousFlatItem.id === displayItem.id &&
        previousFlatItem.item === displayItem &&
        previousFlatItem.timestamp === timestamp
          ? previousFlatItem.entries
          : buildConversationItemEntries(displayItem, {
              author,
              timestamp,
              commandLookup,
              showCompactionDetails:
                displayItem.type !== "contextCompaction" ||
                displayItem.id === latestCompactItemId,
            }).map((entry) => ({
              ...entry,
              turnId: turn.id,
            }));

      flatItems.push({
        id: displayItem.id,
        item: displayItem,
        timestamp,
        orderKeyMs,
        sequence: flatItemSequence,
        entries: rebuiltEntries,
      });
      if (displayItem.type === "contextCompaction") {
        latestCompactOrderKeyMs =
          latestCompactOrderKeyMs === null
            ? orderKeyMs
            : Math.max(latestCompactOrderKeyMs, orderKeyMs);
      }
      flatItemIndex += 1;
      flatItemSequence += 1;
    }
  }

  const activeTail = buildActiveCommandConversationTail({
    thread,
    author,
    timestamp: (item) =>
      formatItemTimestamp(item) ?? formatClockTime(thread.updatedAt),
    orderKeyMs: (item) => {
      const itemOrderKey =
        itemOrderKeyMs(item) ??
        timestampMsFromSeconds(thread.updatedAt) ??
        flatItemSequence;
      return latestCompactOrderKeyMs === null
        ? itemOrderKey
        : Math.max(itemOrderKey, latestCompactOrderKeyMs);
    },
    sequenceStart: flatItemSequence,
    commandLookup,
    historyItemIds,
    includeItem: (item) =>
      !isActiveCommandBeforeCompactBoundary(item, latestCompactOrderKeyMs),
    previous: canReusePrevious ? previous : null,
    buildItemEntries: buildConversationItemEntries,
  });
  flatItems.push(...activeTail.flatItems);
  const orderedFlatItems =
    activeTail.flatItems.length === 0
      ? flatItems
      : mergeActiveTailIntoHistory(flatItems, activeTail.flatItems);
  const entries = orderedFlatItems.flatMap((item) => item.entries);

  return {
    threadId: thread.id,
    author,
    flatItems: orderedFlatItems,
    entries,
    cells: buildConversationCells(
      entries,
      canReusePrevious ? previous?.cells : null,
      options,
    ),
  };
}

function liveCommandDisplayItem(
  item: ThreadItem,
  activeCommandItemsById: ReadonlyMap<string, ThreadItem>,
): ThreadItem {
  if (item.type !== "commandExecution") {
    return item;
  }
  const activeItem = activeCommandItemsById.get(item.id);
  if (!activeItem || activeItem.type !== "commandExecution") {
    return item;
  }
  return {
    ...item,
    ...activeItem,
    startedAtMs: item.startedAtMs ?? activeItem.startedAtMs,
    completedAtMs: activeItem.completedAtMs ?? item.completedAtMs,
  };
}

function isActiveCommandBeforeCompactBoundary(
  item: ThreadItem,
  latestCompactOrderKeyMs: number | null,
) {
  if (latestCompactOrderKeyMs === null) {
    return false;
  }
  const itemOrderKey = itemOrderKeyMs(item);
  return itemOrderKey !== null && itemOrderKey < latestCompactOrderKeyMs;
}

function latestContextCompactionItemId(thread: Thread) {
  let latest:
    | {
        id: string;
        orderKeyMs: number;
        sequence: number;
      }
    | null = null;
  let sequence = 0;

  for (const turn of thread.turns) {
    const turnTimestampSeconds = turn.completedAt ?? turn.startedAt;
    const turnOrderKeyMs = timestampMsFromSeconds(
      turnTimestampSeconds ?? thread.updatedAt,
    );

    for (const item of turn.items) {
      if (item.type === "contextCompaction") {
        const orderKeyMs = itemOrderKeyMs(item) ?? turnOrderKeyMs ?? sequence;
        if (
          latest === null ||
          orderKeyMs > latest.orderKeyMs ||
          (orderKeyMs === latest.orderKeyMs && sequence > latest.sequence)
        ) {
          latest = { id: item.id, orderKeyMs, sequence };
        }
      }
      sequence += 1;
    }
  }

  return latest?.id ?? null;
}

function compareConversationFlatItems(
  left: ConversationFlatItemState,
  right: ConversationFlatItemState,
) {
  const leftOrder = left.orderKeyMs ?? Number.POSITIVE_INFINITY;
  const rightOrder = right.orderKeyMs ?? Number.POSITIVE_INFINITY;
  if (leftOrder !== rightOrder) {
    return leftOrder - rightOrder;
  }
  return (left.sequence ?? 0) - (right.sequence ?? 0);
}

function mergeActiveTailIntoHistory(
  flatItems: ConversationFlatItemState[],
  activeTailItems: ConversationFlatItemState[],
) {
  if (activeTailItems.length === 0) {
    return flatItems;
  }
  const activeTailIds = new Set(activeTailItems.map((item) => item.id));
  const historyItems = flatItems.filter((item) => !activeTailIds.has(item.id));
  const sortedActiveTailItems = [...activeTailItems].sort(
    compareConversationFlatItems,
  );
  const orderedItems = [...historyItems];

  for (const activeItem of sortedActiveTailItems) {
    const insertIndex = orderedItems.findIndex((historyItem) => {
      const activeOrder = activeItem.orderKeyMs ?? Number.POSITIVE_INFINITY;
      const historyOrder = historyItem.orderKeyMs ?? Number.POSITIVE_INFINITY;
      if (historyOrder !== activeOrder) {
        return historyOrder > activeOrder;
      }
      return (historyItem.sequence ?? 0) > (activeItem.sequence ?? 0);
    });
    if (insertIndex === -1) {
      orderedItems.push(activeItem);
    } else {
      orderedItems.splice(insertIndex, 0, activeItem);
    }
  }

  return orderedItems;
}

function buildConversationItemEntries(
  item: ThreadItem,
  {
    author,
    timestamp,
    commandLookup,
    showCompactionDetails = true,
  }: {
    author: string;
    timestamp: string;
    commandLookup: Map<string, string>;
    showCompactionDetails?: boolean;
  },
): ConversationEntry[] {
  if (item.type === "userMessage") {
    const attachments = attachmentsFromUserInput(item.content);
    return [
      {
        id: item.id,
        kind: "message" as const,
        author: "You",
        role: "user" as const,
        text: formatUserInputContent(item.content),
        timestamp,
        attachments,
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
    return buildInjectedContextEntries(item, { author, timestamp });
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
    const commandLabel =
      commandLookup.get(item.commandItemId) ?? item.commandItemId;
    return [
      {
        id: item.id,
        kind: "tool" as const,
        author,
        role: "system" as const,
        text: summarizeCommandExecutionNotification(item, commandLookup),
        timestamp,
        attachments: [],
        toolName: commandLabel,
        toolStatus: statusForCommandExecutionNotification(item),
        toolDetails: formatCommandExecutionNotificationDetails(
          item,
          commandLabel,
        ),
        toolOutput: commandExecutionNotificationOutput(item),
        toolCategory: "command",
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
    const pollEventProgress = buildPollEventProgress(item);
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
        pollEventProgress,
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
        interAgent: buildCollabAgentToolPresentation(item),
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
    return [buildCollabAgentStatusUpdateEntry(item, { author, timestamp })];
  }

  if (item.type === "conversationArtifact") {
    return [
      {
        id: item.id,
        kind: "artifact" as const,
        author,
        role: "agent" as const,
        text: summarizeConversationArtifact(item),
        timestamp,
        attachments: [],
        artifact: {
          title: item.title || "Artifact",
          source: conversationArtifactSource(item),
          mimeType: item.mimeType,
          content: item.content,
          language: item.language,
          truncated: item.truncated,
        },
      },
    ];
  }

  if (item.type === "contextCompaction") {
    return [buildContextCompactionEntry(item, { author, timestamp })];
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
    return [];
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

  if (item.type === "clientRecovery") {
    return [
      {
        id: item.id,
        kind: "event" as const,
        author,
        role: "system" as const,
        text: [
          `Runtime Capsule recovery: ${item.reason}`,
          `Release: ${item.releaseId}`,
          item.fallbackReleaseId
            ? `Fallback release: ${item.fallbackReleaseId}`
            : null,
          `Activation: ${item.activationId}`,
        ]
          .filter((line): line is string => line !== null)
          .join("\n"),
        timestamp,
        attachments: [],
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

function summarizeConversationArtifact(
  item: Extract<ThreadItem, { type: "conversationArtifact" }>,
) {
  const title = item.title.trim() || "Artifact";
  const source = conversationArtifactSource(item);
  const mimeType = artifactSourceMimeType(source, item.mimeType);
  const summary =
    source.type === "url"
      ? `${title} • ${mimeType} • ${source.url}`
      : `${title} • ${mimeType}`;
  return previewInlineText(summary, ARTIFACT_SUMMARY_MAX_CHARS);
}

function conversationArtifactSource(
  item: Extract<ThreadItem, { type: "conversationArtifact" }>,
): ConversationArtifactSource {
  if (item.source?.type === "url") {
    return {
      type: "url",
      url: item.source.url,
      mimeType: item.source.mimeType ?? item.mimeType,
      fallbackContent: item.source.fallbackContent ?? item.content,
    };
  }
  if (item.source?.type === "inline") {
    return {
      type: "inline",
      content: item.source.content,
      mimeType: item.source.mimeType,
      language: item.source.language,
      truncated: item.source.truncated,
    };
  }
  return {
    type: "inline",
    content: item.content,
    mimeType: item.mimeType,
    language: item.language,
    truncated: item.truncated,
  };
}

function artifactSourceMimeType(
  source: ConversationArtifactSource,
  fallbackMimeType: string,
) {
  return (source.mimeType ?? fallbackMimeType).trim() || "unknown";
}

function buildContextCompactionEntry(
  item: Extract<ThreadItem, { type: "contextCompaction" }>,
  {
    author,
    timestamp,
  }: {
    author: string;
    timestamp: string;
  },
): ConversationEntry {
  return {
    id: item.id,
    kind: "compact",
    author,
    role: "system",
    text: "Context compacted",
    timestamp,
    attachments: [],
    compactSummary: null,
  };
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

  if (item.type === "clientRecovery") {
    const occurredAtMs = Date.parse(item.occurredAt);
    if (Number.isFinite(occurredAtMs)) {
      return formatClockTime(occurredAtMs / 1000);
    }
  }

  const timestampMs = item.completedAtMs ?? item.startedAtMs;
  return timestampMs === null || timestampMs === undefined
    ? null
    : formatClockTime(timestampMs / 1000);
}

function itemOrderKeyMs(item: ThreadItem) {
  if (
    (item.type === "commandExecutionNotification" ||
      item.type === "commandWait" ||
      item.type === "commandWriteStdin") &&
    Number.isFinite(item.createdAtMs)
  ) {
    return item.createdAtMs;
  }

  if (
    item.type === "workflowRunProgress" &&
    Number.isFinite(item.event.updatedAt)
  ) {
    return item.event.updatedAt * 1000;
  }

  if (item.type === "eventCommandEvent" && Number.isFinite(item.createdAt)) {
    return item.createdAt * 1000;
  }

  if (item.type === "clientRecovery") {
    const occurredAtMs = Date.parse(item.occurredAt);
    if (Number.isFinite(occurredAtMs)) {
      return occurredAtMs;
    }
  }

  const timestampMs = item.completedAtMs ?? item.startedAtMs;
  return Number.isFinite(timestampMs) ? timestampMs! : null;
}

function timestampMsFromSeconds(timestampSeconds?: number | null) {
  return Number.isFinite(timestampSeconds) ? timestampSeconds! * 1000 : null;
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
    interAgent: buildCollabAgentMessagePresentation(item),
  };
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
  if (item.tool === "read_agent") {
    return summarizeReadAgentToolCall(item);
  }
  if (item.tool === "poll_event") {
    const output = objectOrNull(item.output);
    const error = stringOrNull(output?.error);
    const sourceHint = stringOrNull(output?.sourceHint);
    const sourceCategory = stringOrNull(output?.sourceCategory);
    if (item.status === "failed" || error) {
      return error ? `poll_event • failed: ${error}` : "poll_event • failed";
    }
    const currentTimeoutMs = numberOrNull(output?.currentTimeoutMs);
    if (isToolStatusInProgress(item.status) && currentTimeoutMs !== null) {
      return `poll_event • waiting up to ${formatMillisecondsDuration(currentTimeoutMs)}`;
    }
    if (output?.timedOut === true) {
      return "poll_event • timeout";
    }
    if (sourceCategory) {
      return `poll_event • ${sourceCategory}`;
    }
    if (sourceHint) {
      return `poll_event • ${sourceHint}`;
    }
    return "poll_event • woke";
  }
  if (item.tool === "list_commands") {
    return summarizeListCommandsToolCall(item);
  }
  if (item.tool === "list_subscriptions") {
    return summarizeListSubscriptionsToolCall(item);
  }
  const details = extractEventDrivenSummaryDetails(item.tool, item.arguments);
  return details ? `${item.tool} • ${details}` : item.tool;
}

function summarizeListCommandsToolCall(
  item: Extract<ThreadItem, { type: "builtinToolCall" }>,
) {
  if (isToolStatusInProgress(item.status)) {
    return "list_commands • checking";
  }
  const output = objectOrNull(item.output);
  const commands = Array.isArray(output?.commands) ? output.commands : [];
  if (commands.length === 0) {
    return "list_commands • no live commands";
  }
  const firstCommand = objectOrNull(commands[0]);
  const label =
    stringOrNull(firstCommand?.label) ??
    stringOrNull(firstCommand?.command_text) ??
    stringOrNull(firstCommand?.call_id);
  const countLabel = `${commands.length} live command${commands.length === 1 ? "" : "s"}`;
  return label
    ? `list_commands • ${countLabel} • ${previewInlineText(label, 120)}`
    : `list_commands • ${countLabel}`;
}

function summarizeListSubscriptionsToolCall(
  item: Extract<ThreadItem, { type: "builtinToolCall" }>,
) {
  if (isToolStatusInProgress(item.status)) {
    return "list_subscriptions • checking";
  }
  const output = objectOrNull(item.output);
  const subscriptions = Array.isArray(output?.subscriptions)
    ? output.subscriptions
    : [];
  if (subscriptions.length === 0) {
    return "list_subscriptions • no active subscriptions";
  }
  const firstSubscription = objectOrNull(subscriptions[0]);
  const label =
    stringOrNull(firstSubscription?.label) ??
    formatSubscriptionSnapshot(firstSubscription) ??
    stringOrNull(firstSubscription?.subscription_id);
  const countLabel = `${subscriptions.length} active subscription${subscriptions.length === 1 ? "" : "s"}`;
  return label
    ? `list_subscriptions • ${countLabel} • ${previewInlineText(label, 120)}`
    : `list_subscriptions • ${countLabel}`;
}

function formatSubscriptionSnapshot(
  subscription: Record<string, unknown> | null,
) {
  if (!subscription) {
    return null;
  }
  const type = stringOrNull(subscription.type);
  if (type === "schedule") {
    return formatScheduleArgument(subscription.schedule) ?? "schedule";
  }
  if (type === "event_command") {
    return stringOrNull(subscription.command_text) ?? "event command";
  }
  if (type === "fs") {
    return stringOrNull(subscription.path) ?? "file watch";
  }
  if (type === "process_exit") {
    const sessionId = numberOrNull(subscription.session_id);
    return sessionId === null ? "process exit" : `process ${sessionId}`;
  }
  return type;
}

function summarizeReadAgentToolCall(
  item: Extract<ThreadItem, { type: "builtinToolCall" }>,
) {
  const argumentsRecord = objectOrNull(item.arguments);
  const output = objectOrNull(item.output);
  const target =
    stringOrNull(output?.target) ?? stringOrNull(argumentsRecord?.target);
  const agentName = stringOrNull(output?.agentName);
  const agentLabel = agentName && agentName !== target ? agentName : null;
  const error = stringOrNull(output?.error);
  if (item.status === "failed" || error) {
    return [
      "read_agent",
      target,
      error ? `failed: ${previewInlineText(error, 120)}` : "failed",
    ]
      .filter((value): value is string => Boolean(value))
      .join(" • ");
  }
  if (isToolStatusInProgress(item.status)) {
    return ["read_agent", target ?? "reading"].join(" • ");
  }

  const lifecycleStatus = lifecycleStatusFromUnknown(output?.lifecycleStatus);
  const status = lifecycleStatus
    ? formatLifecycleStatus(lifecycleStatus)
    : null;
  const message =
    stringOrNull(output?.lastAgentMessage) ??
    stringOrNull(output?.lastTaskMessage);
  const preview = message ? previewInlineText(message, 120) : null;
  return ["read_agent", target, agentLabel, status, preview]
    .filter((value): value is string => Boolean(value))
    .join(" • ");
}

function buildPollEventProgress(
  item: Extract<ThreadItem, { type: "builtinToolCall" }>,
) {
  if (item.tool !== "poll_event" || !isToolStatusInProgress(item.status)) {
    return undefined;
  }
  const currentTimeoutMs = numberOrNull(
    objectOrNull(item.output)?.currentTimeoutMs,
  );
  const startedAtMs = numberOrNull(item.startedAtMs);
  if (currentTimeoutMs === null || startedAtMs === null) {
    return undefined;
  }
  return {
    startedAtMs,
    currentTimeoutMs,
  };
}

function summarizeCollabAgentToolCall(
  item: Extract<ThreadItem, { type: "collabAgentToolCall" }>,
) {
  const stateByPath = collabAgentStatesByPath(item);
  const receiverLabel =
    item.receiverPaths.length === 1
      ? formatCollabAgentLabel(
          item.receiverPaths[0],
          stateByPath.get(item.receiverPaths[0]),
        )
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
      if (
        item.receiverPaths.length > 0 &&
        item.timeoutMs !== null &&
        item.timeoutMs !== undefined
      ) {
        return `wait on ${receiverLabel} for ${formatWaitTimeout(item.timeoutMs)}`;
      }
      if (item.receiverPaths.length > 0) {
        return `wait on ${receiverLabel}`;
      }
      return "wait_agent";
    case "listAgents":
    case "list_agents":
      if (item.receiverPaths.length === 0) {
        return "list_agents";
      }
      return `listed ${item.receiverPaths.length} agents${formatProviderSummary(
        item.agentsStates,
      )}`;
    case "closeAgent":
      return `Closed ${receiverLabel}.`;
    default:
      return `${item.tool} for ${receiverLabel}.`;
  }
}

function buildCollabAgentToolPresentation(
  item: Extract<ThreadItem, { type: "collabAgentToolCall" }>,
): ConversationEntry["interAgent"] {
  const stateByPath = collabAgentStatesByPath(item);
  const targetPaths = item.receiverPaths.map((path) =>
    stringOrFallback(path, "unknown"),
  );
  const primaryPath = targetPaths[0] ?? "unknown";
  const primaryState = stateByPath.get(primaryPath);
  const providerLabel = collabExternalProviderLabel(primaryState);
  const status = isToolStatusInProgress(item.status) ? "sending" : item.status;

  switch (item.tool) {
    case "spawnAgent":
      return {
        kind: "spawn",
        direction: "outgoing",
        senderPath: stringOrNull(item.senderPath),
        targetPaths,
        primaryPath,
        title: `Created ${formatAgentMention(primaryPath)}`,
        body: stringOrNull(item.prompt),
        status,
        chips: [
          providerLabel,
          item.model,
          item.reasoningEffort,
          targetPaths.length > 1 ? `${targetPaths.length} agents` : null,
        ].filter((value): value is string => Boolean(value)),
      };
    case "sendInput":
    case "resumeAgent":
      return {
        kind: "followup",
        direction: "outgoing",
        senderPath: stringOrNull(item.senderPath),
        targetPaths,
        primaryPath,
        title: `You -> ${formatAgentMention(primaryPath)}`,
        body: stringOrNull(item.prompt),
        status,
        chips: [
          providerLabel,
          targetPaths.length > 1 ? `${targetPaths.length} recipients` : null,
        ].filter((value): value is string => Boolean(value)),
      };
    default:
      return undefined;
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
    sections.push(
      `Previous Status\n${formatThreadGoalStatus(item.previousStatus)}`,
    );
  }

  if (item.goal.tokenBudget !== null && item.goal.tokenBudget !== undefined) {
    sections.push(
      `Token Usage\n${formatThreadGoalNumber(item.goal.tokensUsed)} / ${formatThreadGoalNumber(item.goal.tokenBudget)}`,
    );
  } else if (item.goal.tokensUsed > 0) {
    sections.push(
      `Token Usage\n${formatThreadGoalNumber(item.goal.tokensUsed)}`,
    );
  }

  if (item.goal.timeUsedSeconds > 0) {
    sections.push(
      `Time Used\n${formatThreadGoalDuration(item.goal.timeUsedSeconds)}`,
    );
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

  return omitted ? `${preview}\n… omitted additional captured output` : preview;
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
  const sections: Array<[string, unknown]> = [
    ["Tool", formatCollabAgentToolName(item.tool)],
    ["Status", item.status],
  ];

  switch (item.tool) {
    case "spawnAgent":
      sections.push(
        ["message", item.prompt?.trim()],
        ["model", item.model],
        ["reasoning_effort", item.reasoningEffort],
        ["Result", formatCollabAgentToolResult(item)],
      );
      break;
    case "sendInput":
      sections.push(
        [
          formatCollabAgentTargetLabel(item.receiverPaths),
          formatCollabAgentTargets(item),
        ],
        ["content", item.prompt?.trim()],
      );
      break;
    case "resumeAgent":
      sections.push([
        formatCollabAgentTargetLabel(item.receiverPaths),
        formatCollabAgentTargets(item),
      ]);
      break;
    case "wait":
      sections.push(
        [
          formatCollabAgentTargetLabel(item.receiverPaths),
          formatCollabAgentTargets(item),
        ],
        ["timeout_ms", item.timeoutMs],
        ["Result", formatCollabAgentToolResult(item)],
      );
      break;
    case "listAgents":
    case "list_agents":
      sections.push(
        ["path_prefix", item.prompt?.trim()],
        ["Result", formatCollabAgentToolResult(item)],
      );
      break;
    case "closeAgent":
      sections.push(
        [
          formatCollabAgentTargetLabel(item.receiverPaths),
          formatCollabAgentTargets(item),
        ],
        ["Result", formatCollabAgentToolResult(item)],
      );
      break;
    default:
      sections.push(
        [
          formatCollabAgentTargetLabel(item.receiverPaths),
          formatCollabAgentTargets(item),
        ],
        ["content", item.prompt?.trim()],
        ["timeout_ms", item.timeoutMs],
        ["model", item.model],
        ["reasoning_effort", item.reasoningEffort],
        ["Result", formatCollabAgentToolResult(item)],
      );
  }

  return formatResponseItemDetails(sections);
}

function formatCollabAgentToolResult(
  item: Extract<ThreadItem, { type: "collabAgentToolCall" }>,
) {
  const agentStates = Object.entries(item.agentsStates ?? {});
  if (!shouldShowCollabAgentResult(item) || agentStates.length === 0) {
    return null;
  }

  return agentStates
    .map(([threadId, state]) =>
      [
        formatCollabAgentLabel(
          stringOrNull(state.path) ?? trimThreadId(threadId),
          state,
        ),
        formatLifecycleStatus(state.lifecycleStatus),
        stringOrNull(state.message),
      ]
        .filter((value) => value && value.length > 0)
        .join(" • "),
    )
    .join("\n");
}

function shouldShowCollabAgentResult(
  item: Extract<ThreadItem, { type: "collabAgentToolCall" }>,
) {
  switch (item.tool) {
    case "spawnAgent":
    case "listAgents":
    case "list_agents":
    case "wait":
    case "closeAgent":
      return true;
    case "sendInput":
    case "resumeAgent":
      return false;
    default:
      return false;
  }
}

function formatCollabAgentTargets(
  item: Extract<ThreadItem, { type: "collabAgentToolCall" }>,
) {
  if (item.receiverPaths.length === 0) {
    return null;
  }
  return item.receiverPaths
    .map((path) => stringOrFallback(path, "unknown"))
    .join("\n");
}

function formatCollabAgentTargetLabel(receiverPaths: string[]) {
  return receiverPaths.length > 1 ? "targets" : "target";
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

function buildCollabAgentMessagePresentation(
  item: Extract<ThreadItem, { type: "collabAgentMessage" }>,
): ConversationEntry["interAgent"] {
  const senderPath = stringOrFallback(item.senderPath, "unknown");
  const recipientPath = stringOrFallback(item.recipientPath, "unknown");
  const otherRecipients = item.otherRecipientPaths.map((path) =>
    stringOrFallback(path, "unknown"),
  );
  const content = stringOrNull(item.content);

  if (item.operation === "childCompletion") {
    return {
      kind: "completion",
      direction: "event",
      senderPath,
      targetPaths: [recipientPath, ...otherRecipients],
      primaryPath: senderPath,
      title: `${formatAgentMention(senderPath)} completed`,
      body: content,
      status: "completed",
      chips: item.triggerTurn ? ["triggered turn"] : [],
    };
  }

  const operation = formatCollabAgentMessageOperation(item.operation);
  return {
    kind: operation === "spawnAgent" ? "spawn" : "incoming",
    direction: "incoming",
    senderPath,
    targetPaths: [recipientPath, ...otherRecipients],
    primaryPath: senderPath,
    title:
      operation === "spawnAgent"
        ? `${formatAgentMention(senderPath)} invited this agent`
        : `${formatAgentMention(senderPath)} -> You`,
    body: content,
    status: "received",
    chips: item.triggerTurn ? ["triggered turn"] : [],
  };
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
  return item.operation === "childCompletion"
    ? "childCompletion"
    : "multiAgent";
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

function buildCollabAgentStatusUpdateEntry(
  item: Extract<ThreadItem, { type: "collabAgentStatusUpdate" }>,
  {
    author,
    timestamp,
  }: {
    author: string;
    timestamp: string;
  },
): ConversationEntry {
  return {
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
    interAgent: buildCollabAgentStatusPresentation(item),
  };
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
    stringOrNull(item.lifecycleStatus.path) ??
    stringOrNull(item.senderPath) ??
    "unknown";
  const agentLabel = formatCollabAgentLabel(agentPath, item.lifecycleStatus);
  const message = previewInlineText(
    item.lifecycleStatus.message,
    AGENT_STATUS_PREVIEW_MAX_CHARS,
  );
  return [
    agentLabel,
    formatLifecycleStatus(item.lifecycleStatus.lifecycleStatus),
    message,
  ]
    .filter((value) => value && value.length > 0)
    .join(" • ");
}

function buildCollabAgentStatusPresentation(
  item: Extract<ThreadItem, { type: "collabAgentStatusUpdate" }>,
): ConversationEntry["interAgent"] {
  const agentPath = stringOrFallback(
    item.lifecycleStatus.path ?? item.senderPath,
    "unknown",
  );
  const recipientPath = stringOrFallback(item.recipientPath, "unknown");
  const status = formatLifecycleStatus(item.lifecycleStatus.lifecycleStatus);
  const providerLabel = collabExternalProviderLabel(item.lifecycleStatus);
  const isFinal = item.lifecycleStatus.lifecycleStatus.type === "final";
  return {
    kind: isFinal ? "completion" : "status",
    direction: "event",
    senderPath: stringOrNull(item.senderPath),
    targetPaths: [recipientPath],
    primaryPath: agentPath,
    title: isFinal
      ? `${formatAgentMention(agentPath)} completed`
      : `${formatAgentMention(agentPath)} status`,
    body: stringOrNull(item.lifecycleStatus.message),
    status,
    chips: [providerLabel].filter((value): value is string => Boolean(value)),
  };
}

function formatCollabAgentStatusUpdateTitle(
  item: Extract<ThreadItem, { type: "collabAgentStatusUpdate" }>,
) {
  const agentPath = resolveAgentPath(
    item.lifecycleStatus.path,
    item.senderPath,
  );
  const agentLabel = formatCollabAgentLabel(agentPath, item.lifecycleStatus);
  return item.lifecycleStatus.lifecycleStatus.type === "final"
    ? `${agentLabel} subagent completion`
    : `status from ${agentLabel}`;
}

function formatCollabAgentStatusUpdateDetails(
  item: Extract<ThreadItem, { type: "collabAgentStatusUpdate" }>,
) {
  const sections = [
    `From\n${stringOrFallback(item.senderPath, "unknown")}`,
    `To\n${stringOrFallback(item.recipientPath, "unknown")}`,
    `Status\n${formatLifecycleStatus(item.lifecycleStatus.lifecycleStatus)}`,
  ];

  const statusPath = stringOrNull(item.lifecycleStatus.path);
  if (statusPath) {
    sections.push(
      `Agent\n${formatCollabAgentLabel(statusPath, item.lifecycleStatus)}`,
    );
  }

  const providerLabel = collabExternalProviderLabel(item.lifecycleStatus);
  if (providerLabel) {
    sections.push(`Provider\n${providerLabel}`);
  }

  const statusMessage = stringOrNull(item.lifecycleStatus.message);
  if (statusMessage) {
    sections.push(`Message\n${statusMessage}`);
  }

  return sections.join("\n\n");
}

function collabAgentStatesByPath(
  item: Extract<ThreadItem, { type: "collabAgentToolCall" }>,
) {
  const states = new Map<string, CollabAgentStateView>();
  for (const state of Object.values(item.agentsStates ?? {})) {
    const path = stringOrNull(state.path);
    if (path) {
      states.set(path, state);
    }
  }
  return states;
}

function formatCollabAgentLabel(
  path: string,
  state?: CollabAgentStateView | null,
) {
  const providerLabel = collabExternalProviderLabel(state);
  return providerLabel ? `${providerLabel} ${path}` : path;
}

function formatProviderSummary(
  states: Extract<ThreadItem, { type: "collabAgentToolCall" }>["agentsStates"],
) {
  const providers = [
    ...new Set(
      Object.values(states ?? {})
        .map((state) => collabExternalProviderLabel(state))
        .filter((value): value is string => Boolean(value)),
    ),
  ];
  return providers.length > 0 ? ` (${providers.join(", ")})` : "";
}

function collabExternalProviderLabel(state?: CollabAgentStateView | null) {
  const providerId = [state?.agentRole, state?.agentNickname]
    .map((value) => value?.trim())
    .find((value) => value && externalProviderLabels[value]);
  return providerId ? externalProviderLabels[providerId] : null;
}

const externalProviderLabels: Record<string, string> = {
  codex_cli: "Codex CLI",
  claude_cli: "Claude Code",
  opencode: "OpenCode",
};

function formatAgentMention(path: string) {
  const value = path.trim() || "unknown";
  return value.startsWith("@") ? value : `@${value}`;
}

function formatLifecycleStatus(status: ThreadLifecycleStatus) {
  switch (status.type) {
    case "notLoaded":
      return "notLoaded";
    case "initializing":
      return "initializing";
    case "active":
      return "active";
    case "waiting":
      return `waiting:${status.reason}`;
    case "final":
      return status.result.type;
    case "systemError":
      return "systemError";
  }
}

function lifecycleStatusFromUnknown(
  value: unknown,
): ThreadLifecycleStatus | null {
  const record = objectOrNull(value);
  const type = stringOrNull(record?.type);
  switch (type) {
    case "notLoaded":
    case "initializing":
      return { type };
    case "active":
      return Array.isArray(record?.activeFlags)
        ? { type, activeFlags: record.activeFlags as never[] }
        : null;
    case "waiting": {
      const reason = stringOrNull(record?.reason);
      return reason ? ({ type, reason } as ThreadLifecycleStatus) : null;
    }
    case "final": {
      const result = objectOrNull(record?.result);
      const resultType = stringOrNull(result?.type);
      return resultType
        ? ({
            type,
            result: { ...result, type: resultType },
          } as ThreadLifecycleStatus)
        : null;
    }
    case "systemError":
      return { type, message: stringOrNull(record?.message) ?? undefined };
    default:
      return null;
  }
}

function buildInjectedContextEntries(
  item: Extract<ThreadItem, { type: "injectedContext" }>,
  {
    author,
    timestamp,
  }: {
    author: string;
    timestamp: string;
  },
): ConversationEntry[] {
  if (item.sections.length === 0) {
    return [
      {
        id: item.id,
        kind: "tool",
        author,
        role: "system",
        text: item.preview || "Initial context was injected.",
        timestamp,
        attachments: [],
        toolName: item.title,
        toolStatus: "completed",
        toolDetails: item.preview || item.title || "Initial context was injected.",
        toolCategory: "context",
      },
    ];
  }
  return item.sections.map((section, index) => {
    const label = section.label || `Section ${index + 1}`;
    const text = stringOrFallback(section.text, "");
    return {
      id: `${item.id}:section:${index}`,
      kind: "tool",
      author,
      role: "system",
      text: previewInlineText(text, 160) ?? "Initial context section is empty.",
      timestamp,
      attachments: [],
      toolName: `${item.title} · ${label}`,
      toolStatus: "completed",
      toolDetails: `${label}\n${text}`,
      toolCategory: "context",
    };
  });
}

function basename(filePath: string) {
  const normalized = filePath.replace(/\\/g, "/").replace(/\/+$/u, "");
  const lastSlash = normalized.lastIndexOf("/");
  return lastSlash === -1
    ? normalized
    : normalized.slice(lastSlash + 1) || normalized;
}

function resolveAgentPath(...paths: Array<unknown>) {
  return (
    paths.map(stringOrNull).find((path) => path && path.length > 0) ?? "unknown"
  );
}

function isToolStatusInProgress(status: unknown) {
  const normalized = stringOrNull(status)
    ?.toLowerCase()
    .replace(/[_\s-]+/gu, "");
  return (
    normalized === "inprogress" ||
    normalized === "running" ||
    normalized === "pending" ||
    normalized === "started"
  );
}
