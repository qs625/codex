import type { Thread, ThreadItem } from "../types";
import { formatScheduleArgument } from "./scheduleDisplay";
import { trimPath } from "./thread";
import {
  formatMillisecondsDuration,
  formatSecondsDuration,
  safeJson,
  stringOrNull,
} from "./conversationFormatting";

export function summarizeCommandExecution(
  item: Extract<ThreadItem, { type: "commandExecution" }>,
) {
  const cwd = trimPath(item.cwd);
  const exitCode =
    item.exitCode === null || item.exitCode === undefined
      ? item.status || "running"
      : `exit ${item.exitCode}`;
  return `${cwd} • ${exitCode}`;
}

export function formatCommandExecutionDetails(
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

  return sections.join("\n\n");
}

export function summarizeCommandExecutionNotification(
  item: Extract<ThreadItem, { type: "commandExecutionNotification" }>,
  commandLookup: Map<string, string>,
) {
  const commandLabel =
    commandLookup.get(item.commandItemId) ?? item.commandItemId;
  const kind = item.kind || "notification";
  if (item.kind === "output") {
    return `Command notification • output • ${commandLabel}`;
  }

  if (item.kind === "exit") {
    const exitCode =
      item.exitCode === null || item.exitCode === undefined
        ? "unknown exit"
        : `exit ${item.exitCode}`;
    return `Command notification • ${exitCode} • ${commandLabel}`;
  }

  return `Command notification • ${kind} • ${commandLabel}`;
}

export function statusForCommandExecutionNotification(
  item: Extract<ThreadItem, { type: "commandExecutionNotification" }>,
) {
  if (
    item.kind === "exit" &&
    item.exitCode !== null &&
    item.exitCode !== undefined
  ) {
    return item.exitCode === 0 ? "completed" : "failed";
  }
  return "completed";
}

export function formatCommandExecutionNotificationDetails(
  item: Extract<ThreadItem, { type: "commandExecutionNotification" }>,
  commandLabel: string,
) {
  const sections = [
    `Kind\n${item.kind || "notification"}`,
    `Command\n${commandLabel}`,
    `Command ID\n${item.commandItemId}`,
  ];

  if (item.exitCode !== null && item.exitCode !== undefined) {
    sections.push(`Exit Code\n${item.exitCode}`);
  }

  if (item.message) {
    sections.push(`Message\n${item.message}`);
  }

  return sections.join("\n\n");
}

export function commandExecutionNotificationOutput(
  item: Extract<ThreadItem, { type: "commandExecutionNotification" }>,
) {
  if (item.output === null || item.output === undefined) {
    return undefined;
  }
  return {
    label: item.kind === "exit" ? "Command exit output" : "Command output",
    text: item.output,
    isEmpty: item.output.length === 0,
    terminalEmulated: true,
  };
}

export function isLegacyOrphanCommandOutputPlaceholder(
  item: Extract<ThreadItem, { type: "commandExecution" }>,
) {
  return (
    item.command === "Command output" &&
    item.cwd === "cwd pending" &&
    (item.status === "running" || item.status === "inProgress") &&
    item.exitCode === null
  );
}

export function buildCommandLookup(thread: Thread) {
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

export function summarizeCommandWait(
  item: Extract<ThreadItem, { type: "commandWait" }>,
) {
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

export function summarizeCommandWriteStdin(
  item: Extract<ThreadItem, { type: "commandWriteStdin" }>,
) {
  const suffix = item.containsNewline ? " including newline" : "";
  return `Wrote ${item.bytesWritten} bytes to command ${item.commandId}${suffix}.`;
}

export function summarizeEventCommandCall(
  item: Extract<ThreadItem, { type: "eventCommandCall" }>,
) {
  const label = stringOrNull(item.label);
  return label ? `${label} • ${item.command}` : item.command;
}

export function summarizeEventCommandEvent(
  item: Extract<ThreadItem, { type: "eventCommandEvent" }>,
) {
  const label = stringOrNull(item.label) ?? item.command;
  switch (item.kind) {
    case "output":
      return item.line
        ? `${label}: ${item.line}`
        : `${label}: output received.`;
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
      return item.message
        ? `${label}: ${item.message}`
        : `${label}: ${item.kind}.`;
  }
}

export function formatStructuredToolDetails(input: unknown, output: unknown) {
  const sections: string[] = [];

  if (input !== null && input !== undefined) {
    sections.push(`Arguments\n${safeJson(input)}`);
  }

  if (output !== null && output !== undefined) {
    sections.push(`Output\n${safeJson(output)}`);
  }

  return sections.join("\n\n");
}

export function formatEventCommandCallDetails(
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

export function extractEventDrivenSummaryDetails(tool: string, args: unknown) {
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
      const schedule = formatScheduleArgument(record.schedule);
      if (label) {
        return `${label} • ${schedule ?? "schedule"}`;
      }
      return schedule;
    default:
      return label;
  }
}

export function toolCategoryForName(tool: string, namespace?: string | null) {
  if (tool === "read_agent") {
    return "multiAgent" as const;
  }
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
