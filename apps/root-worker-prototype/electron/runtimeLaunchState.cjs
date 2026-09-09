const crypto = require("node:crypto");
const fsConstants = require("node:fs").constants;
const path = require("node:path");

const READY_SCHEMA_VERSION = 1;
const RECOVERY_RESTART_EXIT_CODE = 76;

function createRuntimeLaunchReadiness({
  env = process.env,
  fs,
  now = Date.now,
  onReady,
  createId = crypto.randomUUID,
} = {}) {
  const readyPath = normalizeString(env.MORPHEUS_LAUNCH_READY_PATH);
  const identity = {
    transactionId: normalizeString(env.MORPHEUS_LAUNCH_TRANSACTION_ID),
    buildId: normalizeString(env.MORPHEUS_LAUNCH_BUILD_ID),
    instanceId: normalizeString(env.MORPHEUS_LAUNCH_INSTANCE_ID),
  };
  let appServerReady = false;
  let rendererReady = false;
  let completionPromise = null;

  const writeIfReady = () => {
    if (!appServerReady || !rendererReady) {
      return Promise.resolve({ written: false });
    }
    if (!completionPromise) {
      completionPromise = completeReadiness({
        readyPath,
        identity,
        fs,
        now,
        onReady,
        createId,
      });
    }
    return completionPromise;
  };

  return {
    markAppServerReady() {
      appServerReady = true;
      return writeIfReady();
    },
    markRendererReady() {
      rendererReady = true;
      return writeIfReady();
    },
    identity,
    readyPath,
  };
}

async function completeReadiness({
  readyPath,
  identity,
  fs,
  now,
  onReady,
  createId,
}) {
  const hasReadyIdentity =
    readyPath &&
    identity.transactionId &&
    identity.buildId &&
    identity.instanceId;
  const result = hasReadyIdentity
    ? await writeReadyFile(
        readyPath,
        {
          schemaVersion: READY_SCHEMA_VERSION,
          ...identity,
          readyAtMs: now(),
        },
        fs,
        createId,
      )
    : { written: false, payload: null };
  await onReady?.(result.payload);
  return result;
}

async function writeReadyFile(readyPath, payload, fs, createId) {
  const temporaryPath = `${readyPath}.${process.pid}.${createId()}.tmp`;
  await fs.mkdir(path.dirname(readyPath), { recursive: true });
  const openFlags =
    fsConstants.O_WRONLY |
    fsConstants.O_CREAT |
    fsConstants.O_EXCL |
    (fsConstants.O_NOFOLLOW ?? 0);
  let handle = null;
  let created = false;
  try {
    handle = await fs.open(temporaryPath, openFlags, 0o600);
    created = true;
    await handle.writeFile(`${JSON.stringify(payload, null, 2)}\n`, "utf8");
    await handle.sync();
    await handle.close();
    handle = null;
    await fs.rename(temporaryPath, readyPath);
    created = false;
    return { written: true, payload };
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

async function readLauncherFailureEvidence(evidencePath, fs) {
  if (!evidencePath) {
    return null;
  }
  try {
    const snapshot = await readEvidenceSnapshot(evidencePath, fs);
    const parsed = JSON.parse(snapshot.contents);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return null;
    }
    return Object.hasOwn(parsed, "evidence")
      ? parseFailureEvidenceClaimWrapper(parsed, snapshot)
      : withRecoveryIdentity(parsed, snapshot);
  } catch (error) {
    if (error?.code === "ENOENT") {
      return null;
    }
    throw error;
  }
}

async function prepareCanonicalLauncherFailureEvidence({
  initialEvidence,
  runtimeLauncher,
  maxClaimAttempts = 3,
}) {
  const locatorEvidence = withRecoveryIdentity(
    initialEvidence,
    initialEvidence?.evidenceSnapshot,
  );
  const expectedProvenance = launcherFailureEvidenceProvenance(locatorEvidence);
  if (
    !runtimeLauncher?.supported ||
    typeof runtimeLauncher.status !== "function" ||
    typeof runtimeLauncher.claimFailure !== "function"
  ) {
    throw new Error(
      "Runtime launcher claim support is required before recording failure evidence",
    );
  }
  if (!Number.isInteger(maxClaimAttempts) || maxClaimAttempts < 1) {
    throw new Error("maxClaimAttempts must be a positive integer");
  }

  let lastVersionError = null;
  for (let attempt = 0; attempt < maxClaimAttempts; attempt += 1) {
    const status = await runtimeLauncher.status(
      normalizeString(locatorEvidence.appBundlePath),
      expectedProvenance.recoveryIdentity,
    );
    const match = validateFailureEvidenceMatch(
      status?.result?.failureEvidenceMatch,
      expectedProvenance,
    );
    if (match.artifactState === "consumed") {
      throw new Error(
        "Launcher failure evidence was already consumed before typed recording",
      );
    }

    let claim;
    if (match.artifactState === "claimed") {
      claim = claimFromStatusMatch(match);
    } else {
      try {
        claim = await runtimeLauncher.claimFailure({
          recoveryIdentity: expectedProvenance.recoveryIdentity,
          transactionId: expectedProvenance.transactionId,
          requestId: expectedProvenance.requestId,
          expectedVersion: match.versionToken,
        });
      } catch (error) {
        if (
          error?.errorCode === "failure-evidence-version-changed" &&
          attempt + 1 < maxClaimAttempts
        ) {
          lastVersionError = error;
          continue;
        }
        throw error;
      }
      claim = validateFailureClaim(claim, expectedProvenance);
      if (
        claim.claimState === "created" &&
        claim.requestedVersionMatched !== true
      ) {
        throw new Error(
          "Runtime launcher created a claim for an unexpected evidence version",
        );
      }
    }

    return normalizedClaimedFailureEvidence(claim);
  }
  throw (
    lastVersionError ??
    new Error("Runtime launcher failure claim attempts were exhausted")
  );
}

function parseFailureEvidenceClaimWrapper(wrapper, evidenceSnapshot) {
  const allowedFields = new Set([
    "schemaVersion",
    "claimId",
    "recoveryIdentity",
    "launcherOwnerNonce",
    "sourceVersionToken",
    "versionToken",
    "evidence",
  ]);
  for (const field of Object.keys(wrapper)) {
    if (!allowedFields.has(field)) {
      throw new Error(
        `Launcher failure claim wrapper contains unknown field ${field}`,
      );
    }
  }
  if (wrapper.schemaVersion !== 1) {
    throw new Error(
      "Launcher failure claim wrapper requires schemaVersion 1",
    );
  }
  const claimId = canonicalUuid(wrapper.claimId, "claimId");
  const recoveryIdentity = canonicalUuid(
    wrapper.recoveryIdentity,
    "recoveryIdentity",
  );
  const launcherOwnerNonce = requiredString(
    wrapper.launcherOwnerNonce,
    "launcherOwnerNonce",
  );
  const sourceVersionToken = versionToken(
    wrapper.sourceVersionToken,
    "sourceVersionToken",
  );
  const claimVersionToken = versionToken(
    wrapper.versionToken,
    "versionToken",
  );
  if (
    !wrapper.evidence ||
    typeof wrapper.evidence !== "object" ||
    Array.isArray(wrapper.evidence)
  ) {
    throw new Error("Launcher failure claim wrapper requires nested evidence");
  }
  const canonicalEvidence = canonicalLauncherFailureEvidence(wrapper.evidence);
  const evidence = withRecoveryIdentity(canonicalEvidence);
  const provenance = launcherFailureEvidenceProvenance(evidence);
  if (
    provenance.recoveryIdentity !== recoveryIdentity ||
    evidence.claimId !== claimId ||
    evidence.launcherOwnerNonce !== launcherOwnerNonce ||
    evidence.acknowledged === true ||
    failureEvidenceVersionToken(canonicalEvidence) !== claimVersionToken
  ) {
    throw new Error(
      "Launcher failure claim wrapper metadata does not match nested evidence",
    );
  }
  return {
    ...evidence,
    evidenceSnapshot,
    launcherClaim: {
      claimId,
      recoveryIdentity,
      launcherOwnerNonce,
      sourceVersionToken,
      versionToken: claimVersionToken,
    },
  };
}

function canonicalLauncherFailureEvidence(evidence) {
  if (!evidence || typeof evidence !== "object" || Array.isArray(evidence)) {
    throw new Error("Launcher failure evidence must be an object");
  }
  if (evidence.schemaVersion !== 1) {
    throw new Error("Launcher failure evidence requires schemaVersion 1");
  }
  const allowedFields = new Set([
    "schemaVersion",
    "recoveryIdentity",
    "launcherOwnerNonce",
    "occurredAt",
    "transactionId",
    "requestId",
    "requestedByThreadId",
    "mode",
    "buildId",
    "sourceCommit",
    "manifestHash",
    "failedBuildHash",
    "failurePhase",
    "summary",
    "reason",
    "appBundlePath",
    "exitCode",
    "signal",
    "readyTimeoutMs",
    "logPath",
    "transactionPath",
    "recoveredBuildId",
    "claimId",
    "acknowledged",
  ]);
  for (const field of Object.keys(evidence)) {
    if (!allowedFields.has(field)) {
      throw new Error(
        `Launcher failure evidence contains unknown field ${field}`,
      );
    }
  }
  for (const field of allowedFields) {
    if (field !== "claimId" && !Object.hasOwn(evidence, field)) {
      throw new Error(`Launcher failure evidence is missing field ${field}`);
    }
  }
  const canonical = {
    schemaVersion: evidence.schemaVersion,
    recoveryIdentity: evidence.recoveryIdentity,
    launcherOwnerNonce: evidence.launcherOwnerNonce,
    occurredAt: evidence.occurredAt,
    transactionId: evidence.transactionId,
    requestId: evidence.requestId,
    requestedByThreadId: evidence.requestedByThreadId,
    mode: evidence.mode,
    buildId: evidence.buildId,
    sourceCommit: evidence.sourceCommit,
    manifestHash: evidence.manifestHash,
    failedBuildHash: evidence.failedBuildHash,
    failurePhase: evidence.failurePhase,
    summary: evidence.summary,
    reason: evidence.reason,
    appBundlePath: evidence.appBundlePath,
    exitCode: evidence.exitCode,
    signal: evidence.signal,
    readyTimeoutMs: evidence.readyTimeoutMs,
    logPath: evidence.logPath,
    transactionPath: evidence.transactionPath,
    recoveredBuildId: evidence.recoveredBuildId,
    ...(evidence.claimId === null || evidence.claimId === undefined
      ? {}
      : { claimId: evidence.claimId }),
    acknowledged: evidence.acknowledged,
  };
  return canonical;
}

function failureEvidenceVersionToken(evidence) {
  return `sha256:${crypto
    .createHash("sha256")
    .update(JSON.stringify(evidence))
    .digest("hex")}`;
}

function launcherFailureEvidenceProvenance(evidence) {
  const durableEvidence = withRecoveryIdentity(
    evidence,
    evidence?.evidenceSnapshot,
  );
  return {
    recoveryIdentity: durableEvidence.recoveryIdentity,
    transactionId: rawOptionalString(
      durableEvidence.transactionId,
      "transactionId",
    ),
    requestId: rawOptionalString(durableEvidence.requestId, "requestId"),
  };
}

function validateFailureEvidenceMatch(match, expectedProvenance) {
  if (!match || typeof match !== "object" || Array.isArray(match)) {
    throw new Error(
      "Runtime launcher status did not return canonical failure evidence",
    );
  }
  if (
    !["current", "pending", "claimed", "consumed"].includes(
      match.artifactState,
    )
  ) {
    throw new Error("Runtime launcher returned an invalid artifactState");
  }
  const provenance = {
    recoveryIdentity: canonicalUuid(
      match.recoveryIdentity,
      "status recoveryIdentity",
    ),
    transactionId: rawOptionalString(
      match.transactionId,
      "status transactionId",
    ),
    requestId: rawOptionalString(match.requestId, "status requestId"),
  };
  assertSameLauncherProvenance(provenance, expectedProvenance);
  const evidence = withRecoveryIdentity(match.evidence);
  assertSameLauncherProvenance(
    launcherFailureEvidenceProvenance(evidence),
    expectedProvenance,
  );
  return {
    ...match,
    ...provenance,
    configuredEvidencePath: requiredString(
      match.configuredEvidencePath,
      "status configuredEvidencePath",
    ),
    activeEvidencePath: requiredString(
      match.activeEvidencePath,
      "status activeEvidencePath",
    ),
    versionToken: versionToken(match.versionToken, "status versionToken"),
    evidence,
  };
}

function claimFromStatusMatch(match) {
  return {
    ok: true,
    claimId: canonicalUuid(match.evidence.claimId, "claimed evidence claimId"),
    recoveryIdentity: match.recoveryIdentity,
    transactionId: match.transactionId,
    requestId: match.requestId,
    versionToken: match.versionToken,
    sourceVersionToken: null,
    configuredEvidencePath: match.configuredEvidencePath,
    activeEvidencePath: match.activeEvidencePath,
    claimState: "existing",
    requestedVersionMatched: true,
    evidence: match.evidence,
  };
}

function validateFailureClaim(claim, expectedProvenance) {
  if (!claim || typeof claim !== "object" || Array.isArray(claim)) {
    throw new Error("Runtime launcher did not return a failure claim");
  }
  if (!["created", "existing"].includes(claim.claimState)) {
    throw new Error("Runtime launcher returned an invalid claimState");
  }
  if (typeof claim.requestedVersionMatched !== "boolean") {
    throw new Error(
      "Runtime launcher claim omitted requestedVersionMatched",
    );
  }
  const provenance = {
    recoveryIdentity: canonicalUuid(
      claim.recoveryIdentity,
      "claim recoveryIdentity",
    ),
    transactionId: rawOptionalString(
      claim.transactionId,
      "claim transactionId",
    ),
    requestId: rawOptionalString(claim.requestId, "claim requestId"),
  };
  assertSameLauncherProvenance(provenance, expectedProvenance);
  const evidence = withRecoveryIdentity(claim.evidence);
  assertSameLauncherProvenance(
    launcherFailureEvidenceProvenance(evidence),
    expectedProvenance,
  );
  const claimId = canonicalUuid(claim.claimId, "claimId");
  if (evidence.claimId !== claimId) {
    throw new Error(
      "Runtime launcher claim metadata does not match frozen evidence",
    );
  }
  return {
    ...claim,
    ...provenance,
    claimId,
    versionToken: versionToken(claim.versionToken, "claim versionToken"),
    sourceVersionToken: versionToken(
      claim.sourceVersionToken,
      "claim sourceVersionToken",
    ),
    configuredEvidencePath: requiredString(
      claim.configuredEvidencePath,
      "claim configuredEvidencePath",
    ),
    activeEvidencePath: requiredString(
      claim.activeEvidencePath,
      "claim activeEvidencePath",
    ),
    evidence,
  };
}

function normalizedClaimedFailureEvidence(claim) {
  const evidence = normalizeLauncherFailureEvidence(
    claim.evidence,
    claim.configuredEvidencePath,
  );
  return {
    ...evidence,
    launcherClaim: {
      claimId: claim.claimId,
      recoveryIdentity: claim.recoveryIdentity,
      versionToken: claim.versionToken,
      sourceVersionToken: claim.sourceVersionToken,
      activeEvidencePath: claim.activeEvidencePath,
      configuredEvidencePath: claim.configuredEvidencePath,
      claimState: claim.claimState,
    },
  };
}

function assertSameLauncherProvenance(actual, expected) {
  if (
    actual.recoveryIdentity !== expected.recoveryIdentity ||
    actual.transactionId !== expected.transactionId ||
    actual.requestId !== expected.requestId
  ) {
    throw new Error(
      "Canonical launcher failure evidence provenance changed during claim",
    );
  }
}

function normalizeLauncherFailureEvidence(evidence, evidencePath) {
  const durableEvidence = withRecoveryIdentity(
    evidence,
    evidence?.evidenceSnapshot,
  );
  const launcherTransactionId = rawOptionalString(
    durableEvidence.transactionId,
    "transactionId",
  );
  const launcherRequestId = rawOptionalString(
    durableEvidence.requestId,
    "requestId",
  );
  const transactionId = normalizeString(launcherTransactionId);
  const requestId = normalizeString(launcherRequestId);
  const requestedByThreadId = normalizeRequestedByThreadId(durableEvidence);
  const failedBuildId =
    normalizeString(durableEvidence.failedBuildId) ??
    normalizeString(durableEvidence.buildId);
  const failedArtifactHash =
    normalizeString(durableEvidence.failedBuildHash) ??
    normalizeString(durableEvidence.manifestHash);
  return {
    targetThreadId:
      normalizeString(durableEvidence.targetThreadId) ?? requestedByThreadId,
    transactionId:
      transactionId ?? `legacy-transaction-${durableEvidence.recoveryIdentity}`,
    requestId: requestId ?? `legacy-request-${durableEvidence.recoveryIdentity}`,
    launcherTransactionId,
    launcherRequestId,
    recoveryIdentity: durableEvidence.recoveryIdentity,
    evidenceSnapshot: durableEvidence.evidenceSnapshot,
    requestedByThreadId,
    mode: normalizeString(durableEvidence.mode) ?? "unknown",
    failedBuildId: failedBuildId ?? "unknown",
    failedBuildHash: failedArtifactHash ?? "unknown",
    sourceCommit: normalizeString(durableEvidence.sourceCommit) ?? "unknown",
    failurePhase:
      normalizeString(durableEvidence.failurePhase) ??
      normalizeString(durableEvidence.phase) ??
      "unknown",
    exitCode: Number.isInteger(durableEvidence.exitCode)
      ? durableEvidence.exitCode
      : null,
    signal: normalizeString(durableEvidence.signal),
    readyTimeoutMs: Number.isFinite(durableEvidence.readyTimeoutMs)
      ? durableEvidence.readyTimeoutMs
      : null,
    logPath: normalizeString(durableEvidence.logPath),
    transactionPath: normalizeString(durableEvidence.transactionPath),
    recoveredBuildId:
      normalizeString(durableEvidence.recoveredBuildId) ?? "unknown",
    appBundlePath: normalizeString(durableEvidence.appBundlePath),
    evidencePath,
    prompt: buildLauncherFailurePrompt({
      ...durableEvidence,
      failedBuildId,
      failedBuildHash: failedArtifactHash,
    }),
  };
}

function normalizeRequestedByThreadId(evidence) {
  if (
    !Object.hasOwn(evidence, "requestedByThreadId") ||
    evidence.requestedByThreadId === null
  ) {
    return null;
  }
  const requestedByThreadId = normalizeString(evidence.requestedByThreadId);
  if (!requestedByThreadId) {
    throw new Error(
      "requestedByThreadId must be null/omitted for legacy evidence or a non-empty thread UUID",
    );
  }
  return requestedByThreadId;
}

function withRecoveryIdentity(evidence, evidenceSnapshot) {
  const recoveryIdentity = evidence?.recoveryIdentity;
  if (
    typeof recoveryIdentity !== "string" ||
    !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(
      recoveryIdentity,
    )
  ) {
    throw new Error(
      "Launcher failure evidence requires a valid persistent recoveryIdentity UUID",
    );
  }
  return { ...evidence, recoveryIdentity, evidenceSnapshot };
}

async function readEvidenceSnapshot(evidencePath, fs) {
  const openFlags =
    fsConstants.O_RDONLY | (fsConstants.O_NOFOLLOW ?? 0);
  let handle = null;
  let primaryError = null;
  try {
    handle = await fs.open(evidencePath, openFlags);
    const before = await handle.stat();
    assertRegularEvidenceFile(before);
    const contents = await handle.readFile("utf8");
    const after = await handle.stat();
    assertSameEvidenceEntry(before, after);
    return {
      contents,
      contentHash: crypto.createHash("sha256").update(contents).digest("hex"),
      dev: before.dev,
      ino: before.ino,
      size: before.size,
      mtimeMs: before.mtimeMs,
      ctimeMs: before.ctimeMs,
    };
  } catch (error) {
    primaryError = error;
    throw error;
  } finally {
    if (handle) {
      try {
        await handle.close();
      } catch (error) {
        if (!primaryError) {
          throw error;
        }
      }
    }
  }
}

function assertRegularEvidenceFile(stat) {
  if (!stat.isFile()) {
    throw new Error("Launcher failure evidence must be a regular file");
  }
}

function assertSameEvidenceEntry(expected, actual) {
  if (
    expected.dev !== actual.dev ||
    expected.ino !== actual.ino ||
    expected.size !== actual.size ||
    expected.mtimeMs !== actual.mtimeMs ||
    expected.ctimeMs !== actual.ctimeMs
  ) {
    throw new Error("Launcher failure evidence changed while being read");
  }
}

function buildLauncherFailurePrompt(evidence) {
  const failedBuild =
    normalizeString(evidence.failedBuildId) ??
    normalizeString(evidence.buildId) ??
    "unknown";
  const artifactHash =
    normalizeString(evidence.failedBuildHash) ??
    normalizeString(evidence.manifestHash) ??
    "unknown";
  return [
    `Morpheus runtime activation ${evidence.transactionId ?? "unknown"} failed during ${evidence.failurePhase ?? evidence.phase ?? "unknown phase"} and the launcher recovered build ${evidence.recoveredBuildId ?? "unknown"}.`,
    `Failed build=${failedBuild}, source=${evidence.sourceCommit ?? "unknown"}, mode=${evidence.mode ?? "unknown"}, exit=${evidence.exitCode ?? "unknown"}, signal=${evidence.signal ?? "none"}, timeoutMs=${evidence.readyTimeoutMs ?? "unknown"}, log=${evidence.logPath ?? "unknown"}, transaction=${evidence.transactionPath ?? "unknown"}.`,
    `Launcher evidence: ${evidence.summary ?? evidence.reason ?? "no additional summary"}.`,
    `Do not activate the same failed build ${failedBuild} or artifact hash ${artifactHash} again.`,
    "Inspect the durable failure evidence and fix the underlying runtime update before requesting another activation.",
  ].join(" ");
}

function normalizeString(value) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function rawOptionalString(value, label) {
  if (value === null || value === undefined) {
    return null;
  }
  if (typeof value !== "string" || !value.trim()) {
    throw new Error(`${label} must be null/omitted or a non-empty string`);
  }
  return value;
}

async function completeLauncherFailureRecovery({
  evidence,
  consumeRestartIntent,
  finalizeFailureClaim,
  requestQueueDrainRestart,
}) {
  const claim = evidence.launcherClaim;
  if (!claim) {
    throw new Error("Launcher failure recovery requires a durable claim");
  }
  if (
    evidence.launcherRequestId !== null &&
    evidence.launcherRequestId !== undefined
  ) {
    await consumeRestartIntent(evidence.requestId);
  }
  const acknowledgement = await finalizeFailureClaim({
    claimId: claim.claimId,
    recoveryIdentity: evidence.recoveryIdentity,
  });
  if (
    acknowledgement?.acknowledged !== true ||
    acknowledgement?.consumed !== true ||
    acknowledgement?.claimId !== claim.claimId ||
    acknowledgement?.versionToken !== claim.versionToken ||
    acknowledgement?.recoveryIdentity !== evidence.recoveryIdentity ||
    acknowledgement?.transactionId !== evidence.launcherTransactionId ||
    acknowledgement?.requestId !== evidence.launcherRequestId
  ) {
    throw new Error(
      "Runtime launcher did not acknowledge and consume failure evidence",
    );
  }
  if (typeof acknowledgement.remainingEvidence !== "boolean") {
    throw new Error(
      "Runtime launcher acknowledgement did not report remainingEvidence",
    );
  }

  if (acknowledgement.remainingEvidence) {
    requestQueueDrainRestart(evidence.recoveryIdentity);
  }

  return {
    consumed: true,
    consumedEvidencePath: acknowledgement.consumedEvidencePath ?? null,
    remainingEvidence: acknowledgement.remainingEvidence,
  };
}

async function recordClaimedLauncherFailureRecovery({
  evidence,
  recordRecovery,
  consumeRestartIntent,
  finalizeFailureClaim,
  requestQueueDrainRestart,
}) {
  const recorded = await recordRecovery(evidence);
  if (!recorded?.accepted) {
    throw new Error("App-server did not acknowledge launcher recovery evidence");
  }
  return completeLauncherFailureRecovery({
    evidence,
    consumeRestartIntent,
    finalizeFailureClaim,
    requestQueueDrainRestart,
  });
}

function canonicalUuid(value, label) {
  if (
    typeof value !== "string" ||
    !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(
      value,
    )
  ) {
    throw new Error(`${label} must be a canonical UUID`);
  }
  return value;
}

function versionToken(value, label) {
  if (
    typeof value !== "string" ||
    !/^sha256:[0-9a-f]{64}$/.test(value)
  ) {
    throw new Error(`${label} must be a canonical sha256 token`);
  }
  return value;
}

function requiredString(value, label) {
  if (typeof value !== "string" || !value.trim()) {
    throw new Error(`${label} must be a non-empty string`);
  }
  return value;
}

function createLauncherQueueDrainRestartRequester({
  appExit,
  exitCode = RECOVERY_RESTART_EXIT_CODE,
}) {
  const requestedIdentities = new Set();
  return (recoveryIdentity) => {
    if (requestedIdentities.has(recoveryIdentity)) {
      return false;
    }
    appExit(exitCode);
    requestedIdentities.add(recoveryIdentity);
    return true;
  };
}

function buildLauncherRecoveryRecordParams(evidence, targetThreadId) {
  return {
    targetThreadId,
    recoveryIdentity: evidence.recoveryIdentity,
    launcherClaimId: evidence.launcherClaim.claimId,
    launcherEvidenceVersion: evidence.launcherClaim.versionToken,
    transactionId: evidence.transactionId,
    requestId: evidence.requestId,
    failedBuildId: evidence.failedBuildId,
    failedBuildHash: evidence.failedBuildHash,
    sourceCommit: evidence.sourceCommit,
    requestedByThreadId: evidence.requestedByThreadId,
    mode: evidence.mode,
    failurePhase: evidence.failurePhase,
    exitCode: evidence.exitCode,
    signal: evidence.signal,
    readyTimeoutMs: evidence.readyTimeoutMs,
    logPath: evidence.logPath,
    transactionPath: evidence.transactionPath,
    recoveredBuildId: evidence.recoveredBuildId,
    prompt: evidence.prompt,
    evidencePath: evidence.evidencePath,
  };
}

module.exports = {
  buildLauncherRecoveryRecordParams,
  buildLauncherFailurePrompt,
  completeLauncherFailureRecovery,
  createLauncherQueueDrainRestartRequester,
  createRuntimeLaunchReadiness,
  normalizeLauncherFailureEvidence,
  prepareCanonicalLauncherFailureEvidence,
  readLauncherFailureEvidence,
  recordClaimedLauncherFailureRecovery,
};
