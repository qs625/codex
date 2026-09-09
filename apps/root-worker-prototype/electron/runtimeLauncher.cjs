"use strict";

const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

function createRuntimeLauncher({
  env = process.env,
  spawnSync: spawn = spawnSync,
} = {}) {
  const launcherPath = resolveRuntimeLauncherPath({ env });
  const stateRoot = resolveRuntimeLauncherStateRoot(env);
  const invocationOptions = { env, spawnSync: spawn };
  const invoke = (args) =>
    invokeLauncher(launcherPath, ["--state-root", stateRoot, ...args], {
      ...invocationOptions,
    });
  const invokeMutation = (command, activationId, reason) => {
    const control = currentControl(invoke(["status"]));
    return invokeLauncherRequest(
      launcherPath,
      stateRoot,
      command,
      {
        activationId: requiredString(activationId),
        expectedRevision: requiredInteger(control.revision, "revision"),
        expectedExecutorEpoch: requiredInteger(
          control.executorEpoch,
          "executorEpoch",
        ),
        reason: normalizeString(reason) ?? "",
      },
      invocationOptions,
    );
  };
  return {
    supported: Boolean(launcherPath),
    launcherPath,
    stateRoot,
    prepareActivation: ({ activationId, manifest, reason, releaseId }) => {
      const control = currentControl(invoke(["status"]));
      const response = invokeLauncherRequest(
        launcherPath,
        stateRoot,
        "prepare-activation",
        {
          schemaVersion: 1,
          activationId: requiredString(activationId),
          releaseId: requiredString(releaseId),
          expectedRevision: requiredInteger(control.revision, "revision"),
          expectedExecutorEpoch: requiredInteger(
            control.executorEpoch,
            "executorEpoch",
          ),
          target: manifest?.target,
          reason: normalizeString(reason) ?? "",
        },
        invocationOptions,
      );
      return normalizePrepareActivationResult(response, {
        activationId,
        releaseId,
      });
    },
    cancelActivation: (activationId, reason = null) =>
      invokeMutation("cancel-activation", activationId, reason),
    requestRollback: (activationId, reason = null) =>
      invokeMutation("request-rollback", activationId, reason),
    ackFailure: (activationId, reason = null) =>
      invokeMutation("ack-failure", activationId, reason),
    status: () => invoke(["status"]),
  };
}

function currentControl(statusResult) {
  const result = statusResult?.result ?? statusResult;
  const control = result?.control ?? result?.state ?? result;
  if (!control || typeof control !== "object") {
    throw new Error("Runtime launcher status did not include control state");
  }
  return control;
}

function normalizePrepareActivationResult(
  response,
  { activationId, releaseId },
) {
  const result = response?.result ?? response;
  if (!result || typeof result !== "object") {
    throw new Error("Runtime launcher prepare did not return a typed result");
  }
  if (
    result.activationId !== requiredString(activationId) ||
    result.releaseId !== requiredString(releaseId)
  ) {
    throw new Error(
      "Runtime launcher prepare result does not match the requested activation",
    );
  }
  if (
    ![
      "prepared",
      "already_prepared",
      "already_committed",
      "terminal_failed",
    ].includes(result.disposition)
  ) {
    throw new Error(
      `Runtime launcher prepare returned unsupported disposition: ${String(result.disposition)}`,
    );
  }
  if (!result.control || typeof result.control !== "object") {
    throw new Error("Runtime launcher prepare result did not include control state");
  }
  return result;
}

function resolveRuntimeLauncherPath({
  env = process.env,
} = {}) {
  if (normalizeString(env.RUNTIME_CAPSULE_LAUNCHER_PATH)) {
    return path.resolve(env.RUNTIME_CAPSULE_LAUNCHER_PATH);
  }
  if (normalizeString(env.MORPHEUS_LAUNCHER_PATH)) {
    return path.resolve(env.MORPHEUS_LAUNCHER_PATH);
  }
  return null;
}

function resolveRuntimeLauncherStateRoot(env = process.env) {
  const configured =
    normalizeString(env.RUNTIME_CAPSULE_LAUNCHER_HOME) ??
    normalizeString(env.MORPHEUS_RUNTIME_LAUNCHER_HOME);
  if (configured) {
    return path.resolve(configured);
  }
  const morpheusHome =
    normalizeString(env.MORPHEUS_HOME) ??
    (normalizeString(env.HOME)
      ? path.join(path.resolve(env.HOME), ".morpheus")
      : null);
  return morpheusHome
    ? path.join(morpheusHome, "runtime-launcher")
    : path.join(os.tmpdir(), "morpheus-runtime-launcher");
}

function invokeLauncherRequest(
  launcherPath,
  stateRoot,
  command,
  request,
  options = {},
) {
  assertLauncherAvailable(launcherPath);
  const temporaryRoot = (options.mkdtempSync ?? fs.mkdtempSync)(
    path.join(os.tmpdir(), "runtime-capsule-launch-request-"),
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
      ["--state-root", stateRoot, command, "--request", requestPath],
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
  const stdout = String(result?.stdout ?? "").trim();
  let parsed = null;
  try {
    parsed = stdout ? JSON.parse(stdout) : null;
  } catch (error) {
    throw new Error(`Runtime launcher returned invalid JSON: ${error.message}`);
  }
  if (result?.status !== 0 || parsed?.ok !== true) {
    const message =
      normalizeString(parsed?.error?.message) ??
      normalizeString(parsed?.error) ??
      normalizeString(parsed?.reason) ??
      normalizeString(result?.stderr) ??
      `exit status ${String(result?.status)}`;
    const error = new Error(message);
    error.name = "RuntimeLauncherError";
    error.launcherResult = parsed;
    throw error;
  }
  return parsed;
}

function assertLauncherAvailable(launcherPath) {
  if (!launcherPath) {
    throw new Error("Runtime Capsule launcher is unavailable");
  }
}

function requiredString(value) {
  if (typeof value !== "string" || !value.trim()) {
    throw new Error("Runtime launcher argument must be a non-empty string");
  }
  return value;
}

function requiredInteger(value, label) {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new Error(`Runtime launcher status has invalid ${label}`);
  }
  return value;
}

function normalizeString(value) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

module.exports = {
  createRuntimeLauncher,
  currentControl,
  invokeLauncher,
  invokeLauncherRequest,
  normalizePrepareActivationResult,
  resolveRuntimeLauncherPath,
  resolveRuntimeLauncherStateRoot,
};
