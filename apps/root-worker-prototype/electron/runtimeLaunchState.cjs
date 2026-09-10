async function readLauncherFailureEvidence(
  evidencePath = process.env.RUNTIME_CAPSULE_FAILURE_EVIDENCE_PATH,
  fs,
) {
  if (!normalizeString(evidencePath)) {
    return null;
  }
  try {
    const parsed = JSON.parse(await fs.readFile(evidencePath, "utf8"));
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      throw new Error("Launcher failure evidence has an invalid payload");
    }
    return parsed;
  } catch (error) {
    if (error?.code === "ENOENT") {
      return null;
    }
    throw error;
  }
}

function buildLauncherRecoveryRecordParams(evidence) {
  const activationId = normalizeString(evidence?.activationId);
  const releaseId = normalizeString(evidence?.releaseId);
  const occurredAt = resolveOccurredAt(evidence);
  if (!activationId || !releaseId) {
    throw new Error(
      "Launcher failure evidence is missing activation or release identity",
    );
  }
  const recoveryId =
    normalizeString(evidence?.recoveryId) ?? `${activationId}:${releaseId}`;
  const fallbackReleaseId = normalizeString(evidence?.fallbackReleaseId);
  return {
    recoveryId,
    activationId,
    releaseId,
    reason:
      normalizeString(evidence?.reason) ??
      normalizeString(evidence?.message) ??
      normalizeString(evidence?.summary) ??
      "Runtime activation failed",
    occurredAt,
    ...(fallbackReleaseId ? { fallbackReleaseId } : {}),
  };
}

async function recordLauncherRecovery({
  appServerClient,
  evidence,
  listResult,
  subscribeThread,
}) {
  const selfProjectThreadId = normalizeString(listResult?.selfProjectThreadId);
  if (!selfProjectThreadId) {
    throw new Error("System /self thread is unavailable");
  }
  const recoveryParams = buildLauncherRecoveryRecordParams(evidence);
  if (listResult?.materializedSelfThreadId !== selfProjectThreadId) {
    await subscribeThread(selfProjectThreadId);
  }
  const response = await appServerClient.request(
    "thread/clientRecovery/record",
    {
      threadId: selfProjectThreadId,
      ...recoveryParams,
    },
  );
  if (typeof response?.recorded !== "boolean") {
    throw new Error("thread/clientRecovery/record returned an invalid response");
  }
  return response.recorded;
}

async function recordLauncherRecoveryIfPresent({
  appServerClient,
  evidencePath,
  fs,
  listThreads,
  logger = console,
  subscribeThread,
}) {
  try {
    const evidence = await readLauncherFailureEvidence(evidencePath, fs);
    if (!evidence) {
      return { recorded: false, evidence: false };
    }
    const listResult = await listThreads();
    const recorded = await recordLauncherRecovery({
      appServerClient,
      evidence,
      listResult,
      subscribeThread,
    });
    return { recorded, evidence: true };
  } catch (error) {
    try {
      logger.warn?.(
        "[prototype] launcher recovery recording failed; evidence retained",
        JSON.stringify({
          reason: error instanceof Error ? error.message : String(error),
        }),
      );
    } catch {}
    return {
      recorded: false,
      evidence: Boolean(normalizeString(evidencePath)),
    };
  }
}

function resolveOccurredAt(evidence) {
  const explicit = normalizeString(evidence?.occurredAt);
  if (explicit) {
    const timestamp = Date.parse(explicit);
    if (!Number.isFinite(timestamp)) {
      throw new Error("Launcher failure evidence has an invalid occurredAt");
    }
    return new Date(timestamp).toISOString();
  }
  const observedAtUnixMs = evidence?.observedAtUnixMs;
  if (
    typeof observedAtUnixMs !== "number" ||
    !Number.isFinite(observedAtUnixMs) ||
    observedAtUnixMs < 0
  ) {
    throw new Error("Launcher failure evidence is missing occurrence time");
  }
  return new Date(observedAtUnixMs).toISOString();
}

function normalizeString(value) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

module.exports = {
  buildLauncherRecoveryRecordParams,
  readLauncherFailureEvidence,
  recordLauncherRecovery,
  recordLauncherRecoveryIfPresent,
};
