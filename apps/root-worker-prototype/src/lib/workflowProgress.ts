import type {
  Thread,
  ThreadWorkflowRunProgressEvent,
  ThreadWorkflowRunProgressKind,
  WorkflowSummary,
} from "../types";

export type WorkflowStageStatus = "completed" | "current" | "failed" | "aborted" | "pending";

export type WorkflowStageView = {
  id: string;
  label: string;
  status: WorkflowStageStatus;
};

export type WorkflowTimelineItem = {
  id: string;
  runId: string;
  workflowId: string;
  kind: ThreadWorkflowRunProgressKind;
  statusTone: WorkflowStatusTone;
  label: string;
  message: string;
  runnerStatus: string;
  statusLabel: string;
  updatedAt: number;
};

export type WorkflowStatusTone = "running" | "completed" | "failed" | "aborted" | "unknown";

export type WorkflowRunView = {
  runId: string;
  workflowId: string;
  workflowName: string;
  source: WorkflowSummary["source"] | "unknown";
  kind: ThreadWorkflowRunProgressKind;
  statusTone: WorkflowStatusTone;
  statusLabel: string;
  runnerStatus: string;
  message: string;
  updatedAt: number;
  stages: WorkflowStageView[];
  graphSource: "metadata" | "fallback" | "missing";
  graphNote: string | null;
  timeline: WorkflowTimelineItem[];
};

export type WorkflowPanelViewModel = {
  selectedRun: WorkflowRunView | null;
  runs: WorkflowRunView[];
  availableWorkflows: WorkflowSummary[];
  timeline: WorkflowTimelineItem[];
};

type WorkflowProgressRecord = {
  itemId: string;
  event: ThreadWorkflowRunProgressEvent;
  order: number;
};

const FEATURE_DEV_STAGES: Array<{ id: string; label: string }> = [
  { id: "research", label: "Research" },
  { id: "implement", label: "Implement" },
  { id: "review_fix", label: "Review/Fix" },
  { id: "verify", label: "Verify" },
];

const WORKFLOW_TIMELINE_LIMIT = 10;

export function buildWorkflowPanelViewModel(
  thread: Thread | null,
  availableWorkflows: WorkflowSummary[],
): WorkflowPanelViewModel {
  const records = collectWorkflowProgressRecords(thread);
  const workflowsById = new Map(availableWorkflows.map((workflow) => [workflow.id, workflow]));
  const recordsByRun = new Map<string, WorkflowProgressRecord[]>();
  for (const record of records) {
    const existing = recordsByRun.get(record.event.runId) ?? [];
    existing.push(record);
    recordsByRun.set(record.event.runId, existing);
  }

  const runs = [...recordsByRun.entries()]
    .map(([runId, runRecords]) => buildWorkflowRunView(runId, runRecords, workflowsById))
    .sort(compareWorkflowRuns);
  const selectedRun = selectWorkflowRun(runs);
  return {
    selectedRun,
    runs,
    availableWorkflows,
    timeline: recentWorkflowTimeline(records),
  };
}

export function collectWorkflowProgressRecords(thread: Thread | null): WorkflowProgressRecord[] {
  if (!thread) {
    return [];
  }
  const records: WorkflowProgressRecord[] = [];
  let order = 0;
  for (const turn of thread.turns) {
    for (const item of turn.items) {
      if (item.type !== "workflowRunProgress") {
        continue;
      }
      records.push({
        itemId: item.id,
        event: item.event,
        order,
      });
      order += 1;
    }
  }
  return records.sort(compareWorkflowProgressRecords);
}

function buildWorkflowRunView(
  runId: string,
  records: WorkflowProgressRecord[],
  workflowsById: Map<string, WorkflowSummary>,
): WorkflowRunView {
  const sortedRecords = [...records].sort(compareWorkflowProgressRecords);
  const latestRecord = sortedRecords[sortedRecords.length - 1];
  const latestEvent = latestRecord.event;
  const workflow = workflowsById.get(latestEvent.workflowId);
  const timeline = recentWorkflowTimeline(sortedRecords);
  return {
    runId,
    workflowId: latestEvent.workflowId,
    workflowName: workflow?.name || latestEvent.workflowId,
    source: workflow?.source ?? "unknown",
    kind: latestEvent.kind,
    statusTone: statusToneForKind(latestEvent.kind),
    statusLabel: formatWorkflowKind(latestEvent.kind),
    runnerStatus: latestEvent.runnerStatus,
    message: latestEvent.message,
    updatedAt: latestEvent.updatedAt,
    stages: workflowStagesForRun(latestEvent),
    graphSource: latestEvent.workflowId === "feature-dev" ? "fallback" : "missing",
    graphNote:
      latestEvent.workflowId === "feature-dev"
        ? "Using built-in feature-dev stage fallback; graph metadata is not in this update."
        : "Graph metadata is unavailable for this workflow.",
    timeline,
  };
}

function selectWorkflowRun(runs: WorkflowRunView[]) {
  return runs.find((run) => run.statusTone === "running") ?? runs[0] ?? null;
}

function workflowStagesForRun(event: ThreadWorkflowRunProgressEvent): WorkflowStageView[] {
  const stageTemplates = event.workflowId === "feature-dev" ? FEATURE_DEV_STAGES : [];
  return stageTemplates.map((stage, index) => ({
    ...stage,
    status: stageStatusForRun(event.kind, index),
  }));
}

function stageStatusForRun(
  kind: ThreadWorkflowRunProgressKind,
  index: number,
): WorkflowStageStatus {
  if (kind === "completed") {
    return "completed";
  }
  if (kind === "failed") {
    return index === 0 ? "failed" : "pending";
  }
  if (kind === "aborted") {
    return index === 0 ? "aborted" : "pending";
  }
  return index === 0 ? "current" : "pending";
}

function workflowTimelineItem(record: WorkflowProgressRecord): WorkflowTimelineItem {
  return {
    id: record.itemId,
    runId: record.event.runId,
    workflowId: record.event.workflowId,
    kind: record.event.kind,
    statusTone: statusToneForKind(record.event.kind),
    label: formatWorkflowKind(record.event.kind),
    message: record.event.message,
    runnerStatus: record.event.runnerStatus,
    statusLabel: formatWorkflowStatus(record.event.status),
    updatedAt: record.event.updatedAt,
  };
}

function recentWorkflowTimeline(records: WorkflowProgressRecord[]): WorkflowTimelineItem[] {
  return [...records]
    .sort(compareWorkflowProgressRecordsDescending)
    .slice(0, WORKFLOW_TIMELINE_LIMIT)
    .map((record) => workflowTimelineItem(record));
}

function statusToneForKind(kind: ThreadWorkflowRunProgressKind): WorkflowStatusTone {
  switch (kind) {
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

function formatWorkflowKind(kind: ThreadWorkflowRunProgressKind) {
  switch (kind) {
    case "started":
      return "Started";
    case "resumed":
      return "Resumed";
    case "completed":
      return "Completed";
    case "failed":
      return "Failed";
    case "aborted":
      return "Aborted";
    default:
      return "Unknown";
  }
}

export function formatWorkflowStatus(status: unknown): string {
  if (typeof status === "string") {
    return status;
  }
  if (status && typeof status === "object") {
    const keys = Object.keys(status);
    if (keys.length === 1) {
      const value = (status as Record<string, unknown>)[keys[0]];
      return typeof value === "string" && value.trim()
        ? `${keys[0]}: ${value}`
        : keys[0];
    }
  }
  return "unknown";
}

export function formatWorkflowTimestamp(updatedAt: number): string {
  if (!Number.isFinite(updatedAt) || updatedAt <= 0) {
    return "time unavailable";
  }
  return new Date(updatedAt * 1000).toLocaleString();
}

function compareWorkflowProgressRecords(
  left: WorkflowProgressRecord,
  right: WorkflowProgressRecord,
) {
  if (left.event.updatedAt !== right.event.updatedAt) {
    return left.event.updatedAt - right.event.updatedAt;
  }
  return left.order - right.order;
}

function compareWorkflowProgressRecordsDescending(
  left: WorkflowProgressRecord,
  right: WorkflowProgressRecord,
) {
  return compareWorkflowProgressRecords(right, left);
}

function compareWorkflowRuns(left: WorkflowRunView, right: WorkflowRunView) {
  if (left.updatedAt !== right.updatedAt) {
    return right.updatedAt - left.updatedAt;
  }
  return left.runId.localeCompare(right.runId);
}
