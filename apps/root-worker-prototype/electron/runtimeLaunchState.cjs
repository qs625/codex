const path = require("node:path");
const { createHash, randomUUID } = require("node:crypto");

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

async function writePayloadFailureEvidence({
  evidencePath = process.env.RUNTIME_CAPSULE_FAILURE_EVIDENCE_PATH,
  fs,
  releaseId = process.env.RUNTIME_CAPSULE_RELEASE_ID,
  payloadPid = process.pid,
  reason,
  now = Date.now,
  createId = randomUUID,
}) {
  const resolvedEvidencePath = normalizeString(evidencePath);
  const resolvedReleaseId = normalizeString(releaseId);
  const resolvedReason = normalizeString(reason);
  if (!resolvedEvidencePath || !resolvedReleaseId || !resolvedReason) {
    return null;
  }
  const timestamp = now();
  if (!Number.isFinite(timestamp) || timestamp < 0) {
    throw new Error("Payload failure evidence requires a valid timestamp");
  }
  const evidence = {
    activationId: `payload-${payloadPid}-${Math.trunc(timestamp)}-${createId()}`,
    releaseId: resolvedReleaseId,
    occurredAt: new Date(timestamp).toISOString(),
    code: "payload_reported_error",
    message: resolvedReason,
    details: {
      payloadPid: String(payloadPid),
      source: "payload",
    },
  };
  const parentDirectory = path.dirname(resolvedEvidencePath);
  const temporaryPath = path.join(
    parentDirectory,
    `.${path.basename(resolvedEvidencePath)}.${createId()}.tmp`,
  );
  await fs.mkdir(parentDirectory, { recursive: true });
  try {
    await fs.writeFile(
      temporaryPath,
      `${JSON.stringify(evidence)}\n`,
      { encoding: "utf8", mode: 0o600 },
    );
    await fs.rename(temporaryPath, resolvedEvidencePath);
  } catch (error) {
    await fs.rm(temporaryPath, { force: true }).catch(() => {});
    throw error;
  }
  return evidence;
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

async function recoverPayloadRuntimeFailureIfPresent({
  evidencePath,
  formatPayloadRuntimeRecoveryPrompt,
  fs,
  sendSelfCommand,
}) {
  let evidence;
  try {
    evidence = await readLauncherFailureEvidence(evidencePath, fs);
  } catch (error) {
    error.payloadEvidence = false;
    throw error;
  }
  if (!isPayloadRuntimeRecoveryEvidence(evidence)) {
    return {
      evidence: Boolean(evidence),
      payloadEvidence: false,
      recovered: false,
    };
  }
  try {
    if (typeof formatPayloadRuntimeRecoveryPrompt !== "function") {
      throw new Error("Payload recovery prompt formatter is unavailable");
    }
    if (typeof sendSelfCommand !== "function") {
      throw new Error("Payload recovery requires the /self input path");
    }
    const delivery = await claimPayloadRecoveryDelivery({
      evidence,
      evidencePath,
      fs,
    });
    if (delivery.inFlight) {
      return {
        delivery: "in_flight",
        evidence: true,
        payloadEvidence: true,
        recovered: false,
      };
    }
    if (delivery.alreadyDelivered) {
      await consumePayloadRecoveryEvidence({
        evidence,
        evidencePath,
        fs,
        markerPath: delivery.markerPath,
      });
      return {
        delivery: "already_sent",
        evidence: true,
        payloadEvidence: true,
        recovered: true,
      };
    }
    const text = formatPayloadRuntimeRecoveryPrompt({
      failedReleaseId: evidence.releaseId,
      reason: normalizeString(evidence.message),
      exitCode: numericDetail(evidence.details?.exitCode),
      signal: normalizeString(evidence.details?.signal),
    });
    if (!normalizeString(text)) {
      throw new Error("Payload recovery prompt formatter returned no input");
    }
    try {
      await sendSelfCommand(text);
    } catch (error) {
      await releasePayloadRecoveryDelivery(delivery, fs);
      throw error;
    }
    await markPayloadRecoveryDeliveryDelivered(delivery, fs);
    await consumePayloadRecoveryEvidence({
      evidence,
      evidencePath,
      fs,
      markerPath: delivery.markerPath,
    });
    return {
      delivery: "sent",
      evidence: true,
      payloadEvidence: true,
      recovered: true,
    };
  } catch (error) {
    error.payloadEvidence = true;
    throw error;
  }
}

async function claimPayloadRecoveryDelivery({ evidence, evidencePath, fs }) {
  const eventId = payloadRecoveryEventId(evidence);
  const markerPath = `${evidencePath}.${createHash("sha256")
    .update(eventId)
    .digest("hex")}.self-delivery`;
  const existing = await readPayloadRecoveryDeliveryMarker(markerPath, fs);
  if (existing) {
    if (existing.eventId !== eventId) {
      throw new Error("Payload recovery delivery marker has the wrong event");
    }
    return {
      alreadyDelivered: existing.state === "delivered",
      inFlight: existing.state === "claimed",
      markerPath,
    };
  }
  const claimId = randomUUID();
  try {
    await fs.writeFile(
      markerPath,
      `${JSON.stringify({ claimId, eventId, state: "claimed" })}\n`,
      { encoding: "utf8", flag: "wx", mode: 0o600 },
    );
    return { claimId, eventId, markerPath };
  } catch (error) {
    if (error?.code === "EEXIST") {
      return claimPayloadRecoveryDelivery({ evidence, evidencePath, fs });
    }
    throw error;
  }
}

async function markPayloadRecoveryDeliveryDelivered(delivery, fs) {
  const marker = await readPayloadRecoveryDeliveryMarker(delivery.markerPath, fs);
  if (
    marker?.eventId !== delivery.eventId ||
    marker?.claimId !== delivery.claimId ||
    marker?.state !== "claimed"
  ) {
    return false;
  }
  await writePayloadRecoveryDeliveryMarker(
    delivery.markerPath,
    {
      claimId: delivery.claimId,
      eventId: delivery.eventId,
      state: "delivered",
    },
    fs,
  );
  return true;
}

async function releasePayloadRecoveryDelivery(delivery, fs) {
  const marker = await readPayloadRecoveryDeliveryMarker(delivery.markerPath, fs);
  if (
    marker?.eventId === delivery.eventId &&
    marker?.claimId === delivery.claimId &&
    marker?.state === "claimed"
  ) {
    await fs.rm(delivery.markerPath, { force: true }).catch(() => {});
  }
}

async function consumePayloadRecoveryEvidence({
  evidence,
  evidencePath,
  fs,
  markerPath,
}) {
  const current = await readLauncherFailureEvidence(evidencePath, fs);
  if (payloadRecoveryEventId(current) !== payloadRecoveryEventId(evidence)) {
    return false;
  }
  await fs.rm(evidencePath, { force: true });
  await fs.rm(markerPath, { force: true }).catch(() => {});
  return true;
}

async function readPayloadRecoveryDeliveryMarker(markerPath, fs) {
  try {
    const parsed = JSON.parse(await fs.readFile(markerPath, "utf8"));
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      throw new Error("Payload recovery delivery marker has an invalid payload");
    }
    if (
      !normalizeString(parsed.eventId) ||
      !normalizeString(parsed.state) ||
      (parsed.state !== "claimed" && parsed.state !== "delivered")
    ) {
      throw new Error("Payload recovery delivery marker has invalid fields");
    }
    return parsed;
  } catch (error) {
    if (error?.code === "ENOENT") {
      return null;
    }
    throw error;
  }
}

async function writePayloadRecoveryDeliveryMarker(markerPath, marker, fs) {
  const temporaryPath = `${markerPath}.${randomUUID()}.tmp`;
  try {
    await fs.writeFile(
      temporaryPath,
      `${JSON.stringify(marker)}\n`,
      { encoding: "utf8", mode: 0o600 },
    );
    await fs.rename(temporaryPath, markerPath);
  } catch (error) {
    await fs.rm(temporaryPath, { force: true }).catch(() => {});
    throw error;
  }
}

function payloadRecoveryEventId(evidence) {
  const activationId = normalizeString(evidence?.activationId);
  const releaseId = normalizeString(evidence?.releaseId);
  return activationId && releaseId ? `${activationId}:${releaseId}` : null;
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

async function recoverLauncherStateAtStartup({
  logger = console,
  recordLauncherRecovery,
  recoverPayloadFailure,
}) {
  const recorded = await recordLauncherRecovery();
  let payloadRecovery = null;
  let hasDurableRestartRecovery = false;
  try {
    payloadRecovery = await recoverPayloadFailure();
    hasDurableRestartRecovery =
      payloadRecovery?.payloadEvidence === true;
  } catch (error) {
    hasDurableRestartRecovery = error?.payloadEvidence === true;
    logger.error?.(
      "[prototype] payload recovery input failed; evidence retained",
      error,
    );
  }
  return { hasDurableRestartRecovery, payloadRecovery, recorded };
}

function isPayloadRuntimeRecoveryEvidence(evidence) {
  const code = normalizeString(evidence?.code);
  return Boolean(
    normalizeString(evidence?.activationId) &&
      normalizeString(evidence?.releaseId) &&
      normalizeString(evidence?.fallbackReleaseId) &&
      (code === "payload_reported_error" ||
        code === "payload_exit_code" ||
        code === "payload_exit_signal" ||
        code === "payload_spawn_or_load_error"),
  );
}

function numericDetail(value) {
  if (typeof value !== "string" || !/^-?\d+$/.test(value)) {
    return null;
  }
  const numeric = Number(value);
  return Number.isSafeInteger(numeric) ? numeric : null;
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
  recoverLauncherStateAtStartup,
  recoverPayloadRuntimeFailureIfPresent,
  readLauncherFailureEvidence,
  recordLauncherRecovery,
  recordLauncherRecoveryIfPresent,
  writePayloadFailureEvidence,
};
