const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const LAUNCHER_FILE_NAME = "MorpheusLauncher";

function createRuntimeLauncher({
  env = process.env,
  platform = process.platform,
  resourcesPath = currentResourcesPath(),
  spawnSync: spawn = spawnSync,
} = {}) {
  const launcherPath = resolveRuntimeLauncherPath({
    env,
    platform,
    resourcesPath,
  });
  const stateRoot = resolveRuntimeLauncherStateRoot(env);
  const stateRootArgs = ["--state-root", stateRoot];
  return {
    supported: Boolean(launcherPath),
    launcherPath,
    prepareFull: (request) =>
      invokeLauncherRequest(launcherPath, "prepare-full", request, {
        env,
        spawnSync: spawn,
      }),
    abortFull: (transactionId) =>
      invokeLauncher(launcherPath, [
        "abort-full",
        "--transaction",
        requireString(transactionId, "transactionId"),
      ], { env, spawnSync: spawn }),
    activateHot: (request) =>
      invokeLauncherRequest(launcherPath, "activate-hot", request, {
        env,
        spawnSync: spawn,
      }),
    commitHot: (transactionId) =>
      invokeLauncher(launcherPath, [
        "commit-hot",
        "--transaction",
        requireString(transactionId, "transactionId"),
      ], { env, spawnSync: spawn }),
    rollbackHot: (transactionId, appBundlePath) => {
      const rollback = invokeLauncher(launcherPath, [
        "rollback-hot",
        "--transaction",
        requireString(transactionId, "transactionId"),
      ], { env, spawnSync: spawn });
      const status = appBundlePath
        ? invokeLauncher(launcherPath, [
            ...stateRootArgs,
            "status",
            "--app-bundle",
            requireString(appBundlePath, "appBundlePath"),
          ], { env, spawnSync: spawn })
        : null;
      const failureEvidence = status?.result?.failureEvidence
        ? {
            ...status.result.failureEvidence,
            recoveredBuildId:
              status?.result?.state?.current?.buildId ??
              status.result.failureEvidence.recoveredBuildId,
          }
        : null;
      return {
        ...rollback,
        failureEvidence,
        evidencePath: failureEvidence
          ? path.join(stateRoot, "failure-evidence.json")
          : null,
      };
    },
    claimFailure: ({
      recoveryIdentity,
      expectedVersion,
      transactionId = null,
      requestId = null,
    }) => {
      const requestedVersion = requireVersionToken(
        expectedVersion,
        "expectedVersion",
      );
      const result = invokeLauncher(
        launcherPath,
        [
          ...stateRootArgs,
          "claim-failure",
          "--recovery-identity",
          requireCanonicalUuid(recoveryIdentity, "recoveryIdentity"),
          "--expected-version",
          requestedVersion,
          ...optionalLauncherArgument(
            "--transaction",
            transactionId,
            "transactionId",
          ),
          ...optionalLauncherArgument("--request", requestId, "requestId"),
        ],
        { env, spawnSync: spawn },
      );
      return validateFailureClaimResult(result, requestedVersion);
    },
    finalizeFailureClaim: ({ claimId, recoveryIdentity }) => {
      const expectedClaimId = requireCanonicalUuid(claimId, "claimId");
      const expectedRecoveryIdentity = requireCanonicalUuid(
        recoveryIdentity,
        "recoveryIdentity",
      );
      const result = invokeLauncher(
        launcherPath,
        [
          ...stateRootArgs,
          "finalize-failure-claim",
          "--claim-id",
          expectedClaimId,
          "--recovery-identity",
          expectedRecoveryIdentity,
        ],
        { env, spawnSync: spawn },
      );
      return validateFailureFinalizeResult(result, {
        claimId: expectedClaimId,
        recoveryIdentity: expectedRecoveryIdentity,
      });
    },
    status: (appBundlePath = null, recoveryIdentity = null) => {
      const hasRecoveryIdentity =
        recoveryIdentity !== null && recoveryIdentity !== undefined;
      const status = invokeLauncher(
        launcherPath,
        [
          ...stateRootArgs,
          "status",
          ...(appBundlePath
            ? ["--app-bundle", requireString(appBundlePath, "appBundlePath")]
            : []),
          ...(hasRecoveryIdentity
            ? [
                "--recovery-identity",
                requireCanonicalUuid(recoveryIdentity, "recoveryIdentity"),
              ]
            : []),
        ],
        { env, spawnSync: spawn },
      );
      if (
        hasRecoveryIdentity &&
        (status?.ok !== true ||
          !status.result ||
          typeof status.result !== "object" ||
          Array.isArray(status.result))
      ) {
        throw new Error(
          "Runtime launcher identity status did not return a successful result",
        );
      }
      const failureEvidence = status?.result?.failureEvidence
        ? {
            ...status.result.failureEvidence,
            recoveredBuildId:
              status?.result?.state?.current?.buildId ??
              status.result.failureEvidence.recoveredBuildId,
          }
        : null;
      return {
        ...status,
        result: status?.result
          ? {
              ...status.result,
              failureEvidence,
              ...(hasRecoveryIdentity
                ? {
                    failureEvidenceMatch: validateFailureEvidenceMatch(
                      status.result.failureEvidenceMatch,
                      recoveryIdentity,
                    ),
                  }
                : {}),
            }
          : status?.result,
        evidencePath: failureEvidence
          ? path.join(stateRoot, "failure-evidence.json")
          : null,
      };
    },
  };
}

function resolveRuntimeLauncherStateRoot(env = process.env) {
  if (env.MORPHEUS_RUNTIME_LAUNCHER_HOME) {
    return path.resolve(env.MORPHEUS_RUNTIME_LAUNCHER_HOME);
  }
  const morpheusHome =
    env.MORPHEUS_HOME ??
    (env.HOME ? path.join(env.HOME, ".morpheus") : null);
  return morpheusHome
    ? path.join(path.resolve(morpheusHome), "runtime-launcher")
    : path.join(os.tmpdir(), "morpheus-runtime-launcher-unresolved");
}

function resolveRuntimeLauncherPath({
  env = process.env,
  platform = process.platform,
  resourcesPath = currentResourcesPath(),
} = {}) {
  if (env.MORPHEUS_LAUNCHER_PATH) {
    return path.resolve(env.MORPHEUS_LAUNCHER_PATH);
  }
  if (platform !== "darwin" || !resourcesPath) {
    return null;
  }
  return path.join(
    path.dirname(resourcesPath),
    "MacOS",
    LAUNCHER_FILE_NAME,
  );
}

function invokeLauncherRequest(launcherPath, command, request, options = {}) {
  assertLauncherAvailable(launcherPath);
  const temporaryRoot = (options.mkdtempSync ?? fs.mkdtempSync)(
    path.join(os.tmpdir(), "morpheus-launch-request-"),
  );
  const requestPath = path.join(temporaryRoot, "request.json");
  try {
    (options.writeFileSync ?? fs.writeFileSync)(
      requestPath,
      `${JSON.stringify(request, null, 2)}\n`,
      { encoding: "utf8", mode: 0o600 },
    );
    return invokeLauncher(
      launcherPath,
      [command, "--request", requestPath],
      options,
    );
  } finally {
    (options.rmSync ?? fs.rmSync)(temporaryRoot, {
      force: true,
      recursive: true,
    });
  }
}

function invokeLauncher(launcherPath, args, options = {}) {
  assertLauncherAvailable(launcherPath);
  const result = (options.spawnSync ?? spawnSync)(launcherPath, args, {
    encoding: "utf8",
    env: options.env ?? process.env,
    stdio: "pipe",
  });
  if (result?.error) {
    throw result.error;
  }
  const output = String(result?.stdout ?? "").trim();
  const launcherResult = output ? parseLauncherResult(output) : null;
  if (result?.status !== 0) {
    throw createLauncherError({
      args,
      launcherResult,
      output,
      result,
    });
  }
  if (!output) {
    throw new Error(`Runtime launcher ${args[0]} returned no JSON result`);
  }
  if (!launcherResult) {
    throw new Error(`Runtime launcher ${args[0]} returned invalid JSON`, {
      cause: parseLauncherResult.lastError,
    });
  }
  return launcherResult;
}

function parseLauncherResult(output) {
  try {
    parseLauncherResult.lastError = null;
    return JSON.parse(output);
  } catch (error) {
    parseLauncherResult.lastError = error;
    return null;
  }
}

function createLauncherError({ args, launcherResult, output, result }) {
  const stderr = String(result?.stderr ?? "").trim();
  const launcherMessage =
    normalizeString(launcherResult?.error) ??
    normalizeString(launcherResult?.reason) ??
    normalizeString(stderr) ??
    `exit status ${String(result?.status)}`;
  const error = new Error(
    `Runtime launcher ${args[0]} exited with ${String(result?.status)}: ${launcherMessage}`,
  );
  error.name = "RuntimeLauncherError";
  error.command = args[0];
  error.exitCode = result?.status ?? null;
  error.stderr = stderr;
  error.stdout = output;
  error.launcherResult = launcherResult;
  error.rolledBack =
    launcherResult?.rolledBack === true ||
    launcherResult?.result?.rolledBack === true;
  error.failureEvidence =
    launcherResult?.failureEvidence ??
    launcherResult?.result?.failureEvidence ??
    null;
  error.evidencePath =
    normalizeString(launcherResult?.evidencePath) ??
    normalizeString(launcherResult?.result?.evidencePath);
  error.errorCode = normalizeString(launcherResult?.errorCode);
  error.expectedVersion = normalizeString(launcherResult?.expectedVersion);
  error.actualVersion = normalizeString(launcherResult?.actualVersion);
  return error;
}

function assertLauncherAvailable(launcherPath) {
  if (!launcherPath) {
    throw new Error(
      "Runtime launcher is unavailable; Windows and Linux activation adapters are not implemented.",
    );
  }
}

function requireString(value, label) {
  if (typeof value !== "string" || !value.trim()) {
    throw new Error(`Runtime launcher ${label} is required`);
  }
  return value.trim();
}

function requireCanonicalUuid(value, label) {
  const normalized = requireString(value, label);
  if (
    !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(
      normalized,
    )
  ) {
    throw new Error(`Runtime launcher ${label} must be a canonical UUID`);
  }
  return normalized;
}

function requireVersionToken(value, label) {
  const normalized = requireString(value, label);
  if (!/^sha256:[0-9a-f]{64}$/.test(normalized)) {
    throw new Error(
      `Runtime launcher ${label} must be a canonical sha256 token`,
    );
  }
  return normalized;
}

function optionalLauncherArgument(flag, value, label) {
  if (value === null || value === undefined) {
    return [];
  }
  if (typeof value !== "string" || !value.trim()) {
    throw new Error(`Runtime launcher ${label} must be a non-empty string`);
  }
  return [flag, value];
}

function validateFailureEvidenceMatch(match, expectedRecoveryIdentity) {
  if (!match || typeof match !== "object" || Array.isArray(match)) {
    throw new Error(
      "Runtime launcher status did not return failureEvidenceMatch",
    );
  }
  const recoveryIdentity = requireCanonicalUuid(
    match.recoveryIdentity,
    "failureEvidenceMatch.recoveryIdentity",
  );
  if (recoveryIdentity !== expectedRecoveryIdentity) {
    throw new Error(
      "Runtime launcher status returned a different recovery identity",
    );
  }
  if (
    !["current", "pending", "claimed", "consumed"].includes(
      match.artifactState,
    )
  ) {
    throw new Error(
      "Runtime launcher status returned an invalid failure artifact state",
    );
  }
  const transactionId = optionalRawString(
    match.transactionId,
    "failureEvidenceMatch.transactionId",
  );
  const requestId = optionalRawString(
    match.requestId,
    "failureEvidenceMatch.requestId",
  );
  const evidence = requirePlainObject(
    match.evidence,
    "failureEvidenceMatch.evidence",
  );
  validateEvidenceProvenance(evidence, {
    recoveryIdentity,
    transactionId,
    requestId,
  });
  if (match.artifactState === "claimed") {
    requireCanonicalUuid(
      evidence.claimId,
      "failureEvidenceMatch.evidence.claimId",
    );
  }
  return {
    ...match,
    recoveryIdentity,
    transactionId,
    requestId,
    configuredEvidencePath: requireString(
      match.configuredEvidencePath,
      "failureEvidenceMatch.configuredEvidencePath",
    ),
    activeEvidencePath: requireString(
      match.activeEvidencePath,
      "failureEvidenceMatch.activeEvidencePath",
    ),
    versionToken: requireVersionToken(
      match.versionToken,
      "failureEvidenceMatch.versionToken",
    ),
    evidence,
  };
}

function validateFailureClaimResult(result, requestedVersion) {
  const claim = requirePlainObject(result, "claim result");
  if (claim.ok !== true) {
    throw new Error("Runtime launcher claim result was not successful");
  }
  const recoveryIdentity = requireCanonicalUuid(
    claim.recoveryIdentity,
    "claim.recoveryIdentity",
  );
  const claimId = requireCanonicalUuid(claim.claimId, "claim.claimId");
  const transactionId = optionalRawString(
    claim.transactionId,
    "claim.transactionId",
  );
  const requestId = optionalRawString(claim.requestId, "claim.requestId");
  if (!["created", "existing"].includes(claim.claimState)) {
    throw new Error("Runtime launcher returned an invalid claimState");
  }
  if (typeof claim.requestedVersionMatched !== "boolean") {
    throw new Error(
      "Runtime launcher claim result omitted requestedVersionMatched",
    );
  }
  const claimVersionToken = requireVersionToken(
    claim.versionToken,
    "claim.versionToken",
  );
  const sourceVersionToken = requireVersionToken(
    claim.sourceVersionToken,
    "claim.sourceVersionToken",
  );
  if (
    claim.requestedVersionMatched &&
    requestedVersion !== claimVersionToken &&
    requestedVersion !== sourceVersionToken
  ) {
    throw new Error(
      "Runtime launcher claim incorrectly reported a requested version match",
    );
  }
  if (
    claim.claimState === "created" &&
    (!claim.requestedVersionMatched ||
      requestedVersion !== sourceVersionToken)
  ) {
    throw new Error(
      "Runtime launcher created a claim for an unexpected evidence version",
    );
  }
  const evidence = requirePlainObject(claim.evidence, "claim.evidence");
  validateEvidenceProvenance(evidence, {
    recoveryIdentity,
    transactionId,
    requestId,
  });
  if (evidence.claimId !== claimId) {
    throw new Error("Runtime launcher claim evidence has a different claimId");
  }
  return {
    ...claim,
    claimId,
    recoveryIdentity,
    transactionId,
    requestId,
    versionToken: claimVersionToken,
    sourceVersionToken,
    configuredEvidencePath: requireString(
      claim.configuredEvidencePath,
      "claim.configuredEvidencePath",
    ),
    activeEvidencePath: requireString(
      claim.activeEvidencePath,
      "claim.activeEvidencePath",
    ),
    evidence,
  };
}

function validateFailureFinalizeResult(
  result,
  { claimId, recoveryIdentity },
) {
  const finalized = requirePlainObject(result, "finalize result");
  if (
    finalized.ok !== true ||
    finalized.acknowledged !== true ||
    finalized.consumed !== true
  ) {
    throw new Error(
      "Runtime launcher did not finalize and consume the failure claim",
    );
  }
  if (
    finalized.claimId !== claimId ||
    finalized.recoveryIdentity !== recoveryIdentity
  ) {
    throw new Error(
      "Runtime launcher finalized a different failure claim",
    );
  }
  if (typeof finalized.remainingEvidence !== "boolean") {
    throw new Error(
      "Runtime launcher finalize result omitted remainingEvidence",
    );
  }
  return {
    ...finalized,
    versionToken: requireVersionToken(
      finalized.versionToken,
      "finalize.versionToken",
    ),
  };
}

function validateEvidenceProvenance(
  evidence,
  { recoveryIdentity, transactionId, requestId },
) {
  if (
    evidence.recoveryIdentity !== recoveryIdentity ||
    optionalRawString(evidence.transactionId, "evidence.transactionId") !==
      transactionId ||
    optionalRawString(evidence.requestId, "evidence.requestId") !== requestId
  ) {
    throw new Error(
      "Runtime launcher failure evidence provenance does not match its metadata",
    );
  }
}

function requirePlainObject(value, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`Runtime launcher ${label} must be an object`);
  }
  return value;
}

function optionalRawString(value, label) {
  if (value === null || value === undefined) {
    return null;
  }
  if (typeof value !== "string" || !value.trim()) {
    throw new Error(`Runtime launcher ${label} must be null or non-empty`);
  }
  return value;
}

function normalizeString(value) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function currentResourcesPath() {
  return typeof process.resourcesPath === "string"
    ? process.resourcesPath
    : null;
}

module.exports = {
  createRuntimeLauncher,
  invokeLauncher,
  invokeLauncherRequest,
  resolveRuntimeLauncherPath,
  resolveRuntimeLauncherStateRoot,
};
