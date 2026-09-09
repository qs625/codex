const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { Worker } = require("node:worker_threads");
const { buildDesktopEnvironment } = require("./environment.cjs");
const {
  isPackagedApp,
  resolveDefaultWorkspace,
} = require("./workspace.cjs");

const APP_ASAR_RELATIVE_PATH = "app.asar";
const APP_SERVER_RELATIVE_PATH = path.join("bin", "app-server");
const DEFAULT_COMPACT_RELATIVE_PATH = path.join(
  "default-config",
  "compact",
  "COMPACT.md",
);
const SOURCE_APP_RELATIVE_PATH = path.join("apps", "root-worker-prototype");
const CANDIDATE_PREFIX = "morpheus-runtime-candidate-";
const ELECTRON_SHELL_RELATIVE_DIR = "electron";
const WORKER_FILES = [
  "environment.cjs",
  "installedArtifactUpdate.cjs",
  "installedArtifactUpdateWorker.cjs",
  "workspace.cjs",
];
let workerBundlePath = null;

function resolveInstalledArtifactUpdatePlan({
  commandEnv,
  env = process.env,
  isPackaged = isPackagedApp({ resourcesPath }),
  platform = process.platform,
  resourcesPath = currentResourcesPath(),
  spawnSync: spawn = spawnSync,
  workspace,
} = {}) {
  if (platform !== "darwin" || !isPackaged || !resourcesPath) {
    return null;
  }
  const resolvedWorkspace =
    workspace ?? resolveDefaultWorkspace(env, { isPackagedApp: true });
  if (!resolvedWorkspace) {
    return null;
  }
  const resolvedCommandEnv = commandEnv ?? buildDesktopEnvironment(env);
  const codexRsDir = path.join(resolvedWorkspace, "codex-rs");
  const cargoTargetDir = resolveCargoTargetDirectory({
    codexRsDir,
    env: resolvedCommandEnv,
    spawnSync: spawn,
  });
  const sourceAppDir = path.join(resolvedWorkspace, SOURCE_APP_RELATIVE_PATH);
  const electronShell = resolveElectronShellUpdate({
    commandEnv: resolvedCommandEnv,
    resourcesPath,
    sourceAppDir,
    spawnSync: spawn,
    workspace: resolvedWorkspace,
  });
  return {
    appBundlePath: path.dirname(path.dirname(resourcesPath)),
    appServerBinaryPath: path.join(cargoTargetDir, "release", "app-server"),
    commandEnv: resolvedCommandEnv,
    defaultCompactPromptSourcePath: path.join(
      codexRsDir,
      "thread-service",
      "templates",
      "compact",
      "prompt.md",
    ),
    frontendDistPath: path.join(sourceAppDir, "dist"),
    requiresFullRelaunch: electronShell.changed,
    resourcesPath,
    runtimeUpdate: { electronShell },
    sourceAppDir,
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

function resolveElectronShellUpdate({
  commandEnv = buildDesktopEnvironment(),
  mkdtempSync = fs.mkdtempSync,
  readFileSync = fs.readFileSync,
  readdirSync = fs.readdirSync,
  rmSync = fs.rmSync,
  resourcesPath,
  sourceAppDir,
  spawnSync: spawn = spawnSync,
  workspace = sourceAppDir,
} = {}) {
  const extractedRoot = mkdtempSync(
    path.join(os.tmpdir(), "morpheus-installed-asar-"),
  );
  try {
    extractInstalledAppAsar(
      path.join(resourcesPath, APP_ASAR_RELATIVE_PATH),
      extractedRoot,
      { commandEnv, spawnSync: spawn, workspace },
    );
    const relativePaths = [
      ...new Set([
        ...listElectronShellSourceRelativePaths(sourceAppDir, { readdirSync }),
        ...listElectronShellRelativePaths(extractedRoot, { readdirSync }),
      ]),
    ].sort();
    const changedPaths = relativePaths.filter((relativePath) => {
      const source = readDigest(
        path.join(sourceAppDir, relativePath),
        readFileSync,
      );
      const installed = readDigest(
        path.join(extractedRoot, relativePath),
        readFileSync,
      );
      return source !== installed;
    });
    return {
      category: "electronShell",
      changed: changedPaths.length > 0,
      changedPaths,
    };
  } finally {
    rmSync(extractedRoot, { force: true, recursive: true });
  }
}

function listElectronShellSourceRelativePaths(sourceAppDir, options = {}) {
  return listElectronShellRelativePaths(sourceAppDir, options);
}

function listElectronShellRelativePaths(appRoot, options = {}) {
  const electronRoot = path.join(appRoot, ELECTRON_SHELL_RELATIVE_DIR);
  const result = [];
  collectElectronShellFiles(electronRoot, electronRoot, result, {
    readdirSync: options.readdirSync ?? fs.readdirSync,
  });
  return result.sort();
}

function collectElectronShellFiles(current, electronRoot, result, options) {
  let entries;
  try {
    entries = options.readdirSync(current, { withFileTypes: true });
  } catch (error) {
    if (error?.code === "ENOENT" || error?.code === "ENOTDIR") {
      return;
    }
    throw error;
  }
  for (const entry of entries) {
    const entryPath = path.join(current, entry.name);
    if (entry.isDirectory()) {
      collectElectronShellFiles(entryPath, electronRoot, result, options);
    } else if (
      entry.isFile() &&
      entry.name.endsWith(".cjs") &&
      !entry.name.endsWith(".test.cjs")
    ) {
      result.push(
        path.join(
          ELECTRON_SHELL_RELATIVE_DIR,
          path.relative(electronRoot, entryPath),
        ),
      );
    }
  }
}

function extractInstalledAppAsar(
  archivePath,
  destinationPath,
  {
    commandEnv = buildDesktopEnvironment(),
    spawnSync: spawn = spawnSync,
    workspace = path.dirname(archivePath),
  } = {},
) {
  const result = spawn(
    "pnpm",
    ["dlx", "@electron/asar", "extract", archivePath, destinationPath],
    {
      cwd: workspace,
      encoding: "utf8",
      env: commandEnv,
      stdio: "pipe",
    },
  );
  assertSuccessfulSpawn(result, "pnpm dlx @electron/asar extract");
}

function updateInstalledArtifacts(plan, options = {}) {
  return prepareInstalledArtifacts(plan, options);
}

function prepareInstalledArtifacts(plan, options = {}) {
  assertPreparedSources(plan, options);
  const fsOps = {
    cpSync: options.cpSync ?? fs.cpSync,
    mkdirSync: options.mkdirSync ?? fs.mkdirSync,
    mkdtempSync: options.mkdtempSync ?? fs.mkdtempSync,
    readFileSync: options.readFileSync ?? fs.readFileSync,
    rmSync: options.rmSync ?? fs.rmSync,
    statSync: options.statSync ?? fs.statSync,
    writeFileSync: options.writeFileSync ?? fs.writeFileSync,
  };
  const preparedRoot = fsOps.mkdtempSync(
    path.join(options.candidateParent ?? os.tmpdir(), CANDIDATE_PREFIX),
  );
  const resourcesRoot = path.join(preparedRoot, "resources");
  const appSourceRoot = path.join(preparedRoot, "app-source");
  const appAsarPath = path.join(resourcesRoot, APP_ASAR_RELATIVE_PATH);
  const appServerPath = path.join(resourcesRoot, APP_SERVER_RELATIVE_PATH);
  const compactPath = path.join(resourcesRoot, DEFAULT_COMPACT_RELATIVE_PATH);
  try {
    fsOps.mkdirSync(path.dirname(appAsarPath), { recursive: true });
    fsOps.mkdirSync(path.dirname(appServerPath), { recursive: true });
    fsOps.mkdirSync(path.dirname(compactPath), { recursive: true });
    fsOps.cpSync(plan.sourceAppDir, appSourceRoot, {
      recursive: true,
      filter: candidateSourceFilter,
    });
    packAppAsar(plan, appSourceRoot, appAsarPath, options);
    fsOps.rmSync(appSourceRoot, { force: true, recursive: true });
    fsOps.cpSync(plan.appServerBinaryPath, appServerPath);
    signCandidateAppServer(plan, appServerPath, options);
    fsOps.cpSync(plan.defaultCompactPromptSourcePath, compactPath);

    const sourceCommit =
      normalizeString(options.sourceCommit) ??
      resolveSourceCommit(plan.workspace, {
        env: plan.commandEnv,
        spawnSync: options.spawnSync,
      });
    const artifacts = [
      artifactDescriptor(resourcesRoot, APP_ASAR_RELATIVE_PATH, fsOps),
      artifactDescriptor(resourcesRoot, APP_SERVER_RELATIVE_PATH, fsOps),
      artifactDescriptor(resourcesRoot, DEFAULT_COMPACT_RELATIVE_PATH, fsOps),
    ];
    const buildId =
      normalizeString(options.buildId) ??
      sha256(
        Buffer.from(
          JSON.stringify({
            sourceCommit,
            artifacts: artifacts.map(({ relativePath, sha256 }) => ({
              relativePath,
              sha256,
            })),
          }),
        ),
      ).slice(0, 24);
    const manifest = {
      schemaVersion: 1,
      buildId,
      sourceCommit,
      entrypoint: APP_ASAR_RELATIVE_PATH,
      artifacts,
    };
    fsOps.writeFileSync(
      path.join(preparedRoot, "manifest.json"),
      `${JSON.stringify(manifest, null, 2)}\n`,
      { encoding: "utf8", mode: 0o600 },
    );
    return {
      ok: true,
      updated: false,
      appBundlePath: plan.appBundlePath,
      buildId,
      changes: {
        main:
          plan.runtimeUpdate?.electronShell?.changedPaths?.some(
            (relativePath) => relativePath !== "electron/preload.cjs",
          ) === true,
        preload:
          plan.runtimeUpdate?.electronShell?.changedPaths?.includes(
            "electron/preload.cjs",
          ) === true,
      },
      manifest,
      preparedRoot,
      sourceCommit,
      transactionId: options.transactionId ?? crypto.randomUUID(),
    };
  } catch (error) {
    fsOps.rmSync(preparedRoot, { force: true, recursive: true });
    throw error;
  }
}

function signCandidateAppServer(plan, appServerPath, options = {}) {
  const result = (options.spawnSync ?? spawnSync)(
    "codesign",
    ["--force", "--sign", "-", appServerPath],
    {
      cwd: plan.workspace,
      encoding: "utf8",
      env: plan.commandEnv,
      stdio: options.stdio ?? "pipe",
    },
  );
  assertSuccessfulSpawn(result, "codesign candidate app-server");
}

function packAppAsar(plan, sourceRoot, targetPath, options = {}) {
  const result = (options.spawnSync ?? spawnSync)(
    "pnpm",
    ["dlx", "@electron/asar", "pack", sourceRoot, targetPath],
    {
      cwd: plan.workspace,
      encoding: "utf8",
      env: plan.commandEnv,
      stdio: options.stdio ?? "pipe",
    },
  );
  assertSuccessfulSpawn(result, "pnpm dlx @electron/asar pack");
}

function resolveSourceCommit(workspace, options = {}) {
  const result = (options.spawnSync ?? spawnSync)(
    "git",
    ["rev-parse", "HEAD"],
    {
      cwd: workspace,
      encoding: "utf8",
      env: options.env,
      stdio: "pipe",
    },
  );
  assertSuccessfulSpawn(result, "git rev-parse HEAD");
  const commit = normalizeString(result.stdout);
  if (!commit) {
    throw new Error("git rev-parse HEAD returned an empty commit");
  }
  return commit;
}

function artifactDescriptor(resourcesRoot, relativePath, fsOps) {
  const artifactPath = path.join(resourcesRoot, relativePath);
  if (!fsOps.statSync(artifactPath).isFile()) {
    throw new Error(`Prepared artifact is not a file: ${artifactPath}`);
  }
  return {
    relativePath,
    sha256: sha256(fsOps.readFileSync(artifactPath)),
  };
}

function assertPreparedSources(plan, options = {}) {
  const statSync = options.statSync ?? fs.statSync;
  for (const [label, targetPath, type] of [
    ["source app", plan.sourceAppDir, "directory"],
    ["frontend dist", plan.frontendDistPath, "directory"],
    ["release app-server", plan.appServerBinaryPath, "file"],
    ["default compact prompt", plan.defaultCompactPromptSourcePath, "file"],
  ]) {
    let stat;
    try {
      stat = statSync(targetPath);
    } catch (error) {
      throw new Error(`Missing ${label}: ${targetPath}`, { cause: error });
    }
    if (
      (type === "file" && !stat.isFile()) ||
      (type === "directory" && !stat.isDirectory())
    ) {
      throw new Error(`Expected ${label} to be a ${type}: ${targetPath}`);
    }
  }
}

function candidateSourceFilter(source) {
  const name = path.basename(source);
  return (
    name !== "dist-app" &&
    name !== "dist-package-resources" &&
    !name.startsWith(CANDIDATE_PREFIX)
  );
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
  const bundlePath = (options.mkdtempSync ?? fs.mkdtempSync)(
    path.join(os.tmpdir(), "morpheus-artifact-worker-"),
  );
  try {
    for (const fileName of WORKER_FILES) {
      (options.copyFileSync ?? fs.copyFileSync)(
        path.join(__dirname, fileName),
        path.join(bundlePath, fileName),
      );
    }
  } catch (error) {
    (options.rmSync ?? fs.rmSync)(bundlePath, {
      force: true,
      recursive: true,
    });
    throw error;
  }
  workerBundlePath = bundlePath;
  process.once("exit", () => {
    try {
      fs.rmSync(bundlePath, { force: true, recursive: true });
    } catch {}
  });
  return bundlePath;
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

function readDigest(filePath, readFileSync) {
  try {
    return sha256(readFileSync(filePath));
  } catch {
    return null;
  }
}

function sha256(value) {
  return crypto.createHash("sha256").update(value).digest("hex");
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
  APP_ASAR_RELATIVE_PATH,
  APP_SERVER_RELATIVE_PATH,
  DEFAULT_COMPACT_RELATIVE_PATH,
  extractInstalledAppAsar,
  listElectronShellSourceRelativePaths,
  materializeInstalledArtifactWorkerBundle,
  packAppAsar,
  prepareInstalledArtifacts,
  resolveCargoTargetDirectory,
  resolveElectronShellUpdate,
  resolveInstalledArtifactUpdatePlan,
  resolveInstalledArtifactUpdatePlanInWorker,
  runInstalledArtifactWorker,
  signCandidateAppServer,
  updateInstalledArtifacts,
  updateInstalledArtifactsInWorker,
};
