import type { Thread, ThreadItem } from "../types";
import { formatScheduleArgument } from "./scheduleDisplay";
import { trimPath } from "./thread";
import {
  formatMillisecondsDuration,
  formatSecondsDuration,
  previewBlockText,
  safeJson,
  stringOrNull,
} from "./conversationFormatting";

const COMMAND_OUTPUT_PREVIEW_MAX_CHARS = 12_000;
const COMMAND_OUTPUT_PREVIEW_MAX_LINES = 240;

type CommandExecutionItem = Extract<ThreadItem, { type: "commandExecution" }>;
type CommandExecutionNotificationItem = Extract<
  ThreadItem,
  { type: "commandExecutionNotification" }
>;
type CommandWaitItem = Extract<ThreadItem, { type: "commandWait" }>;
type CommandWriteStdinItem = Extract<ThreadItem, { type: "commandWriteStdin" }>;
type EventCommandCallItem = Extract<ThreadItem, { type: "eventCommandCall" }>;
type EventCommandEventItem = Extract<ThreadItem, { type: "eventCommandEvent" }>;

export function summarizeCommandExecution(item: CommandExecutionItem) {
  return new CommandExecutionPresentation(item).summary();
}

function displayCommandExecutionStatus(status: string | null | undefined) {
  if (!status) {
    return "running";
  }
  const normalized = status.trim().toLowerCase().replace(/[_-]/g, "");
  return normalized === "inprogress" ? "running" : status;
}

export function formatCommandExecutionDetails(item: CommandExecutionItem) {
  return new CommandExecutionPresentation(item).details();
}

export function summarizeCommandExecutionNotification(
  item: CommandExecutionNotificationItem,
  commandLookup: Map<string, string>,
) {
  return new CommandExecutionNotificationPresentation(
    item,
    commandLookup,
  ).summary();
}

export function statusForCommandExecutionNotification(
  item: CommandExecutionNotificationItem,
) {
  return new CommandExecutionNotificationPresentation(item).status();
}

export function formatCommandExecutionNotificationDetails(
  item: CommandExecutionNotificationItem,
  commandLabel: string,
) {
  return new CommandExecutionNotificationPresentation(
    item,
    undefined,
    commandLabel,
  ).details();
}

export function commandExecutionNotificationOutput(
  item: CommandExecutionNotificationItem,
) {
  return new CommandExecutionNotificationPresentation(item).output();
}

export function isLegacyOrphanCommandOutputPlaceholder(
  item: CommandExecutionItem,
) {
  return (
    item.command === "Command output" &&
    item.cwd === "cwd pending" &&
    (item.status === "running" || item.status === "inProgress") &&
    item.exitCode === null
  );
}

export function buildCommandLookup(thread: Thread) {
  return CommandLookup.fromThread(thread).commands;
}

export function summarizeCommandWait(item: CommandWaitItem) {
  return new CommandWaitPresentation(item).summary();
}

export function summarizeCommandWriteStdin(item: CommandWriteStdinItem) {
  return new CommandWriteStdinPresentation(item).summary();
}

export function summarizeEventCommandCall(item: EventCommandCallItem) {
  return new EventCommandCallPresentation(item).summary();
}

export function summarizeEventCommandEvent(item: EventCommandEventItem) {
  return new EventCommandEventPresentation(item).summary();
}

export function formatStructuredToolDetails(input: unknown, output: unknown) {
  return new StructuredToolDetailsPresentation(input, output).details();
}

export function formatEventCommandCallDetails(item: EventCommandCallItem) {
  return new EventCommandCallPresentation(item).details();
}

export function extractEventDrivenSummaryDetails(tool: string, args: unknown) {
  return EventDrivenToolSummary.from(tool, args);
}

export function toolCategoryForName(tool: string, namespace?: string | null) {
  return ToolCategoryClassifier.categoryFor(tool, namespace);
}

class CommandExecutionPresentation {
  constructor(private readonly item: CommandExecutionItem) {}

  summary() {
    const cwd = trimPath(this.item.cwd);
    const exitCode =
      this.item.exitCode === null || this.item.exitCode === undefined
        ? displayCommandExecutionStatus(this.item.status)
        : `exit ${this.item.exitCode}`;
    return `${cwd} • ${exitCode}`;
  }

  details() {
    const sections = [
      `Command\n${this.item.command}`,
      `Cwd\n${this.item.cwd}`,
      `Status\n${this.item.status}`,
    ];

    if (
      this.item.initialWaitMs !== null &&
      this.item.initialWaitMs !== undefined
    ) {
      sections.push(`Initial Wait\n${this.item.initialWaitMs} ms`);
    }

    if (this.item.notifyOn) {
      sections.push(`Notify On\n${this.item.notifyOn}`);
    }

    if (this.item.durationMs !== null && this.item.durationMs !== undefined) {
      sections.push(`Duration\n${this.item.durationMs} ms`);
    }

    if (this.item.exitCode !== null && this.item.exitCode !== undefined) {
      sections.push(`Exit Code\n${this.item.exitCode}`);
    }

    return sections.join("\n\n");
  }
}

class CommandExecutionNotificationPresentation {
  constructor(
    private readonly item: CommandExecutionNotificationItem,
    private readonly commandLookup?: ReadonlyMap<string, string>,
    private readonly commandLabelOverride?: string,
  ) {}

  summary() {
    const kind = this.item.kind || "notification";
    if (this.item.kind === "output") {
      return `Command notification • output • ${this.commandLabel()}`;
    }

    if (this.item.kind === "exit") {
      const exitCode =
        this.item.exitCode === null || this.item.exitCode === undefined
          ? "unknown exit"
          : `exit ${this.item.exitCode}`;
      return `Command notification • ${exitCode} • ${this.commandLabel()}`;
    }

    return `Command notification • ${kind} • ${this.commandLabel()}`;
  }

  status() {
    if (
      this.item.kind === "exit" &&
      this.item.exitCode !== null &&
      this.item.exitCode !== undefined
    ) {
      return this.item.exitCode === 0 ? "completed" : "failed";
    }
    return "completed";
  }

  details() {
    const sections = [
      `Kind\n${this.item.kind || "notification"}`,
      `Command\n${this.commandLabel()}`,
      `Command ID\n${this.item.commandItemId}`,
    ];

    if (this.item.exitCode !== null && this.item.exitCode !== undefined) {
      sections.push(`Exit Code\n${this.item.exitCode}`);
    }

    if (this.item.message) {
      sections.push(`Message\n${this.item.message}`);
    }

    return sections.join("\n\n");
  }

  output() {
    if (this.item.output === null || this.item.output === undefined) {
      return undefined;
    }
    const outputPreview = previewBlockText(this.item.output, {
      maxChars: COMMAND_OUTPUT_PREVIEW_MAX_CHARS,
      maxLines: COMMAND_OUTPUT_PREVIEW_MAX_LINES,
    });
    return {
      label:
        this.item.kind === "exit" ? "Command exit output" : "Command output",
      text: outputPreview.text,
      isEmpty: this.item.output.length === 0,
      terminalEmulated: true,
    };
  }

  private commandLabel() {
    return (
      this.commandLabelOverride ??
      this.commandLookup?.get(this.item.commandItemId) ??
      this.item.commandItemId
    );
  }
}

class CommandLookup {
  private constructor(readonly commands: Map<string, string>) {}

  static fromThread(thread: Thread) {
    const commandLookup = new Map<string, string>();
    for (const turn of thread.turns) {
      for (const item of turn.items) {
        if (item.type === "commandExecution") {
          commandLookup.set(item.id, item.command);
        }
      }
    }
    return new CommandLookup(commandLookup);
  }
}

class CommandWaitPresentation {
  constructor(private readonly item: CommandWaitItem) {}

  summary() {
    const notification = this.item.notification
      ? ` after ${this.item.notification} notification`
      : "";
    const exitCode =
      this.item.exitCode === null || this.item.exitCode === undefined
        ? ""
        : `, exit ${this.item.exitCode}`;
    const seconds = Number.isFinite(this.item.wallTimeSeconds)
      ? ` in ${formatSecondsDuration(this.item.wallTimeSeconds)}`
      : "";
    const timeout = Number.isFinite(this.item.waitTimeoutMs)
      ? ` with timeout ${formatMillisecondsDuration(this.item.waitTimeoutMs)}`
      : "";
    return `Waited for command ${this.item.commandId}${timeout}${notification}: ${this.item.status}${exitCode}${seconds}.`;
  }
}

class CommandWriteStdinPresentation {
  constructor(private readonly item: CommandWriteStdinItem) {}

  summary() {
    const suffix = this.item.containsNewline ? " including newline" : "";
    return `Wrote ${this.item.bytesWritten} bytes to command ${this.item.commandId}${suffix}.`;
  }
}

class EventCommandCallPresentation {
  constructor(private readonly item: EventCommandCallItem) {}

  summary() {
    const label = stringOrNull(this.item.label);
    return label ? `${label} • ${this.item.command}` : this.item.command;
  }

  details() {
    const sections = [`Command\n${this.item.command}`];

    const cwd = stringOrNull(this.item.cwd);
    if (cwd) {
      sections.push(`Directory\n${cwd}`);
    }

    const label = stringOrNull(this.item.label);
    if (label) {
      sections.push(`Label\n${label}`);
    }

    sections.push(`Subscription\n${this.item.subscriptionId}`);

    if (this.item.output !== null && this.item.output !== undefined) {
      sections.push(`Output\n${safeJson(this.item.output)}`);
    }

    return sections.join("\n\n");
  }
}

class EventCommandEventPresentation {
  constructor(private readonly item: EventCommandEventItem) {}

  summary() {
    const label = stringOrNull(this.item.label) ?? this.item.command;
    switch (this.item.kind) {
      case "output":
        return this.item.line
          ? `${label}: ${this.item.line}`
          : `${label}: output received.`;
      case "exited": {
        if (this.item.signal) {
          return `${label}: signal ${this.item.signal}.`;
        }
        const exitCode =
          this.item.exitCode === null || this.item.exitCode === undefined
            ? "unknown exit"
            : `exit ${this.item.exitCode}`;
        return `${label}: ${exitCode}.`;
      }
      case "cancelled":
        return `${label}: cancelled.`;
      case "failedToStart":
        return this.item.message
          ? `${label}: failed to start. ${this.item.message}`
          : `${label}: failed to start.`;
      default:
        return this.item.message
          ? `${label}: ${this.item.message}`
          : `${label}: ${this.item.kind}.`;
    }
  }
}

class StructuredToolDetailsPresentation {
  constructor(
    private readonly input: unknown,
    private readonly output: unknown,
  ) {}

  details() {
    const sections: string[] = [];

    if (this.input !== null && this.input !== undefined) {
      sections.push(`Arguments\n${safeJson(this.input)}`);
    }

    if (this.output !== null && this.output !== undefined) {
      sections.push(`Output\n${safeJson(this.output)}`);
    }

    return sections.join("\n\n");
  }
}

class EventDrivenToolSummary {
  static from(tool: string, args: unknown) {
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
}

class ToolCategoryClassifier {
  static categoryFor(tool: string, namespace?: string | null) {
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
}
