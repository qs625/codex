import type { ConversationCell, ConversationEntry, Thread, ThreadItem } from "../types";
import { formatClockTime, getThreadLabel, trimPath } from "./thread";

export function buildConversationEntries(thread: Thread | null): ConversationEntry[] {
  if (!thread) {
    return [];
  }

  const author = getThreadLabel(thread);

  return thread.turns.flatMap((turn) =>
    turn.items.flatMap((item) => {
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
          },
        ];
      }

      if (item.type === "collabAgentToolCall") {
        return [
          {
            id: item.id,
            kind: "event" as const,
            author,
            role: "system" as const,
            text:
              item.prompt?.trim() ||
              `${author} delegated work to ${item.receiverThreadIds.length} worker${item.receiverThreadIds.length === 1 ? "" : "s"}`,
            timestamp,
            attachments: [],
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

      if (item.type === "dynamicToolCall" || item.type === "mcpToolCall") {
        const details =
          item.type === "dynamicToolCall"
            ? formatStructuredToolDetails(item.arguments, item.contentItems)
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
    return true;
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
  item: Extract<ThreadItem, { type: "dynamicToolCall" | "mcpToolCall" }>,
) {
  if (item.type === "mcpToolCall") {
    return `${item.server}/${item.tool}`;
  }
  if (item.namespace) {
    return `${item.namespace}/${item.tool}`;
  }
  return item.tool;
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
