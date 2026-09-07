import type {
  CompactReplacementHistoryItem,
  ConversationArtifactSource,
  ConversationEntry,
  ResponseItem,
} from "../types";
import {
  attachmentsFromUserInput,
  formatUserInputContent,
} from "./conversationUserInput";
import {
  formatMillisecondsDuration,
  formatRawJson,
  formatResponseItemDetails,
  formatSecondsDuration,
  parseMaybeJsonString,
  previewInlineText,
  stringOrFallback,
  stringOrNull,
  stringOrNumberId,
} from "./conversationFormatting";

const STRUCTURED_TOOL_OUTPUT_NAMES = new Set([
  "command_wait",
  "command_write_stdin",
  "wait_agent",
]);
const ARTIFACT_SUMMARY_MAX_CHARS = 96;

export function buildReplacementHistoryEntries(
  items: Array<CompactReplacementHistoryItem | ResponseItem>,
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
  const rawResponseItems = items.filter(isResponseItem);
  const hiddenFunctionOutputCallIds =
    collectStructuredToolOutputCallIds(rawResponseItems);
  const hiddenFunctionCallIds =
    collectStructuredToolCallIdsWithTypedDisplay(rawResponseItems);
  return items.flatMap((item, index) => {
    if (!isResponseItem(item)) {
      const entry = buildTypedReplacementHistoryEntry(item, {
        author,
        timestamp,
        id: `${parentId}:replacement:${index}`,
      });
      return [
        {
          ...entry,
          isReplacementHistory: true,
        },
      ];
    }
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

function isResponseItem(
  item: CompactReplacementHistoryItem | ResponseItem,
): item is ResponseItem {
  return ![
    "injectedContext",
    "userMessage",
    "agentMessage",
    "conversationArtifact",
  ].includes(String(item.type));
}

function buildTypedReplacementHistoryEntry(
  item: CompactReplacementHistoryItem,
  {
    author,
    timestamp,
    id,
  }: {
    author: string;
    timestamp: string;
    id: string;
  },
): ConversationEntry {
  switch (item.type) {
    case "injectedContext":
      return replacementContextEntry({
        id,
        author,
        timestamp,
        text: item.preview || item.title || "Initial context was injected.",
        details: item.sections
          .map((section) => `${section.label}\n${section.text}`)
          .join("\n\n"),
      });
    case "userMessage":
      return {
        id,
        kind: "message",
        author: "You",
        role: "user",
        text: formatUserInputContent(item.content),
        timestamp,
        attachments: attachmentsFromUserInput(item.content),
      };
    case "agentMessage":
      return {
        id,
        kind: "message",
        author,
        role: "agent",
        text: item.text || "Replacement history assistant message.",
        timestamp,
        attachments: [],
      };
    case "conversationArtifact":
      return {
        id,
        kind: "artifact",
        author,
        role: "agent",
        text: summarizeConversationArtifactReplacement(item),
        timestamp,
        attachments: [],
        artifact: {
          title: item.title || "Artifact",
          source: conversationArtifactReplacementSource(item),
          mimeType: item.mimeType,
          content: item.content,
          language: item.language,
          truncated: item.truncated,
        },
      };
  }
}

function summarizeConversationArtifactReplacement(
  item: Extract<CompactReplacementHistoryItem, { type: "conversationArtifact" }>,
) {
  const title = item.title.trim() || "Artifact";
  const source = conversationArtifactReplacementSource(item);
  const mimeType = artifactSourceMimeType(source, item.mimeType);
  return previewInlineText(
    source.type === "url" ? `${title} • ${mimeType} • ${source.url}` : `${title} • ${mimeType}`,
    ARTIFACT_SUMMARY_MAX_CHARS,
  );
}

function conversationArtifactReplacementSource(
  item: Extract<CompactReplacementHistoryItem, { type: "conversationArtifact" }>,
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
      if (role === "developer") {
        return replacementContextEntry({
          id,
          author,
          timestamp,
          text: previewInlineText(text, 160) ?? "Initial context was injected.",
          details: text,
        });
      }
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

function replacementContextEntry({
  id,
  author,
  timestamp,
  text,
  details,
}: {
  id: string;
  author: string;
  timestamp: string;
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
    toolName: "Init Context",
    toolStatus: "completed",
    toolDetails: details,
    toolCategory: "context",
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
