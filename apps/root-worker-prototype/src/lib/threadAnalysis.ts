import {
  buildContextUsageAnalysis,
  type ContextUsageAnalysis,
} from "./contextUsage";
import type { Thread, ThreadItem } from "../types";
import {
  buildScheduleOccurrences,
  formatScheduleArgument,
  formatScheduleRule,
} from "./scheduleDisplay";

export type MonitorKind = "command" | "schedule";

export type MonitorSummary = {
  id: string;
  subscriptionId: string | null;
  kind: MonitorKind;
  label: string;
  detail: string;
  status: string;
  eventCount: number;
  latestEvent: string | null;
};

type InternalMonitorSummary = MonitorSummary & {
  schedule?: unknown;
  scheduleRule?: string | null;
  nextFireAt?: string | null;
};

export type MonitorSection = {
  kind: MonitorKind;
  title: string;
  emptyLabel: string;
  monitors: MonitorSummary[];
};

export type ChangedFileSummary = {
  path: string;
  displayPath: string;
  kind: string;
  updateCount: number;
};

export type ThreadAnalysis = {
  contextUsage: ContextUsageAnalysis;
  monitors: {
    totalCount: number;
    eventCount: number;
    sections: MonitorSection[];
    scheduleAgenda: ScheduleAgendaGroup[];
  };
  changedFiles: ChangedFileSummary[];
};

export type ScheduleAgendaGroup = {
  dateKey: string;
  dateLabel: string;
  items: ScheduleAgendaItem[];
};

export type ScheduleAgendaItem = {
  id: string;
  subscriptionId: string | null;
  label: string;
  rule: string;
  startsAt: string;
  timeLabel: string;
};

export type ThreadAnalysisOptions = {
  now?: Date | string | number;
  agendaLimit?: number;
  agendaHorizonDays?: number;
  agendaTimeZone?: string;
};

type MonitorEvent = {
  displayText: string;
  matchText: string;
};

const MONITOR_TOOLS = {
  schedule_subscribe: "schedule",
} as const satisfies Record<string, MonitorKind>;

const UNSUBSCRIBE_TOOLS = {
  schedule_unsubscribe: "schedule",
} as const satisfies Record<string, MonitorKind>;

const MONITOR_SECTIONS: Array<{
  kind: MonitorKind;
  title: string;
  emptyLabel: string;
}> = [
  {
    kind: "command",
    title: "Live Commands",
    emptyLabel: "No live commands.",
  },
  {
    kind: "schedule",
    title: "Schedules",
    emptyLabel: "No scheduled listeners.",
  },
];

export function buildThreadAnalysis(
  thread: Thread | null,
  totalSkillMetadataCount: number,
  modelContextWindowOverride?: number | null,
  options: ThreadAnalysisOptions = {},
): ThreadAnalysis {
  const contextUsage = buildContextUsageAnalysis(
    thread,
    totalSkillMetadataCount,
    modelContextWindowOverride,
  );
  const monitors = buildMonitorSections(thread, options);

  return {
    contextUsage,
    monitors,
    changedFiles: buildChangedFiles(thread),
  };
}

export function hasActiveMonitors(thread: Thread | null) {
  return buildMonitorSections(thread, {}).totalCount > 0;
}

function buildMonitorSections(
  thread: Thread | null,
  options: ThreadAnalysisOptions,
): ThreadAnalysis["monitors"] {
  const monitors: InternalMonitorSummary[] = [];
  const eventsByTool = new Map<string, MonitorEvent[]>();
  const commandNotificationsByCommandId = new Map<string, string>();
  const allowLiveCommandMonitors = threadAllowsLiveCommandMonitors(thread);

  if (thread) {
    for (const turn of thread.turns) {
      for (const item of turn.items) {
        if (isMonitorToolCall(item)) {
          const monitor = buildMonitorSummary(item);
          if (monitor) {
            upsertMonitorSummary(monitors, monitor);
          }
          continue;
        }

        if (isUnsubscribeToolCall(item)) {
          removeUnsubscribedMonitor(monitors, item);
          continue;
        }

        if (item.type === "eventDrivenTool" && isMonitorTool(item.tool)) {
          const events = eventsByTool.get(item.tool) ?? [];
          events.push(buildMonitorEvent(item));
          eventsByTool.set(item.tool, events);
        }

        if (item.type === "commandExecution") {
          const commandMonitor = buildCommandMonitorSummary(
            item,
            commandNotificationsByCommandId.get(item.id) ?? null,
            allowLiveCommandMonitors,
          );
          if (commandMonitor) {
            monitors.push(commandMonitor);
          }
          continue;
        }

        if (item.type === "commandExecutionNotification") {
          const summary = summarizeCommandNotification(item);
          commandNotificationsByCommandId.set(
            item.commandItemId,
            summary,
          );
          const existingMonitor = monitors.find(
            (monitor) => monitor.id === item.commandItemId,
          );
          if (existingMonitor) {
            existingMonitor.latestEvent = summary;
            existingMonitor.eventCount = Math.max(existingMonitor.eventCount, 1);
          }
          continue;
        }
      }
    }
  }

  const activeMonitors: InternalMonitorSummary[] = [];
  for (const monitor of monitors) {
    const tool = toolFromMonitorKind(monitor.kind);
    const events = tool ? (eventsByTool.get(tool) ?? []) : [];
    const matchingEvents = events.filter((event) =>
      monitorMatchesEvent(monitor, event.matchText),
    );
    const fallbackEvents =
      monitors.filter((candidate) => candidate.kind === monitor.kind).length ===
      1
        ? events
        : [];
    const observedEvents =
      matchingEvents.length > 0 ? matchingEvents : fallbackEvents;

    if (observedEvents.length > 0) {
      activeMonitors.push({
        ...monitor,
        eventCount: observedEvents.length,
        latestEvent: observedEvents.at(-1)?.displayText ?? null,
      });
      continue;
    }

    activeMonitors.push(monitor);
  }

  const publicMonitors = activeMonitors.map(toPublicMonitorSummary);
  const sections = MONITOR_SECTIONS.map((section) => ({
    ...section,
    monitors: publicMonitors.filter((monitor) => monitor.kind === section.kind),
  }));

  return {
    totalCount: activeMonitors.length,
    eventCount:
      [...eventsByTool.values()].reduce((sum, events) => sum + events.length, 0) +
      activeMonitors
        .filter((monitor) => monitor.kind === "command")
        .reduce((sum, monitor) => sum + monitor.eventCount, 0),
    sections,
    scheduleAgenda: buildScheduleAgenda(activeMonitors, options),
  };
}

function buildChangedFiles(thread: Thread | null): ChangedFileSummary[] {
  if (!thread) {
    return [];
  }

  const changedFilesByPath = new Map<
    string,
    ChangedFileSummary & { lastSeenOrder: number }
  >();
  let changeOrder = 0;

  for (const turn of thread.turns) {
    for (const item of turn.items) {
      if (item.type !== "fileChange" || item.status !== "completed") {
        continue;
      }

      for (const change of item.changes) {
        const filePath = stringOrNull(change.path);
        if (!filePath) {
          continue;
        }

        changeOrder += 1;
        const existing = changedFilesByPath.get(filePath);
        changedFilesByPath.set(filePath, {
          path: filePath,
          displayPath: displayThreadFilePath(thread.cwd, filePath),
          kind: stringOrNull(change.kind) ?? "modified",
          updateCount: (existing?.updateCount ?? 0) + 1,
          lastSeenOrder: changeOrder,
        });
      }
    }
  }

  return [...changedFilesByPath.values()]
    .sort(
      (left, right) =>
        right.lastSeenOrder - left.lastSeenOrder ||
        left.displayPath.localeCompare(right.displayPath),
    )
    .map(({ lastSeenOrder: _lastSeenOrder, ...file }) => file);
}

function buildCommandMonitorSummary(
  item: Extract<ThreadItem, { type: "commandExecution" }>,
  latestNotification: string | null,
  allowLiveCommandMonitors: boolean,
): InternalMonitorSummary | null {
  const status = statusLabel(item.status);
  if (!allowLiveCommandMonitors || !isRunningCommandStatus(item.status)) {
    return null;
  }
  const latestOutput = stringOrNull(item.aggregatedOutput)
    ?.split(/\r?\n/)
    .filter(Boolean)
    .at(-1) ?? null;
  return {
    id: item.id,
    subscriptionId: item.id,
    kind: "command",
    label: item.command,
    detail: item.cwd,
    status,
    eventCount: latestNotification || latestOutput ? 1 : 0,
    latestEvent: latestNotification ?? latestOutput,
  };
}

function isRunningCommandStatus(status: string) {
  const normalized = status.trim().toLowerCase().replace(/[_-]/g, "");
  return normalized === "running" || normalized === "inprogress";
}

function threadAllowsLiveCommandMonitors(thread: Thread | null) {
  if (!thread) {
    return false;
  }
  if (thread.lifecycleStatus.type === "active") {
    return thread.lifecycleStatus.activeFlags.includes("running");
  }
  if (thread.lifecycleStatus.type === "idle") {
    return (
      thread.lifecycleStatus.reason === "waitCommand" ||
      thread.lifecycleStatus.reason === "waitChild"
    );
  }
  return (
    thread.lifecycleStatus.type === "waiting" &&
    (thread.lifecycleStatus.reason === "command" ||
      thread.lifecycleStatus.reason === "child")
  );
}

function summarizeCommandNotification(
  item: Extract<ThreadItem, { type: "commandExecutionNotification" }>,
) {
  if (item.kind === "output") {
    return stringOrNull(item.output) ?? "Output notification";
  }
  if (item.kind === "exit") {
    return item.exitCode === null || item.exitCode === undefined
      ? "Exit notification"
      : `Exit notification ${item.exitCode}`;
  }
  return item.message;
}

function buildMonitorSummary(
  item: Extract<ThreadItem, { type: "eventDrivenToolCall" | "builtinToolCall" }>,
): InternalMonitorSummary | null {
  const subscriptionId = subscriptionIdFromOutput(item.output);
  if (item.status !== "completed" || !subscriptionId) {
    return null;
  }
  const args = objectRecord(item.arguments);
  const output = objectRecord(item.output);
  const kind = MONITOR_TOOLS[item.tool as keyof typeof MONITOR_TOOLS];
  const schedule = args.schedule;

  return {
    id: item.id,
    subscriptionId,
    kind,
    label: monitorLabel(kind, args),
    detail: monitorDetail(kind, args, item.output),
    status: item.status === "completed" ? "Listening" : statusLabel(item.status),
    eventCount: 0,
    latestEvent: null,
    schedule,
    scheduleRule: kind === "schedule" ? formatScheduleRule(schedule) : null,
    nextFireAt: kind === "schedule" ? stringOrNull(output.next_fire_at) : null,
  };
}

function removeUnsubscribedMonitor(
  monitors: InternalMonitorSummary[],
  item: Extract<ThreadItem, { type: "eventDrivenToolCall" | "builtinToolCall" }>,
) {
  if (item.status !== "completed" || objectRecord(item.output).unsubscribed !== true) {
    return;
  }
  const args = objectRecord(item.arguments);
  const subscriptionId = stringOrNull(args.subscription_id);
  if (!subscriptionId) {
    return;
  }

  const monitorIndex = monitors.findIndex(
    (monitor) => monitor.subscriptionId === subscriptionId,
  );
  if (monitorIndex !== -1) {
    monitors.splice(monitorIndex, 1);
  }
}

function upsertMonitorSummary(
  monitors: InternalMonitorSummary[],
  monitor: InternalMonitorSummary,
) {
  const existingIndex = monitors.findIndex(
    (existing) => existing.subscriptionId === monitor.subscriptionId,
  );
  if (existingIndex === -1) {
    monitors.push(monitor);
    return;
  }
  monitors[existingIndex] = monitor;
}

function toPublicMonitorSummary(monitor: InternalMonitorSummary): MonitorSummary {
  const {
    schedule: _schedule,
    scheduleRule: _scheduleRule,
    nextFireAt: _nextFireAt,
    ...summary
  } = monitor;
  return summary;
}

function monitorLabel(_kind: MonitorKind, args: Record<string, unknown>) {
  const label = stringOrNull(args.label);
  if (label) {
    return label;
  }

  return "Schedule";
}

function monitorDetail(
  _kind: MonitorKind,
  args: Record<string, unknown>,
  output: unknown,
) {
  const outputRecord = objectRecord(output);
  return (
    formatScheduleArgument(args.schedule) ??
    stringOrNull(outputRecord.schedule_summary) ??
    displayUnknown(args.schedule) ??
    "schedule unavailable"
  );
}

function buildScheduleAgenda(
  monitors: InternalMonitorSummary[],
  options: ThreadAnalysisOptions,
) {
  const now = normalizeDate(options.now) ?? new Date();
  const limit = options.agendaLimit ?? 20;
  const horizonDays = options.agendaHorizonDays ?? 7;
  const timeZone = options.agendaTimeZone;
  const items = monitors
    .filter((monitor) => monitor.kind === "schedule")
    .flatMap((monitor) => {
      const rule = monitor.scheduleRule ?? formatScheduleArgument(monitor.schedule);
      if (!rule) {
        return [];
      }
      return buildScheduleOccurrences(monitor.schedule, {
        now,
        nextFireAt: monitor.nextFireAt,
        limit,
        horizonDays,
      }).map((occurrence) => ({
        id: `${monitor.id}:${occurrence.startsAt}`,
        subscriptionId: monitor.subscriptionId,
        label: monitor.label,
        rule,
        startsAt: occurrence.startsAt,
        timeLabel: formatAgendaTime(occurrence.startsAt, timeZone),
      }));
    })
    .sort((left, right) => left.startsAt.localeCompare(right.startsAt))
    .slice(0, limit);

  const groups = new Map<string, ScheduleAgendaGroup>();
  for (const item of items) {
    const dateKey = formatAgendaDateKey(item.startsAt, timeZone);
    const existing = groups.get(dateKey);
    if (existing) {
      existing.items.push(item);
      continue;
    }
    groups.set(dateKey, {
      dateKey,
      dateLabel: formatAgendaDateLabel(item.startsAt, now, timeZone),
      items: [item],
    });
  }
  return [...groups.values()];
}

function formatAgendaTime(value: string, timeZone: string | undefined) {
  return new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
    timeZone,
  }).format(new Date(value));
}

function formatAgendaDateKey(value: string, timeZone: string | undefined) {
  const parts = agendaDateParts(new Date(value), timeZone);
  return `${parts.year}-${String(parts.month).padStart(2, "0")}-${String(parts.day).padStart(2, "0")}`;
}

function formatAgendaDateLabel(
  value: string,
  now: Date,
  timeZone: string | undefined,
) {
  const date = new Date(value);
  const dateKey = formatAgendaDateKey(value, timeZone);
  const todayKey = formatAgendaDateKey(now.toISOString(), timeZone);
  const tomorrowKey = formatAgendaDateKey(
    new Date(now.getTime() + 24 * 60 * 60 * 1000).toISOString(),
    timeZone,
  );
  if (dateKey === todayKey) {
    return "Today";
  }
  if (dateKey === tomorrowKey) {
    return "Tomorrow";
  }
  return new Intl.DateTimeFormat(undefined, {
    weekday: "short",
    month: "short",
    day: "numeric",
    timeZone,
  }).format(date);
}

function agendaDateParts(date: Date, timeZone: string | undefined) {
  const formatter = new Intl.DateTimeFormat("en-US", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    timeZone,
  });
  const parts = Object.fromEntries(
    formatter
      .formatToParts(date)
      .filter((part) => part.type !== "literal")
      .map((part) => [part.type, Number(part.value)]),
  );
  return {
    year: parts.year,
    month: parts.month,
    day: parts.day,
  };
}

function buildMonitorEvent(
  item: Extract<ThreadItem, { type: "eventDrivenTool" }>,
) {
  const displayText = item.text || item.title;
  return {
    displayText,
    matchText: [item.title, item.text].filter(Boolean).join("\n"),
  };
}

function monitorMatchesEvent(monitor: MonitorSummary, event: string) {
  return event.includes(monitor.label) || event.includes(monitor.detail);
}

function statusLabel(status: string) {
  if (status === "completed") {
    return "Completed";
  }

  if (status === "failed") {
    return "Failed";
  }

  if (isRunningCommandStatus(status)) {
    return "Running";
  }

  return status || "Unknown";
}

function isMonitorTool(tool: string): tool is keyof typeof MONITOR_TOOLS {
  return Object.prototype.hasOwnProperty.call(MONITOR_TOOLS, tool);
}

function isUnsubscribeTool(
  tool: string,
): tool is keyof typeof UNSUBSCRIBE_TOOLS {
  return Object.prototype.hasOwnProperty.call(UNSUBSCRIBE_TOOLS, tool);
}

function isMonitorToolCall(
  item: ThreadItem,
): item is Extract<ThreadItem, { type: "eventDrivenToolCall" | "builtinToolCall" }> {
  return (
    (item.type === "eventDrivenToolCall" || item.type === "builtinToolCall") &&
    isMonitorTool(item.tool)
  );
}

function isUnsubscribeToolCall(
  item: ThreadItem,
): item is Extract<ThreadItem, { type: "eventDrivenToolCall" | "builtinToolCall" }> {
  return (
    (item.type === "eventDrivenToolCall" || item.type === "builtinToolCall") &&
    isUnsubscribeTool(item.tool)
  );
}

function toolFromMonitorKind(kind: MonitorKind) {
  for (const [tool, toolKind] of Object.entries(MONITOR_TOOLS)) {
    if (toolKind === kind) {
      return tool;
    }
  }

  return null;
}

function objectRecord(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return {};
  }

  return value as Record<string, unknown>;
}

function stringOrNull(value: unknown) {
  return typeof value === "string" && value.trim().length > 0
    ? value.trim()
    : null;
}

function subscriptionIdFromOutput(output: unknown) {
  return stringOrNull(objectRecord(output).subscription_id);
}

function normalizeDate(value: Date | string | number | undefined) {
  if (value instanceof Date) {
    return Number.isNaN(value.getTime()) ? null : value;
  }
  if (typeof value === "string" || typeof value === "number") {
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? null : date;
  }
  return null;
}

function displayUnknown(value: unknown) {
  if (typeof value === "string") {
    return stringOrNull(value);
  }

  if (!value || typeof value !== "object") {
    return null;
  }

  try {
    return JSON.stringify(value);
  } catch {
    return null;
  }
}

function displayThreadFilePath(threadCwd: string, filePath: string) {
  if (!isAbsoluteFilePath(filePath)) {
    return filePath;
  }

  const normalizedCwd = normalizeFilePath(threadCwd).replace(/\/+$/, "");
  const normalizedFilePath = normalizeFilePath(filePath);

  if (normalizedFilePath === normalizedCwd) {
    return normalizedFilePath.split("/").at(-1) ?? filePath;
  }

  const cwdPrefix = `${normalizedCwd}/`;
  if (normalizedCwd && normalizedFilePath.startsWith(cwdPrefix)) {
    return normalizedFilePath.slice(cwdPrefix.length);
  }

  return filePath;
}

function normalizeFilePath(value: string) {
  return value.replace(/\\/g, "/");
}

function isAbsoluteFilePath(value: string) {
  return value.startsWith("/") || /^[A-Za-z]:\//.test(normalizeFilePath(value));
}
