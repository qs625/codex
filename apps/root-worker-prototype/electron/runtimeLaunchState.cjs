const crypto = require("node:crypto");
const fsConstants = require("node:fs").constants;
const path = require("node:path");

const RUNTIME_CAPSULE_READY_PROTOCOL = "1";
const RUNTIME_CAPSULE_READY_PROTOCOL_VERSION = 1;

function createRuntimeLaunchReadiness({
  createId = crypto.randomUUID,
  env = process.env,
  fs,
  onReady,
  pid = process.pid,
} = {}) {
  const protocol = normalizeString(env.RUNTIME_CAPSULE_READY_PROTOCOL);
  const readyPath = normalizeString(env.RUNTIME_CAPSULE_READY_PATH);
  const token = normalizeString(env.RUNTIME_CAPSULE_READY_TOKEN);
  const releaseId = normalizeString(env.RUNTIME_CAPSULE_RELEASE_ID);
  const launchInstanceId = normalizeString(
    env.RUNTIME_CAPSULE_LAUNCH_INSTANCE_ID,
  );
  const spawnAttemptId = normalizeString(
    env.RUNTIME_CAPSULE_SPAWN_ATTEMPT_ID,
  );
  const startIdentity = normalizeUnsignedIntegerString(
    env.RUNTIME_CAPSULE_START_IDENTITY,
  );
  let appServerReady = false;
  let rendererReady = false;
  let completion = null;

  const completeIfReady = () => {
    if (!appServerReady || !rendererReady) {
      return Promise.resolve({ written: false });
    }
    if (!completion) {
      completion = completeReadiness({
        createId,
        fs,
        launchInstanceId,
        onReady,
        pid,
        protocol,
        readyPath,
        releaseId,
        spawnAttemptId,
        startIdentity,
        token,
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
  createId,
  fs,
  launchInstanceId,
  onReady,
  pid,
  protocol,
  readyPath,
  releaseId,
  spawnAttemptId,
  startIdentity,
  token,
}) {
  const payload =
    protocol === RUNTIME_CAPSULE_READY_PROTOCOL &&
    readyPath &&
    token &&
    releaseId &&
    launchInstanceId &&
    spawnAttemptId &&
    Number.isSafeInteger(pid) &&
    pid > 0 &&
    startIdentity !== null
      ? {
          protocolVersion: RUNTIME_CAPSULE_READY_PROTOCOL_VERSION,
          releaseId,
          launchInstanceId,
          spawnAttemptId,
          pid,
          startIdentity,
          token,
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
  await runtimeLauncher.ackFailure(recoveryParams.activationId);
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

function normalizeUnsignedIntegerString(value) {
  const normalized = normalizeString(value);
  if (!normalized || !/^(0|[1-9][0-9]*)$/.test(normalized)) {
    return null;
  }
  const parsed = Number(normalized);
  return Number.isSafeInteger(parsed) ? parsed : null;
}

module.exports = {
  buildLauncherRecoveryRecordParams,
  createRuntimeLaunchReadiness,
  readLauncherFailureEvidence,
  recordLauncherRecovery,
  recordLauncherRecoveryIfPresent,
};
