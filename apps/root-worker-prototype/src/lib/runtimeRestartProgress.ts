import type { BootstrapResponse, NotificationEnvelope } from "../types";

export type RuntimeRestartProgressStatus =
  | "active"
  | "completed"
  | "failed"
  | "recovered";

export type RuntimeRestartProgress = {
  status: RuntimeRestartProgressStatus;
  requestId: string | null;
  originThreadId: string | null;
  stage: string;
  stageLabel: string;
  message: string;
  reason: string | null;
  activationId: string | null;
  releaseId: string | null;
  updatedAtMs: number;
};

export function runtimeRestartProgressFromStatus(
  status: NotificationEnvelope["status"],
  previous: RuntimeRestartProgress | null = null,
  now = Date.now,
): RuntimeRestartProgress | null {
  const runtimeRestart = status?.runtimeRestart;
  if (runtimeRestart) {
    return progressFromRuntimeRestart(runtimeRestart, previous, now);
  }

  const lifecycle = status?.lifecycle;
  if (
    lifecycle?.type !== "installedArtifactUpdate" &&
    lifecycle?.type !== "clientRelaunch"
  ) {
    return previous;
  }
  const requestId = stringOrNull(lifecycle.requestId);
  if (!requestId || !previous || previous.requestId !== requestId) {
    return previous;
  }
  return {
    status: statusForStage(lifecycle.phase),
    requestId: previous.requestId,
    originThreadId: previous?.originThreadId ?? null,
    stage: lifecycle.phase,
    stageLabel: stageLabel(lifecycle.phase),
    message: stageMessage(lifecycle.phase, lifecycle.reason),
    reason: stringOrNull(lifecycle.reason),
    activationId: stringOrNull(lifecycle.activationId) ?? previous?.activationId ?? null,
    releaseId: stringOrNull(lifecycle.releaseId) ?? previous?.releaseId ?? null,
    updatedAtMs: now(),
  };
}

export function runtimeRestartProgressFromBootstrap(
  expectedRestart: BootstrapResponse["expectedRestart"] | null | undefined,
  now = Date.now,
): RuntimeRestartProgress | null {
  const requestIds = expectedRestart?.expectedRequestIds ?? [];
  if (requestIds.length === 0) {
    return null;
  }
  const failed = (expectedRestart?.failedThreadIds ?? []).length > 0;
  const recovered = (expectedRestart?.recoveredThreadIds ?? []).length > 0;
  const stage = failed ? "failed" : recovered ? "recovered" : "completed";
  return {
    status: failed ? "failed" : recovered ? "recovered" : "completed",
    requestId: requestIds.join(", "),
    originThreadId: expectedRestart?.expectedThreadIds?.[0] ?? null,
    stage,
    stageLabel: stageLabel(stage),
    message: failed
      ? "Runtime restart recovery still needs attention."
      : recovered
        ? "Runtime restart recovered after the new capsule started."
        : "Runtime restart handoff was already recorded.",
    reason: null,
    activationId: null,
    releaseId: null,
    updatedAtMs: now(),
  };
}

function progressFromRuntimeRestart(
  restart: NonNullable<NotificationEnvelope["status"]>["runtimeRestart"],
  previous: RuntimeRestartProgress | null,
  now: () => number,
): RuntimeRestartProgress | null {
  const requestId = stringOrNull(restart?.requestId);
  if (!requestId) {
    return previous;
  }
  const stage = stringOrNull(restart?.phase) ?? "received";
  const reason = stringOrNull(restart?.reason);
  return {
    status: statusForStage(stage),
    requestId,
    originThreadId: stringOrNull(restart?.requestedByThreadId),
    stage,
    stageLabel: stageLabel(stage),
    message: stageMessage(stage, reason),
    reason,
    activationId: previous?.activationId ?? null,
    releaseId: previous?.releaseId ?? null,
    updatedAtMs: Number.isFinite(restart?.updatedAtMs)
      ? Number(restart?.updatedAtMs)
      : now(),
  };
}

function statusForStage(stage: string): RuntimeRestartProgressStatus {
  switch (stage) {
    case "completed":
    case "consumed":
      return "completed";
    case "failed":
      return "failed";
    case "recovered":
      return "recovered";
    default:
      return "active";
  }
}

function stageLabel(stage: string) {
  switch (stage) {
    case "received":
      return "Request received";
    case "executing":
      return "Restart accepted";
    case "preparing":
      return "Preparing Runtime Capsule";
    case "selected":
      return "Candidate selected";
    case "shuttingDownAppServer":
      return "Stopping app-server";
    case "exiting":
      return "Switching capsule";
    case "completed":
      return "Completed";
    case "recovered":
      return "Recovered";
    case "failed":
      return "Failed";
    default:
      return stage || "Restarting";
  }
}

function stageMessage(stage: string, reason: string | null | undefined) {
  const suffix = reason ? ` ${reason}` : "";
  switch (stage) {
    case "received":
      return `Runtime restart request is queued.${suffix}`;
    case "executing":
      return `Runtime restart is being executed.${suffix}`;
    case "preparing":
      return `Building or preparing the candidate Runtime Capsule.${suffix}`;
    case "selected":
      return `The candidate Runtime Capsule has been selected.${suffix}`;
    case "shuttingDownAppServer":
      return `Stopping the current app-server before switching capsules.${suffix}`;
    case "exiting":
      return `The client is exiting so the launcher can start the selected capsule.${suffix}`;
    case "completed":
      return `Runtime restart completed.${suffix}`;
    case "failed":
      return reason || "Runtime restart failed.";
    case "recovered":
      return "Runtime restart recovery completed.";
    default:
      return reason || "Runtime restart is in progress.";
  }
}

function stringOrNull(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}
