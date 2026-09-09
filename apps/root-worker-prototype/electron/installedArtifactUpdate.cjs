"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { Worker } = require("node:worker_threads");
const { buildDesktopEnvironment } = require("./environment.cjs");
const { createRuntimeCapsule } = require("./runtimeCapsule.cjs");
const {
  isPackagedApp,
  resolveDefaultWorkspace,
} = require("./workspace.cjs");

const APP_NAME = "Root Worker Runtime";
const SOURCE_APP_RELATIVE_PATH = path.join("apps", "root-worker-prototype");
const PAYLOAD_RELATIVE_PATH = path.join("payload", `${APP_NAME}.app`);
const PAYLOAD_EXECUTABLE_RELATIVE_PATH = path.join(
  PAYLOAD_RELATIVE_PATH,
  "Contents",
  "MacOS",
  APP_NAME,
);
const GENERATED_SOURCE_DIR_NAMES = [
  "dist-app",
  "dist-package-resources",
  "dist-capsule-payload",
  "dist-seed-capsule",
];
const WORKER_FILES = [
  "environment.cjs",
  "installedArtifactUpdate.cjs",
  "installedArtifactUpdateWorker.cjs",
  "runtimeCapsule.cjs",
  "workspace.cjs",
];
let workerBundlePath = null;

function resolveInstalledArtifactFileSystem(options = {}) {
  if (options.fsOps != null) {
    return options.fsOps;
  }
  const defaultFsOps = options.defaultFsOps ?? fs;
  const isElectron =
    options.isElectron ?? typeof process.versions?.electron === "string";
  if (!isElectron) {
    return defaultFsOps;
  }
  const loadOriginalFileSystem =
    options.loadOriginalFileSystem ?? (() => require("original-fs"));
  let originalFileSystem;
  try {
    originalFileSystem = loadOriginalFileSystem();
  } catch (error) {
    throw new Error(
      `Failed to load Electron original-fs for Runtime Capsule filesystem operations: ${error instanceof Error ? error.message : String(error)}`,
      { cause: error },
    );
  }
  if (!originalFileSystem || typeof originalFileSystem !== "object") {
    throw new Error(
      "Electron original-fs did not provide a Runtime Capsule filesystem implementation",
    );
  }
  return originalFileSystem;
}

function resolveInstalledArtifactUpdatePlan(options = {}) {
  const {
    commandEnv,
    env = process.env,
    platform = process.platform,
    spawnSync: spawn = spawnSync,
    workspace,
  } = options;
  const resourcesPath =
    options.resourcesPath === undefined
      ? currentResourcesPath()
      : options.resourcesPath;
  const isPackaged =
    options.isPackaged === undefined
      ? isPackagedApp({ resourcesPath })
      : options.isPackaged;
  if (platform !== "darwin" || !isPackaged || !resourcesPath) {
    return null;
  }
  const resolvedWorkspace =
    workspace ??
    resolveDefaultWorkspace(env, {
      isPackagedApp: true,
      sourceOnly: true,
    });
  if (!resolvedWorkspace) {
    return {
      disabled: true,
      reason:
        "Runtime candidate production is unavailable because the Morpheus source workspace is absent",
    };
  }
  const resolvedCommandEnv = commandEnv ?? buildDesktopEnvironment(env);
  const codexRsDir = path.join(resolvedWorkspace, "codex-rs");
  const sourceAppDir = path.join(resolvedWorkspace, SOURCE_APP_RELATIVE_PATH);
  const cargoTargetDir = resolveCargoTargetDirectory({
    codexRsDir,
    env: resolvedCommandEnv,
    spawnSync: spawn,
  });
  return {
    appServerBinaryPath: path.join(cargoTargetDir, "release", "app-server"),
    codexRsDir,
    commandEnv: resolvedCommandEnv,
    defaultCompactPromptSourcePath: path.join(
      codexRsDir,
      "thread-service",
      "templates",
      "compact",
      "prompt.md",
    ),
    frontendDistPath: path.join(sourceAppDir, "dist"),
    sourceAppDir,
    stateRoot: resolveRuntimeLauncherStateRoot(env),
    workspace: resolvedWorkspace,
  };
}

function resolveCargoTargetDirectory({
  codexRsDir,
  env = buildDesktopEnvironment(),
  spawnSync: spawn = spawnSync,
} = {}) {
  const configured = normalizeString(env.CARGO_TARGET_DIR);
  if (configured) {
    return path.resolve(codexRsDir, configured);
  }
  const result = spawn(
    "cargo",
    ["metadata", "--format-version=1", "--no-deps"],
    {
      cwd: codexRsDir,
      encoding: "utf8",
      env,
      stdio: "pipe",
    },
  );
  assertSuccessfulSpawn(result, "cargo metadata");
  let metadata;
  try {
    metadata = JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(`Failed to parse cargo metadata: ${error.message}`);
  }
  if (!normalizeString(metadata?.target_directory)) {
    throw new Error("Cargo metadata did not include target_directory");
  }
  return metadata.target_directory;
}

function updateInstalledArtifacts(plan, options = {}) {
  const fsOps = resolveInstalledArtifactFileSystem(options);
  assertPlan(plan, fsOps);
  const activationId =
    normalizeString(options.activationId) ?? crypto.randomUUID();
  validateActivationId(activationId);
  const incomingParent = path.join(plan.stateRoot, "incoming");
  const incomingRoot = path.join(incomingParent, activationId);
  const buildRoot = path.join(incomingRoot, ".build");
  const resourceRoot = path.join(incomingRoot, ".resources");
  const payloadPath = path.join(
    incomingRoot,
    ...PAYLOAD_RELATIVE_PATH.split(path.sep),
  );
  const runCommand =
    options.runCommand ??
    ((command, args, commandOptions) =>
      run(command, args, {
        ...commandOptions,
        env: plan.commandEnv,
        spawnSync: options.spawnSync,
      }));
  fsOps.mkdirSync(incomingParent, { recursive: true, mode: 0o755 });
  fsOps.mkdirSync(incomingRoot, { recursive: false, mode: 0o755 });
  try {
    buildRuntimeSources(plan, { runCommand });
    stagePayloadResources(plan, resourceRoot, fsOps);
    runCommand(
      "pnpm",
      [
        "dlx",
        "@electron/packager",
        ".",
        APP_NAME,
        "--platform=darwin",
        "--arch=arm64",
        `--out=${buildRoot}`,
        "--overwrite",
        "--app-bundle-id=com.openai.root-worker-prototype.runtime.dev",
        "--app-category-type=public.app-category.developer-tools",
        "--extend-info=electron/PayloadInfo.plist",
        "--asar",
        ...GENERATED_SOURCE_DIR_NAMES.map(
          (name) => `--ignore=^/${name}($|/)`,
        ),
        "--no-prune",
        `--extra-resource=${path.join(resourceRoot, "bin")}`,
        `--extra-resource=${path.join(resourceRoot, "default-config")}`,
      ],
      { cwd: plan.sourceAppDir },
    );
    const packagedPayload = path.join(
      buildRoot,
      `${APP_NAME}-darwin-arm64`,
      `${APP_NAME}.app`,
    );
    fsOps.mkdirSync(path.dirname(payloadPath), {
      recursive: true,
      mode: 0o755,
    });
    fsOps.renameSync(packagedPayload, payloadPath);
    fsOps.rmSync(buildRoot, { force: true, recursive: true });
    fsOps.rmSync(resourceRoot, { force: true, recursive: true });
    normalizeRuntimeCapsuleTree(incomingRoot, fsOps);
    clearExtendedAttributes(incomingRoot, { runCommand });
    runCommand(
      "codesign",
      ["--force", "--deep", "--sign", "-", payloadPath],
      { cwd: plan.workspace },
    );
    runCommand(
      "codesign",
      ["--verify", "--deep", "--strict", payloadPath],
      { cwd: plan.workspace },
    );
    const sourceCommit =
      normalizeString(options.sourceCommit) ??
      capture("git", ["rev-parse", "HEAD"], {
        cwd: plan.workspace,
        env: plan.commandEnv,
        spawnSync: options.spawnSync,
      }).trim();
    const manifest = createRuntimeCapsule(incomingRoot, {
      arch: "arm64",
      executable: PAYLOAD_EXECUTABLE_RELATIVE_PATH.split(path.sep).join("/"),
      metadata: { sourceCommit },
      os: "darwin",
      readinessTimeoutMs: options.readinessTimeoutMs ?? 30_000,
      fsOps,
    });
    return {
      ok: true,
      activationId,
      incomingRoot,
      releaseId: manifest.releaseId,
      sourceCommit,
      manifest,
    };
  } catch (error) {
    let failure = error;
    try {
      fsOps.rmSync(incomingRoot, { force: true, recursive: true });
    } catch (cleanupError) {
      failure = attachCleanupFailure(error, cleanupError);
    }
    throw failure;
  }
}

function buildRuntimeSources(plan, { runCommand }) {
  runCommand("pnpm", ["build"], { cwd: plan.sourceAppDir });
  runCommand(
    "cargo",
    [
      "build",
      "--manifest-path",
      path.join(plan.codexRsDir, "Cargo.toml"),
      "-p",
      "app-server",
      "--bin",
      "app-server",
      "--release",
    ],
    { cwd: plan.workspace },
  );
}

function stagePayloadResources(
  plan,
  resourceRoot,
  fsOps = resolveInstalledArtifactFileSystem(),
) {
  const appServerTarget = path.join(resourceRoot, "bin", "app-server");
  const compactTarget = path.join(
    resourceRoot,
    "default-config",
    "compact",
    "COMPACT.md",
  );
  fsOps.mkdirSync(path.dirname(appServerTarget), {
    recursive: true,
    mode: 0o755,
  });
  fsOps.mkdirSync(path.dirname(compactTarget), {
    recursive: true,
    mode: 0o755,
  });
  fsOps.copyFileSync(plan.appServerBinaryPath, appServerTarget);
  fsOps.chmodSync(appServerTarget, 0o755);
  fsOps.copyFileSync(plan.defaultCompactPromptSourcePath, compactTarget);
  fsOps.chmodSync(compactTarget, 0o644);
}

function normalizeRuntimeCapsuleTree(
  root,
  fsOps = resolveInstalledArtifactFileSystem(),
) {
  const metadata = fsOps.lstatSync(root);
  if (!metadata.isDirectory()) {
    throw new Error(`Runtime Capsule root is not a directory: ${root}`);
  }
  fsOps.chmodSync(root, 0o755);
  for (const child of fsOps.readdirSync(root, { withFileTypes: true })) {
    const childPath = path.join(root, child.name);
    const childMetadata = fsOps.lstatSync(childPath);
    if (childMetadata.isSymbolicLink()) {
      continue;
    }
    if (childMetadata.isDirectory()) {
      normalizeRuntimeCapsuleTree(childPath, fsOps);
      continue;
    }
    if (!childMetadata.isFile()) {
      throw new Error(`Unsupported Runtime Capsule entry: ${childPath}`);
    }
    if (childMetadata.nlink !== 1) {
      throw new Error(`Runtime Capsule file must not be a hard link: ${childPath}`);
    }
    fsOps.chmodSync(childPath, (childMetadata.mode & 0o111) !== 0 ? 0o755 : 0o644);
  }
}

function clearExtendedAttributes(root, { runCommand }) {
  runCommand("xattr", ["-cr", root], { cwd: path.dirname(root) });
}

function assertPlan(plan, fsOps = resolveInstalledArtifactFileSystem()) {
  for (const [label, targetPath, type] of [
    ["source app", plan?.sourceAppDir, "directory"],
    ["codex-rs", plan?.codexRsDir, "directory"],
    ["runtime launcher state root", plan?.stateRoot, "path"],
  ]) {
    if (typeof targetPath !== "string" || !targetPath) {
      throw new Error(`Missing ${label} path`);
    }
    if (type === "path") {
      continue;
    }
    const metadata = fsOps.statSync(targetPath);
    if (type === "directory" && !metadata.isDirectory()) {
      throw new Error(`Expected ${label} to be a directory: ${targetPath}`);
    }
  }
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

function resolveInstalledArtifactUpdatePlanInWorker(options = {}) {
  const env = options.env ?? { ...process.env };
  const commandEnv = {
    ...(options.commandEnv ??
      buildDesktopEnvironment(env, options.desktopEnvironmentOptions)),
  };
  return runInstalledArtifactWorker(
    "resolvePlan",
    {
      commandEnv,
      env,
      platform: options.platform ?? process.platform,
      resourcesPath: options.resourcesPath ?? currentResourcesPath(),
      workspace: options.workspace,
      ...(Object.hasOwn(options, "isPackaged")
        ? { isPackaged: options.isPackaged }
        : {}),
    },
    options.workerOptions,
  );
}

function updateInstalledArtifactsInWorker(plan, options = {}) {
  return runInstalledArtifactWorker(
    "update",
    { plan },
    options.workerOptions,
  );
}

function runInstalledArtifactWorker(operation, payload, options = {}) {
  const WorkerClass = options.Worker ?? Worker;
  const workerPath =
    options.workerPath ??
    path.join(
      materializeInstalledArtifactWorkerBundle(options),
      "installedArtifactUpdateWorker.cjs",
    );
  return new Promise((resolve, reject) => {
    const worker = new WorkerClass(workerPath, {
      workerData: { operation, payload },
    });
    let settled = false;
    const settle = (callback, value) => {
      if (!settled) {
        settled = true;
        callback(value);
      }
    };
    worker.once("message", (message) => {
      if (message?.ok) {
        settle(resolve, message.result);
      } else {
        settle(
          reject,
          new Error(message?.error?.message ?? "Artifact worker failed"),
        );
      }
    });
    worker.once("error", (error) => settle(reject, error));
    worker.once("exit", (code) => {
      if (code !== 0) {
        settle(reject, new Error(`Artifact worker exited with code ${code}`));
      } else if (!settled) {
        settle(reject, new Error("Artifact worker exited without a result"));
      }
    });
  });
}

function materializeInstalledArtifactWorkerBundle(options = {}) {
  if (workerBundlePath) {
    return workerBundlePath;
  }
  const fsOps = resolveInstalledArtifactFileSystem(options);
  const bundlePath = (options.mkdtempSync ?? fsOps.mkdtempSync)(
    path.join(os.tmpdir(), "morpheus-artifact-worker-"),
  );
  try {
    for (const fileName of WORKER_FILES) {
      (options.copyFileSync ?? fsOps.copyFileSync)(
        path.join(__dirname, fileName),
        path.join(bundlePath, fileName),
      );
    }
  } catch (error) {
    let failure = error;
    try {
      (options.rmSync ?? fsOps.rmSync)(bundlePath, {
        force: true,
        recursive: true,
      });
    } catch (cleanupError) {
      failure = attachCleanupFailure(error, cleanupError);
    }
    throw failure;
  }
  workerBundlePath = bundlePath;
  process.once("exit", () => {
    try {
      fsOps.rmSync(bundlePath, { force: true, recursive: true });
    } catch {}
  });
  return bundlePath;
}

async function removeInstalledArtifactTree(targetPath, options = {}) {
  const fsOps = resolveInstalledArtifactFileSystem(options);
  const removeOptions = { force: true, recursive: true };
  if (typeof fsOps.promises?.rm === "function") {
    await fsOps.promises.rm(targetPath, removeOptions);
    return;
  }
  if (typeof fsOps.rm === "function") {
    await new Promise((resolve, reject) => {
      fsOps.rm(targetPath, removeOptions, (error) => {
        if (error) {
          reject(error);
        } else {
          resolve();
        }
      });
    });
    return;
  }
  fsOps.rmSync(targetPath, removeOptions);
}

function attachCleanupFailure(error, cleanupError) {
  const primaryError =
    error instanceof Error ? error : new Error(String(error));
  const cleanupMessage =
    cleanupError instanceof Error ? cleanupError.message : String(cleanupError);
  primaryError.message = `${primaryError.message}; Runtime Capsule cleanup also failed: ${cleanupMessage}`;
  Object.defineProperty(primaryError, "cleanupError", {
    configurable: true,
    enumerable: false,
    value: cleanupError,
  });
  return primaryError;
}

function run(command, args, options = {}) {
  const result = (options.spawnSync ?? spawnSync)(command, args, {
    cwd: options.cwd,
    encoding: "utf8",
    env: options.env,
    stdio: options.stdio ?? "pipe",
  });
  assertSuccessfulSpawn(result, `${command} ${args.join(" ")}`);
  return result.stdout ?? "";
}

function capture(command, args, options = {}) {
  return run(command, args, { ...options, stdio: "pipe" });
}

function assertSuccessfulSpawn(result, label) {
  if (result?.error) {
    throw result.error;
  }
  if (result?.status !== 0) {
    const stderr = normalizeString(result?.stderr);
    throw new Error(
      `${label} exited with ${String(result?.status)}${stderr ? `: ${stderr}` : ""}`,
    );
  }
}

function validateActivationId(value) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length > 96 ||
    !/^[A-Za-z0-9_.-]+$/.test(value)
  ) {
    throw new Error(`Invalid Runtime Capsule activation id: ${String(value)}`);
  }
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
  APP_NAME,
  GENERATED_SOURCE_DIR_NAMES,
  PAYLOAD_EXECUTABLE_RELATIVE_PATH,
  PAYLOAD_RELATIVE_PATH,
  buildRuntimeSources,
  clearExtendedAttributes,
  materializeInstalledArtifactWorkerBundle,
  normalizeRuntimeCapsuleTree,
  removeInstalledArtifactTree,
  resolveCargoTargetDirectory,
  resolveInstalledArtifactFileSystem,
  resolveInstalledArtifactUpdatePlan,
  resolveInstalledArtifactUpdatePlanInWorker,
  resolveRuntimeLauncherStateRoot,
  runInstalledArtifactWorker,
  stagePayloadResources,
  updateInstalledArtifacts,
  updateInstalledArtifactsInWorker,
};
