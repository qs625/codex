const crypto = require("node:crypto");
const fsConstants = require("node:fs").constants;
const path = require("node:path");

function createRuntimeLaunchReadiness({
  createId = crypto.randomUUID,
  env = process.env,
  fs,
  now = Date.now,
  onReady,
} = {}) {
  const readyPath = normalizeString(env.MORPHEUS_LAUNCHER_READY_PATH);
  const transactionId = normalizeString(env.MORPHEUS_LAUNCH_TRANSACTION_ID);
  const buildId = normalizeString(env.MORPHEUS_LAUNCH_BUILD_ID);
  let appServerReady = false;
  let rendererReady = false;
  let completion = null;

  const completeIfReady = () => {
    if (!appServerReady || !rendererReady) {
      return Promise.resolve({ written: false });
    }
    if (!completion) {
      completion = completeReadiness({
        buildId,
        createId,
        fs,
        now,
        onReady,
        readyPath,
        transactionId,
      });
    }
    return completion;
  };

  return {
    markAppServerReady() {
      appServerReady = true;
      return completeIfReady();
    },
    markRendererReady() {
      rendererReady = true;
      return completeIfReady();
    },
  };
}

async function completeReadiness({
  buildId,
  createId,
  fs,
  now,
  onReady,
  readyPath,
  transactionId,
}) {
  const payload =
    readyPath && transactionId && buildId
      ? {
          transactionId,
          buildId,
          readyAtMs: now(),
        }
      : null;
  if (payload) {
    await writeJsonAtomically(readyPath, payload, fs, createId);
  }
  await onReady?.(payload);
  return { written: Boolean(payload), payload };
}

async function writeJsonAtomically(targetPath, payload, fs, createId) {
  await fs.mkdir(path.dirname(targetPath), { recursive: true });
  const temporaryPath = `${targetPath}.${process.pid}.${createId()}.tmp`;
  const flags =
    fsConstants.O_WRONLY |
    fsConstants.O_CREAT |
    fsConstants.O_EXCL |
    (fsConstants.O_NOFOLLOW ?? 0);
  let handle = null;
  let created = false;
  try {
    handle = await fs.open(temporaryPath, flags, 0o600);
    created = true;
    await handle.writeFile(`${JSON.stringify(payload)}\n`, "utf8");
    await handle.sync();
    await handle.close();
    handle = null;
    await fs.rename(temporaryPath, targetPath);
    created = false;
  } catch (error) {
    try {
      await handle?.close();
    } catch {}
    if (created) {
      try {
        await fs.rm(temporaryPath, { force: true });
      } catch {}
    }
    throw error;
  }
}

async function readLauncherFailureEvidence(
  evidencePath = process.env.MORPHEUS_LAUNCHER_FAILURE_EVIDENCE,
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
  const recoveryId =
    normalizeString(evidence?.recoveryIdentity) ??
    normalizeString(evidence?.recoveryId);
  if (!recoveryId) {
    throw new Error("Launcher failure evidence is missing recovery identity");
  }
  const transactionId =
    normalizeString(evidence?.transactionId) ??
    normalizeString(evidence?.failed?.transactionId);
  const buildId =
    normalizeString(evidence?.buildId) ??
    normalizeString(evidence?.failedBuildId) ??
    normalizeString(evidence?.failed?.buildId);
  const mode = normalizeString(evidence?.mode);
  const occurredAt = resolveOccurredAt(evidence);
  if (!transactionId || !buildId || (mode !== "full" && mode !== "hot")) {
    throw new Error(
      "Launcher failure evidence is missing transaction, build, or activation mode",
    );
  }
  const previousBuildId =
    normalizeString(evidence?.previousBuildId) ??
    normalizeString(evidence?.recoveredBuildId) ??
    normalizeString(evidence?.fallback?.buildId);
  return {
    recoveryId,
    transactionId,
    mode,
    buildId,
    reason:
      normalizeString(evidence?.reason) ??
      normalizeString(evidence?.summary) ??
      "Runtime activation failed",
    occurredAt,
    ...(previousBuildId ? { previousBuildId } : {}),
  };
}

async function recordLauncherRecovery({
  appServerClient,
  evidence,
  listResult,
  runtimeLauncher,
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
  await runtimeLauncher.ackFailure(recoveryParams.recoveryId);
  return response.recorded;
}

async function recordLauncherRecoveryIfPresent({
  appServerClient,
  evidencePath,
  fs,
  listThreads,
  logger = console,
  runtimeLauncher,
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
      runtimeLauncher,
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
  createRuntimeLaunchReadiness,
  readLauncherFailureEvidence,
  recordLauncherRecovery,
  recordLauncherRecoveryIfPresent,
};
