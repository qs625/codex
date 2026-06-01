import {
  buildContextUsageAnalysis,
  type ContextUsageAnalysis,
} from "./contextUsage";
import type { Thread, ThreadItem } from "../types";

export type MonitorKind = "filesystem" | "process" | "schedule";

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

export type MonitorSection = {
  kind: MonitorKind;
  title: string;
  emptyLabel: string;
  monitors: MonitorSummary[];
};

export type ThreadAnalysis = {
  contextUsage: ContextUsageAnalysis;
  monitors: {
    totalCount: number;
    eventCount: number;
    sections: MonitorSection[];
  };
};

const MONITOR_TOOLS = {
  fs_subscribe: "filesystem",
  process_exit_subscribe: "process",
  schedule_subscribe: "schedule",
} as const satisfies Record<string, MonitorKind>;

const UNSUBSCRIBE_TOOLS = {
  fs_unsubscribe: "filesystem",
  process_exit_unsubscribe: "process",
  schedule_unsubscribe: "schedule",
} as const satisfies Record<string, MonitorKind>;

const MONITOR_SECTIONS: Array<{
  kind: MonitorKind;
  title: string;
  emptyLabel: string;
}> = [
  {
    kind: "filesystem",
    title: "Filesystem",
    emptyLabel: "No file watches.",
  },
  {
    kind: "process",
    title: "Processes",
    emptyLabel: "No process listeners.",
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
): ThreadAnalysis {
  const contextUsage = buildContextUsageAnalysis(
    thread,
    totalSkillMetadataCount,
  );
  const monitors = buildMonitorSections(thread);

  return {
    contextUsage,
    monitors,
  };
}

function buildMonitorSections(
  thread: Thread | null,
): ThreadAnalysis["monitors"] {
  const monitors: MonitorSummary[] = [];
  const eventsByTool = new Map<string, string[]>();

  if (thread) {
    for (const turn of thread.turns) {
      for (const item of turn.items) {
        if (item.type === "eventDrivenToolCall" && isMonitorTool(item.tool)) {
          monitors.push(buildMonitorSummary(item));
          continue;
        }

        if (item.type === "eventDrivenToolCall" && isUnsubscribeTool(item.tool)) {
          removeUnsubscribedMonitor(monitors, item);
          continue;
        }

        if (item.type === "eventDrivenTool" && isMonitorTool(item.tool)) {
          const events = eventsByTool.get(item.tool) ?? [];
          events.push(item.text || item.title);
          eventsByTool.set(item.tool, events);
        }
      }
    }
  }

  const activeMonitors: MonitorSummary[] = [];
  for (const monitor of monitors) {
    const tool = toolFromMonitorKind(monitor.kind);
    const events = eventsByTool.get(tool) ?? [];
    const matchingEvents = events.filter((event) =>
      monitorMatchesEvent(monitor, event),
    );
    const fallbackEvents =
      monitors.filter((candidate) => candidate.kind === monitor.kind).length ===
      1
        ? events
        : [];
    const observedEvents =
      matchingEvents.length > 0 ? matchingEvents : fallbackEvents;

    if (observedEvents.length === 0) {
      activeMonitors.push(monitor);
      continue;
    }
  }

  const sections = MONITOR_SECTIONS.map((section) => ({
    ...section,
    monitors: activeMonitors.filter((monitor) => monitor.kind === section.kind),
  }));

  return {
    totalCount: activeMonitors.length,
    eventCount: [...eventsByTool.values()].reduce(
      (sum, events) => sum + events.length,
      0,
    ),
    sections,
  };
}

function buildMonitorSummary(
  item: Extract<ThreadItem, { type: "eventDrivenToolCall" }>,
): MonitorSummary {
  const args = objectRecord(item.arguments);
  const kind = MONITOR_TOOLS[item.tool as keyof typeof MONITOR_TOOLS];

  return {
    id: item.id,
    subscriptionId: subscriptionIdFromOutput(item.output),
    kind,
    label: monitorLabel(kind, args),
    detail: monitorDetail(kind, args, item.output),
    status: statusLabel(item.status),
    eventCount: 0,
    latestEvent: null,
  };
}

function removeUnsubscribedMonitor(
  monitors: MonitorSummary[],
  item: Extract<ThreadItem, { type: "eventDrivenToolCall" }>,
) {
  const args = objectRecord(item.arguments);
  const subscriptionId = stringOrNull(args.subscription_id);
  if (!subscriptionId) {
    return;
  }

  const monitorIndex = monitors.findIndex((monitor) => monitor.subscriptionId === subscriptionId);
  if (monitorIndex !== -1) {
    monitors.splice(monitorIndex, 1);
  }
}

function monitorLabel(kind: MonitorKind, args: Record<string, unknown>) {
  const label = stringOrNull(args.label);
  if (label) {
    return label;
  }

  if (kind === "filesystem") {
    return stringOrNull(args.path) ?? "File watch";
  }

  if (kind === "process") {
    const sessionId = numberOrNull(args.session_id);
    return sessionId === null ? "Process listener" : `Session ${sessionId}`;
  }

  return "Schedule";
}

function monitorDetail(
  kind: MonitorKind,
  args: Record<string, unknown>,
  output: unknown,
) {
  if (kind === "filesystem") {
    const path = stringOrNull(args.path) ?? "path unavailable";
    return args.recursive === true ? `${path} (recursive)` : path;
  }

  if (kind === "process") {
    const sessionId = numberOrNull(args.session_id);
    return sessionId === null ? "session unavailable" : `session ${sessionId}`;
  }

  const outputRecord = objectRecord(output);
  return (
    stringOrNull(outputRecord.schedule_summary) ??
    displayUnknown(args.schedule) ??
    "schedule unavailable"
  );
}

function monitorMatchesEvent(monitor: MonitorSummary, event: string) {
  return event.includes(monitor.label) || event.includes(monitor.detail);
}

function statusLabel(status: string) {
  if (status === "completed") {
    return "Listening";
  }

  if (status === "failed") {
    return "Failed";
  }

  if (status === "running" || status === "inProgress") {
    return "Subscribing";
  }

  return status || "Unknown";
}

function isMonitorTool(tool: string): tool is keyof typeof MONITOR_TOOLS {
  return Object.prototype.hasOwnProperty.call(MONITOR_TOOLS, tool);
}

function isUnsubscribeTool(tool: string): tool is keyof typeof UNSUBSCRIBE_TOOLS {
  return Object.prototype.hasOwnProperty.call(UNSUBSCRIBE_TOOLS, tool);
}

function toolFromMonitorKind(kind: MonitorKind) {
  for (const [tool, toolKind] of Object.entries(MONITOR_TOOLS)) {
    if (toolKind === kind) {
      return tool;
    }
  }

  return "fs_subscribe";
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

function numberOrNull(value: unknown) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function subscriptionIdFromOutput(output: unknown) {
  return stringOrNull(objectRecord(output).subscription_id);
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
