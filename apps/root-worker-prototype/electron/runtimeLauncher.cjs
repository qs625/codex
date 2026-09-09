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
  const invoke = (args) =>
    invokeLauncher(launcherPath, ["--state-root", stateRoot, ...args], {
      env,
      spawnSync: spawn,
    });
  return {
    supported: Boolean(launcherPath),
    launcherPath,
    stateRoot,
    prepareFull: (request) =>
      invokeLauncherRequest(launcherPath, stateRoot, "prepare-full", request, {
        env,
        spawnSync: spawn,
      }),
    abortFull: (transactionId) =>
      invoke(["abort-full", "--transaction", requiredString(transactionId)]),
    activateHot: (request) =>
      invokeLauncherRequest(launcherPath, stateRoot, "activate-hot", request, {
        env,
        spawnSync: spawn,
      }),
    commitHot: (transactionId) =>
      invoke(["commit-hot", "--transaction", requiredString(transactionId)]),
    rollbackHot: (transactionId) =>
      invoke(["rollback-hot", "--transaction", requiredString(transactionId)]),
    ackFailure: (recoveryIdentity) =>
      invoke([
        "ack-failure",
        "--recovery-identity",
        requiredString(recoveryIdentity),
      ]),
    status: () => invoke(["status"]),
  };
}

function resolveRuntimeLauncherPath({
  env = process.env,
  platform = process.platform,
  resourcesPath = currentResourcesPath(),
} = {}) {
  if (normalizeString(env.MORPHEUS_LAUNCHER_PATH)) {
    return path.resolve(env.MORPHEUS_LAUNCHER_PATH);
  }
  if (platform !== "darwin" || !resourcesPath) {
    return null;
  }
  return path.join(path.dirname(resourcesPath), "MacOS", LAUNCHER_FILE_NAME);
}

function resolveRuntimeLauncherStateRoot(env = process.env) {
  if (normalizeString(env.MORPHEUS_RUNTIME_LAUNCHER_HOME)) {
    return path.resolve(env.MORPHEUS_RUNTIME_LAUNCHER_HOME);
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
    throw new Error("Stable runtime launcher is unavailable");
  }
}

function requiredString(value) {
  if (typeof value !== "string" || !value.trim()) {
    throw new Error("Runtime launcher argument must be a non-empty string");
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
