const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { Worker } = require("node:worker_threads");
const { buildDesktopEnvironment } = require("./environment.cjs");
const { isPackagedApp, resolveDefaultWorkspace } = require("./workspace.cjs");

const APP_ASAR_RELATIVE_PATH = "app.asar";
const APP_SERVER_RELATIVE_PATH = path.join("bin", "app-server");
const DEFAULT_CONFIG_RELATIVE_PATH = "default-config";
const ELECTRON_SHELL_RELATIVE_DIR = "electron";
const PREPARED_ARTIFACT_SCHEMA_VERSION = 1;
const PREPARED_ARTIFACT_BUILDING_PREFIX = "morpheus-prepared-building-";
const PREPARED_ARTIFACT_PREFIX = "morpheus-prepared-runtime-";
const PREPARED_ARTIFACT_OWNER_FILE = ".morpheus-prepared-artifact.json";
const PREPARED_ARTIFACT_OWNER_SCHEMA_VERSION = 1;
const PREPARED_ARTIFACT_PRODUCER_LEASE_MS = 10 * 60 * 1000;
const PREPARED_ARTIFACT_GC_CURSOR_FILE = ".morpheus-prepared-gc-cursor";
const PREPARED_ARTIFACT_GC_LIMIT = 32;
const PREPARED_ARTIFACT_GC_SCAN_LIMIT = 256;
const SOURCE_APP_RELATIVE_PATH = path.join("apps", "root-worker-prototype");
const INSTALLED_ARTIFACT_WORKER_FILES = [
  "environment.cjs",
  "installedArtifactUpdate.cjs",
  "installedArtifactUpdateWorker.cjs",
  "workspace.cjs",
];
let installedArtifactWorkerBundlePath = null;

function resolveInstalledArtifactUpdatePlan({
  cargoCwd,
  commandEnv,
  desktopEnvironmentOptions,
  env = process.env,
  platform = process.platform,
  resourcesPath = currentResourcesPath(),
  spawnSync: spawn = spawnSync,
  workspace,
  isPackaged = isPackagedApp({ resourcesPath }),
} = {}) {
  if (platform !== "darwin" || !isPackaged || !resourcesPath) {
    return null;
  }
  const resolvedWorkspace =
    workspace ?? resolveDefaultWorkspace(env, { isPackagedApp: true });
  if (!resolvedWorkspace) {
    return null;
  }
  const sourceAppDir = path.join(resolvedWorkspace, SOURCE_APP_RELATIVE_PATH);
  const codexRsDir = path.join(resolvedWorkspace, "codex-rs");
  const resolvedCommandEnv = {
    ...(commandEnv ?? buildDesktopEnvironment(env, desktopEnvironmentOptions)),
  };
  const targetDir = resolveCargoTargetDirectory({
    cargoCwd: cargoCwd ?? resolvedWorkspace,
    codexRsDir,
    env: resolvedCommandEnv,
    spawnSync: spawn,
  });
  const appBundlePath = path.dirname(path.dirname(resourcesPath));
  const preparedArtifactsRoot = resolvePreparedArtifactsRoot(env, {
    required: false,
  });
  const shellUpdate = resolveElectronShellUpdate({
    resourcesPath,
    sourceAppDir,
  });
  return {
    appBundlePath,
    appServerBinaryPath: path.join(targetDir, "release", "app-server"),
    commandEnv: resolvedCommandEnv,
    defaultCompactPromptSourcePath: path.join(
      codexRsDir,
      "thread-service",
      "templates",
      "compact",
      "prompt.md",
    ),
    frontendDistPath: path.join(sourceAppDir, "dist"),
    preparedArtifactsRoot,
    resourcesPath,
    requiresFullRelaunch: shellUpdate.changed,
    runtimeUpdate: { electronShell: shellUpdate },
    sourceAppDir,
    workspace: resolvedWorkspace,
  };
}

function resolveCargoTargetDirectory({
  cargoCwd,
  codexRsDir,
  env = process.env,
  spawnSync: spawn = spawnSync,
}) {
  const configuredTarget = normalizeString(env.CARGO_TARGET_DIR);
  if (!configuredTarget) {
    return path.join(path.resolve(codexRsDir), "target");
  }
  if (configuredTarget && path.isAbsolute(configuredTarget)) {
    return path.resolve(configuredTarget);
  }
  const commandCwd = path.resolve(cargoCwd);
  const result = spawn(
    "cargo",
    [
      "metadata",
      "--format-version",
      "1",
      "--no-deps",
      "--manifest-path",
      path.join(codexRsDir, "Cargo.toml"),
    ],
    {
      cwd: commandCwd,
      encoding: "utf8",
      env,
      stdio: "pipe",
    },
  );
  if (!result?.error && result?.status === 0) {
    try {
      const targetDirectory = normalizeString(
        JSON.parse(result.stdout)?.target_directory,
      );
      if (targetDirectory) {
        return path.resolve(commandCwd, targetDirectory);
      }
    } catch {
      // Fall back to Cargo's cwd-relative CARGO_TARGET_DIR semantics below.
    }
  }
  return path.resolve(commandCwd, configuredTarget ?? "target");
}

function resolveElectronShellUpdate({
  resourcesPath,
  sourceAppDir,
  relativePaths,
  readFileSync = fs.readFileSync,
  readdirSync = fs.readdirSync,
} = {}) {
  const shellRelativePaths = relativePaths ?? [
    ...new Set([
      ...listElectronShellSourceRelativePaths(sourceAppDir, { readdirSync }),
      ...listInstalledElectronShellRelativePaths(resourcesPath, { readdirSync }),
    ]),
  ].sort();
  const changedPaths = [];
  const missingInstalledPaths = [];
  const missingSourcePaths = [];
  for (const relativePath of shellRelativePaths) {
    const sourceDigest = readDigest(
      path.join(sourceAppDir, relativePath),
      readFileSync,
      missingSourcePaths,
      relativePath,
    );
    const installedDigest = readDigest(
      path.join(resourcesPath, APP_ASAR_RELATIVE_PATH, relativePath),
      readFileSync,
      missingInstalledPaths,
      relativePath,
    );
    if (!sourceDigest || !installedDigest || sourceDigest !== installedDigest) {
      changedPaths.push(relativePath);
    }
  }
  return {
    category: "electronShell",
    changed: changedPaths.length > 0,
    changedPaths,
    missingInstalledPaths,
    missingSourcePaths,
  };
}

function listElectronShellSourceRelativePaths(sourceAppDir, options = {}) {
  return listElectronShellRelativePaths(sourceAppDir, options);
}

function listInstalledElectronShellRelativePaths(resourcesPath, options = {}) {
  return listElectronShellRelativePaths(
    path.join(resourcesPath, APP_ASAR_RELATIVE_PATH),
    options,
  );
}

function listElectronShellRelativePaths(appRoot, options = {}) {
  const readdirSync = options.readdirSync ?? fs.readdirSync;
  const electronRoot = path.join(appRoot, ELECTRON_SHELL_RELATIVE_DIR);
  const relativePaths = [];
  try {
    collectElectronShellFiles({
      currentPath: electronRoot,
      electronRoot,
      relativePaths,
      readdirSync,
    });
  } catch (error) {
    if (error?.code !== "ENOENT" && error?.code !== "ENOTDIR") {
      throw error;
    }
  }
  return relativePaths.sort();
}

function collectElectronShellFiles({
  currentPath,
  electronRoot,
  relativePaths,
  readdirSync,
}) {
  for (const entry of readdirSync(currentPath, { withFileTypes: true })) {
    const entryPath = path.join(currentPath, entry.name);
    if (entry.isDirectory()) {
      collectElectronShellFiles({
        currentPath: entryPath,
        electronRoot,
        relativePaths,
        readdirSync,
      });
    } else if (
      entry.isFile() &&
      entry.name.endsWith(".cjs") &&
      !entry.name.endsWith(".test.cjs")
    ) {
      relativePaths.push(
        path.join(
          ELECTRON_SHELL_RELATIVE_DIR,
          path.relative(electronRoot, entryPath),
        ),
      );
    }
  }
}

function readDigest(filePath, readFileSync, missingPaths, relativePath) {
  try {
    return sha256(readFileSync(filePath));
  } catch {
    missingPaths.push(relativePath);
    return null;
  }
}

function updateInstalledArtifacts(plan, options = {}) {
  return prepareInstalledArtifacts(plan, options);
}

function prepareInstalledArtifacts(plan, options = {}) {
  assertPreparedArtifactSources(plan, options);
  const fsOps = {
    chmodSync: options.chmodSync ?? fs.chmodSync,
    cpSync: options.cpSync ?? fs.cpSync,
    lstatSync: options.lstatSync ?? fs.lstatSync,
    mkdirSync: options.mkdirSync ?? fs.mkdirSync,
    mkdtempSync: options.mkdtempSync ?? fs.mkdtempSync,
    readFileSync: options.readFileSync ?? fs.readFileSync,
    realpathSync: options.realpathSync ?? fs.realpathSync,
    renameSync: options.renameSync ?? fs.renameSync,
    rmSync: options.rmSync ?? fs.rmSync,
    statSync: options.statSync ?? fs.statSync,
    writeFileSync: options.writeFileSync ?? fs.writeFileSync,
  };
  const transactionId = options.transactionId ?? crypto.randomUUID();
  const ownerCapability = {
    ownerToken: options.ownerToken ?? crypto.randomUUID(),
    producerPid: process.pid,
    transactionId,
  };
  const preparedArtifactsRoot =
    options.preparedArtifactsRoot ??
    plan.preparedArtifactsRoot ??
    resolvePreparedArtifactsRoot(options.env ?? plan.commandEnv);
  const canonicalPreparedArtifactsRoot = canonicalizePreparedArtifactsRoot(
    preparedArtifactsRoot,
    fsOps,
  );
  const preparedRoot = options.preparedRoot
    ? validatePreparedArtifactCandidatePath(
        options.preparedRoot,
        canonicalPreparedArtifactsRoot,
        fsOps,
        { allowMissing: true },
      )
    : null;
  if (preparedRoot && pathExists(preparedRoot, fsOps)) {
    throw new Error("Prepared artifact publish target already exists");
  }
  const buildingRoot = fsOps.mkdtempSync(
    path.join(
      canonicalPreparedArtifactsRoot,
      PREPARED_ARTIFACT_BUILDING_PREFIX,
    ),
  );
  const publishedRoot =
    preparedRoot ??
    path.join(
      canonicalPreparedArtifactsRoot,
      `${PREPARED_ARTIFACT_PREFIX}${path.basename(buildingRoot).slice(
        PREPARED_ARTIFACT_BUILDING_PREFIX.length,
      )}`,
    );
  const resourcesRoot = path.join(buildingRoot, "resources");
  const appSourceRoot = path.join(buildingRoot, "app-source");
  const appAsarArtifactPath = APP_ASAR_RELATIVE_PATH;
  const appServerArtifactPath = APP_SERVER_RELATIVE_PATH;
  const compactPromptArtifactPath = path.join(
    DEFAULT_CONFIG_RELATIVE_PATH,
    "compact",
    "COMPACT.md",
  );
  const appAsarPath = path.join(resourcesRoot, appAsarArtifactPath);
  const appServerPath = path.join(resourcesRoot, appServerArtifactPath);
  const compactPromptPath = path.join(
    resourcesRoot,
    compactPromptArtifactPath,
  );
  try {
    writePreparedArtifactOwnerMarker(buildingRoot, transactionId, fsOps, {
      now: options.now,
      ownerToken: ownerCapability.ownerToken,
    });
    fsOps.mkdirSync(path.dirname(appAsarPath), { recursive: true });
    fsOps.mkdirSync(path.dirname(appServerPath), { recursive: true });
    fsOps.mkdirSync(path.dirname(compactPromptPath), { recursive: true });
    fsOps.cpSync(plan.sourceAppDir, appSourceRoot, {
      recursive: true,
      filter: preparedAppSourceFilter,
    });
    packAppAsar(plan, appSourceRoot, appAsarPath, options);
    fsOps.cpSync(plan.appServerBinaryPath, appServerPath);
    fsOps.cpSync(plan.defaultCompactPromptSourcePath, compactPromptPath);

    const artifacts = [
      artifactDescriptor(resourcesRoot, appAsarArtifactPath, "file", fsOps),
      artifactDescriptor(
        resourcesRoot,
        appServerArtifactPath,
        "executable",
        fsOps,
      ),
      artifactDescriptor(
        resourcesRoot,
        compactPromptArtifactPath,
        "file",
        fsOps,
      ),
    ];
    const sourceCommit =
      normalizeString(options.sourceCommit) ??
      resolveSourceCommit(plan.workspace, {
        ...options,
        env: options.env ?? plan.commandEnv,
      });
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
      schemaVersion: PREPARED_ARTIFACT_SCHEMA_VERSION,
      buildId,
      sourceCommit,
      artifacts,
      changes: {
        main: plan.runtimeUpdate?.electronShell?.changed === true,
        preload: plan.runtimeUpdate?.electronShell?.changedPaths?.includes(
          "electron/preload.cjs",
        ) ?? false,
      },
    };
    if (
      manifest.artifacts.some(
        ({ relativePath }) =>
          relativePath ===
            path.join("Contents", "MacOS", "MorpheusLauncher") ||
          path.basename(relativePath) === "MorpheusLauncher",
      )
    ) {
      throw new Error(
        "Prepared runtime artifacts must not replace the stable launcher",
      );
    }
    fsOps.writeFileSync(
      path.join(buildingRoot, "manifest.json"),
      `${JSON.stringify(manifest, null, 2)}\n`,
      { encoding: "utf8", mode: 0o600 },
    );
    writePreparedArtifactOwnerMarker(buildingRoot, transactionId, fsOps, {
      now: options.publishNow,
      ownerToken: ownerCapability.ownerToken,
    });
    fsOps.renameSync(buildingRoot, publishedRoot);
    return {
      ok: true,
      updated: false,
      appBundlePath: plan.appBundlePath,
      buildId,
      manifest,
      preparedArtifactOwner: ownerCapability,
      preparedArtifactsRoot: canonicalPreparedArtifactsRoot,
      preparedRoot: publishedRoot,
      sourceCommit,
      transactionId,
    };
  } catch (error) {
    fsOps.rmSync(buildingRoot, { force: true, recursive: true });
    throw error;
  }
}

function resolvePreparedArtifactsRoot(
  env = process.env,
  { required = true } = {},
) {
  const launcherHome = normalizeString(env?.MORPHEUS_RUNTIME_LAUNCHER_HOME);
  if (launcherHome) {
    return path.join(path.resolve(launcherHome), "producer-artifacts");
  }
  const morpheusHome =
    normalizeString(env?.MORPHEUS_HOME) ??
    (normalizeString(env?.HOME)
      ? path.join(path.resolve(env.HOME), ".morpheus")
      : null);
  if (!morpheusHome) {
    if (!required) {
      return null;
    }
    throw new Error(
      "MORPHEUS_HOME or HOME is required for prepared runtime artifacts",
    );
  }
  return path.join(
    path.resolve(morpheusHome),
    "runtime-launcher",
    "producer-artifacts",
  );
}

function writePreparedArtifactOwnerMarker(
  preparedRoot,
  transactionId,
  fsOps,
  options = {},
) {
  const now = options.now ?? Date.now();
  const ownerToken = options.ownerToken ?? crypto.randomUUID();
  fsOps.writeFileSync(
    path.join(preparedRoot, PREPARED_ARTIFACT_OWNER_FILE),
    `${JSON.stringify({
      schemaVersion: PREPARED_ARTIFACT_OWNER_SCHEMA_VERSION,
      transactionId,
      state: "published",
      ownerToken,
      producerPid: process.pid,
      producerLeaseExpiresAt: new Date(
        now + PREPARED_ARTIFACT_PRODUCER_LEASE_MS,
      ).toISOString(),
    })}\n`,
    { encoding: "utf8", mode: 0o600 },
  );
}

function releaseOwnedPreparedArtifactLease(preparedRoot, options = {}) {
  const fsOps = {
    ...preparedArtifactGcFileSystem(options),
    renameSync: options.renameSync ?? fs.renameSync,
    writeFileSync: options.writeFileSync ?? fs.writeFileSync,
  };
  const parentRoot =
    options.preparedArtifactsRoot ??
    resolvePreparedArtifactsRoot(options.env);
  const canonicalParent = canonicalizePreparedArtifactsRoot(parentRoot, fsOps);
  const candidate = validatePreparedArtifactCandidatePath(
    preparedRoot,
    canonicalParent,
    fsOps,
  );
  const marker = readPreparedArtifactOwnerMarker(candidate, fsOps);
  assertPreparedArtifactOwnerCapability(marker, options.ownerCapability);
  const markerPath = path.join(candidate, PREPARED_ARTIFACT_OWNER_FILE);
  writeFileAtomicallyNoFollow({
    contents: `${JSON.stringify({
      ...marker,
      state: "handedOff",
      producerLeaseExpiresAt: null,
      handedOffAt: new Date(options.now ?? Date.now()).toISOString(),
    })}\n`,
    directory: candidate,
    fsOps,
    randomUUID: options.randomUUID,
    targetPath: markerPath,
    temporaryPrefix: `${PREPARED_ARTIFACT_OWNER_FILE}.handoff-${process.pid}-`,
  });
  return { ok: true, released: candidate };
}

function removeOwnedPreparedArtifact(preparedRoot, options = {}) {
  const fsOps = preparedArtifactGcFileSystem(options);
  const parentRoot =
    options.preparedArtifactsRoot ??
    resolvePreparedArtifactsRoot(options.env);
  const canonicalParent = canonicalizePreparedArtifactsRoot(parentRoot, fsOps);
  const candidate = validatePreparedArtifactCandidatePath(
    preparedRoot,
    canonicalParent,
    fsOps,
  );
  const marker = readPreparedArtifactOwnerMarker(candidate, fsOps);
  assertPreparedArtifactOwnerCapability(marker, options.ownerCapability);
  fsOps.rmSync(candidate, { force: true, recursive: true });
  return { ok: true, removed: candidate };
}

function garbageCollectOwnedPreparedArtifacts(options = {}) {
  const fsOps = preparedArtifactGcFileSystem(options);
  const parentRoot =
    options.preparedArtifactsRoot ??
    resolvePreparedArtifactsRoot(options.env);
  const canonicalParent = canonicalizePreparedArtifactsRoot(parentRoot, fsOps);
  const activePreparedRoots = new Set();
  for (const active of options.activePreparedRoots ?? []) {
    addActivePreparedRootAliases(activePreparedRoots, active, fsOps);
  }
  const result = { removed: [], preserved: [], rejected: [] };
  const removalLimit = Math.max(
    0,
    Math.min(
      PREPARED_ARTIFACT_GC_LIMIT,
      Number.isSafeInteger(options.limit)
        ? options.limit
        : PREPARED_ARTIFACT_GC_LIMIT,
    ),
  );
  const scanLimit = Math.max(
    removalLimit,
    Math.min(
      PREPARED_ARTIFACT_GC_SCAN_LIMIT,
      Number.isSafeInteger(options.scanLimit)
        ? options.scanLimit
        : PREPARED_ARTIFACT_GC_SCAN_LIMIT,
    ),
  );
  const entries = fsOps
    .readdirSync(canonicalParent, { withFileTypes: true })
    .filter((entry) => entry.name.startsWith(PREPARED_ARTIFACT_PREFIX))
    .sort((left, right) => left.name.localeCompare(right.name));
  const cursor = readPreparedArtifactGcCursor(canonicalParent, fsOps);
  const selectedEntries = selectPreparedArtifactGcEntries(
    entries,
    cursor,
    scanLimit,
  );
  const candidates = [];
  const now = options.now ?? Date.now();
  for (const entry of selectedEntries) {
    const candidatePath = path.join(canonicalParent, entry.name);
    try {
      const candidate = validatePreparedArtifactCandidatePath(
        candidatePath,
        canonicalParent,
        fsOps,
      );
      if (activePreparedRoots.has(candidate)) {
        result.preserved.push(candidate);
        continue;
      }
      const marker = readPreparedArtifactOwnerMarker(candidate, fsOps);
      if (
        hasActiveProducerLease(
          marker,
          now,
          options.isProcessAlive ?? isProcessAlive,
        )
      ) {
        result.preserved.push(candidate);
        continue;
      }
      candidates.push(candidate);
    } catch (error) {
      result.rejected.push({
        path: candidatePath,
        reason: error instanceof Error ? error.message : String(error),
      });
    }
  }
  if (selectedEntries.length > 0) {
    writePreparedArtifactGcCursor(
      canonicalParent,
      selectedEntries.at(-1).name,
      fsOps,
    );
  }
  if (typeof options.refreshActivePreparedRoots === "function") {
    let refreshed;
    try {
      refreshed = options.refreshActivePreparedRoots();
    } catch {
      refreshed = null;
    }
    if (!Array.isArray(refreshed)) {
      result.preserved.push(...candidates);
      return result;
    }
    for (const active of refreshed) {
      addActivePreparedRootAliases(activePreparedRoots, active, fsOps);
    }
  }
  for (const candidate of candidates) {
    if (result.removed.length >= removalLimit) {
      break;
    }
    if (activePreparedRoots.has(candidate)) {
      result.preserved.push(candidate);
      continue;
    }
    try {
      fsOps.rmSync(candidate, { force: true, recursive: true });
      result.removed.push(candidate);
    } catch (error) {
      result.rejected.push({
        path: candidate,
        reason: error instanceof Error ? error.message : String(error),
      });
    }
  }
  return result;
}

function cleanupOwnedPreparedArtifactsWithLauncher({
  appBundlePath,
  env = process.env,
  isProcessAlive: processIsAlive,
  logger = console,
  now,
  runtimeLauncher,
} = {}) {
  if (
    !runtimeLauncher?.supported ||
    typeof runtimeLauncher.status !== "function" ||
    !appBundlePath
  ) {
    return { skipped: true, reason: "launcher status unavailable" };
  }
  let status;
  try {
    status = runtimeLauncher.status(appBundlePath);
  } catch (error) {
    logger?.warn?.(
      `[prototype] prepared artifact cleanup deferred: ${
        error instanceof Error ? error.message : String(error)
      }`,
    );
    return { skipped: true, reason: "launcher status failed" };
  }
  const transaction = status?.result?.transaction;
  const activePreparedRoots = activePreparedRootsFromTransaction(transaction);
  return garbageCollectOwnedPreparedArtifacts({
    activePreparedRoots,
    env,
    isProcessAlive: processIsAlive,
    now,
    refreshActivePreparedRoots: () => {
      const refreshed = runtimeLauncher.status(appBundlePath);
      return activePreparedRootsFromTransaction(
        refreshed?.result?.transaction,
      );
    },
  });
}

function validatePreparedArtifactCandidatePath(
  candidatePath,
  canonicalParent,
  fsOps,
  { allowMissing = false } = {},
) {
  if (typeof candidatePath !== "string" || !path.isAbsolute(candidatePath)) {
    throw new Error("Prepared artifact path must be absolute");
  }
  const resolved = path.resolve(candidatePath);
  const comparableResolved = resolveKnownDarwinSystemPathAlias(resolved);
  const comparableParent = resolveKnownDarwinSystemPathAlias(canonicalParent);
  if (
    path.dirname(comparableResolved) !== comparableParent ||
    !path.basename(resolved).startsWith(PREPARED_ARTIFACT_PREFIX)
  ) {
    throw new Error("Prepared artifact path is outside the controlled parent");
  }
  let stat;
  try {
    stat = fsOps.lstatSync(resolved);
  } catch (error) {
    if (allowMissing && error?.code === "ENOENT") {
      return comparableResolved;
    }
    throw error;
  }
  if (stat.isSymbolicLink() || !stat.isDirectory()) {
    throw new Error("Prepared artifact candidate must be a real directory");
  }
  const canonicalCandidate = fsOps.realpathSync(resolved);
  if (
    path.dirname(resolveKnownDarwinSystemPathAlias(canonicalCandidate)) !==
    comparableParent
  ) {
    throw new Error("Prepared artifact candidate escapes the controlled parent");
  }
  return canonicalCandidate;
}

function addActivePreparedRootAliases(activeRoots, candidatePath, fsOps) {
  if (
    typeof candidatePath !== "string" ||
    !path.isAbsolute(candidatePath)
  ) {
    return;
  }
  const resolved = path.resolve(candidatePath);
  activeRoots.add(resolved);
  activeRoots.add(resolveKnownDarwinSystemPathAlias(resolved));
  try {
    activeRoots.add(fsOps.realpathSync(resolved));
  } catch {}
}

function resolveKnownDarwinSystemPathAlias(targetPath) {
  // macOS exposes the system-owned /var alias through /private/var. Keep this
  // mapping explicit instead of realpath-ing an arbitrary candidate parent.
  if (
    process.platform === "darwin" &&
    (targetPath === "/var" || targetPath.startsWith("/var/"))
  ) {
    return `/private${targetPath}`;
  }
  return targetPath;
}

function readPreparedArtifactOwnerMarker(candidate, fsOps) {
  const markerPath = path.join(candidate, PREPARED_ARTIFACT_OWNER_FILE);
  const markerStat = fsOps.lstatSync(markerPath);
  if (markerStat.isSymbolicLink() || !markerStat.isFile()) {
    throw new Error("Prepared artifact owner marker is not a regular file");
  }
  const marker = JSON.parse(fsOps.readFileSync(markerPath, "utf8"));
  if (
    marker?.schemaVersion !== PREPARED_ARTIFACT_OWNER_SCHEMA_VERSION ||
    !normalizeString(marker?.transactionId) ||
    !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(
      normalizeString(marker?.ownerToken) ?? "",
    ) ||
    !Number.isSafeInteger(marker?.producerPid) ||
    marker.producerPid <= 0 ||
    !["published", "handedOff"].includes(marker?.state) ||
    (marker.state === "published" &&
      !Number.isFinite(Date.parse(marker.producerLeaseExpiresAt)))
  ) {
    throw new Error("Prepared artifact owner marker is invalid");
  }
  return marker;
}

function assertPreparedArtifactOwnerCapability(marker, expected) {
  if (
    !expected ||
    marker.ownerToken !== expected.ownerToken ||
    marker.transactionId !== expected.transactionId ||
    marker.producerPid !== expected.producerPid
  ) {
    throw new Error("Prepared artifact owner capability does not match");
  }
}

function preparedArtifactGcFileSystem(options) {
  return {
    chmodSync: options.chmodSync ?? fs.chmodSync,
    closeSync: options.closeSync ?? fs.closeSync,
    lstatSync: options.lstatSync ?? fs.lstatSync,
    mkdirSync: options.mkdirSync ?? fs.mkdirSync,
    openSync: options.openSync ?? fs.openSync,
    randomUUID: options.randomUUID ?? crypto.randomUUID,
    readFileSync: options.readFileSync ?? fs.readFileSync,
    readdirSync: options.readdirSync ?? fs.readdirSync,
    realpathSync: options.realpathSync ?? fs.realpathSync,
    renameSync: options.renameSync ?? fs.renameSync,
    rmSync: options.rmSync ?? fs.rmSync,
    writeFileSync: options.writeFileSync ?? fs.writeFileSync,
  };
}

function hasActiveProducerLease(marker, now, processIsAlive) {
  if (marker?.state !== "published") {
    return false;
  }
  const expiresAt = Date.parse(marker.producerLeaseExpiresAt);
  return (
    Number.isFinite(expiresAt) &&
    expiresAt > now &&
    processIsAlive(marker.producerPid)
  );
}

function isProcessAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error?.code === "EPERM";
  }
}

function activePreparedRootsFromTransaction(transaction) {
  const phase = normalizeString(transaction?.phase)?.toLowerCase();
  return phase === "prepared" &&
    typeof transaction?.request?.preparedRoot === "string"
    ? [transaction.request.preparedRoot]
    : [];
}

function readPreparedArtifactGcCursor(parentRoot, fsOps) {
  const cursorPath = path.join(
    parentRoot,
    PREPARED_ARTIFACT_GC_CURSOR_FILE,
  );
  try {
    const stat = fsOps.lstatSync(cursorPath);
    if (stat.isSymbolicLink() || !stat.isFile()) {
      throw new Error("Prepared artifact GC cursor must be a regular file");
    }
    return normalizeString(
      fsOps.readFileSync(cursorPath, "utf8"),
    );
  } catch (error) {
    if (error?.code === "ENOENT") {
      return null;
    }
    throw error;
  }
}

function writePreparedArtifactGcCursor(parentRoot, cursor, fsOps) {
  const cursorPath = path.join(
    parentRoot,
    PREPARED_ARTIFACT_GC_CURSOR_FILE,
  );
  try {
    const stat = fsOps.lstatSync(cursorPath);
    if (stat.isSymbolicLink() || !stat.isFile()) {
      throw new Error("Prepared artifact GC cursor must be a regular file");
    }
  } catch (error) {
    if (error?.code !== "ENOENT") {
      throw error;
    }
  }
  writeFileAtomicallyNoFollow({
    contents: `${cursor}\n`,
    directory: parentRoot,
    fsOps,
    randomUUID: fsOps.randomUUID,
    targetPath: cursorPath,
    temporaryPrefix: `${PREPARED_ARTIFACT_GC_CURSOR_FILE}.${process.pid}.`,
  });
}

function writeFileAtomicallyNoFollow({
  contents,
  directory,
  fsOps,
  randomUUID = crypto.randomUUID,
  targetPath,
  temporaryPrefix,
}) {
  const temporaryPath = path.join(
    directory,
    `${temporaryPrefix}${randomUUID()}`,
  );
  let created = false;
  let descriptor;
  try {
    descriptor = fsOps.openSync(temporaryPath, "wx", 0o600);
    created = true;
    fsOps.writeFileSync(descriptor, contents, { encoding: "utf8" });
    fsOps.closeSync(descriptor);
    descriptor = undefined;
    fsOps.renameSync(temporaryPath, targetPath);
    created = false;
  } finally {
    if (descriptor !== undefined) {
      try {
        fsOps.closeSync(descriptor);
      } catch {}
    }
    if (created) {
      try {
        fsOps.rmSync(temporaryPath, { force: true });
      } catch {}
    }
  }
}

function selectPreparedArtifactGcEntries(entries, cursor, limit) {
  if (entries.length === 0 || limit === 0) {
    return [];
  }
  const startIndex = cursor
    ? Math.max(
        0,
        entries.findIndex((entry) => entry.name > cursor),
      )
    : 0;
  const selected = [];
  for (
    let offset = 0;
    offset < Math.min(limit, entries.length);
    offset += 1
  ) {
    selected.push(entries[(startIndex + offset) % entries.length]);
  }
  return selected;
}

function canonicalizePreparedArtifactsRoot(parentRoot, fsOps) {
  fsOps.mkdirSync(parentRoot, { recursive: true, mode: 0o700 });
  let stat = fsOps.lstatSync(parentRoot);
  if (stat.isSymbolicLink() || !stat.isDirectory()) {
    throw new Error("Prepared artifact parent must be a real directory");
  }
  const currentUid =
    typeof process.getuid === "function" ? process.getuid() : null;
  if (
    currentUid !== null &&
    typeof stat.uid === "number" &&
    stat.uid !== currentUid
  ) {
    throw new Error("Prepared artifact parent is not owned by the current user");
  }
  if ((stat.mode & 0o077) !== 0) {
    fsOps.chmodSync(parentRoot, 0o700);
    stat = fsOps.lstatSync(parentRoot);
    if ((stat.mode & 0o077) !== 0) {
      throw new Error("Prepared artifact parent permissions are not private");
    }
  }
  return fsOps.realpathSync(parentRoot);
}

function pathExists(targetPath, fsOps) {
  try {
    fsOps.lstatSync(targetPath);
    return true;
  } catch (error) {
    if (error?.code === "ENOENT") {
      return false;
    }
    throw error;
  }
}

function artifactDescriptor(resourcesRoot, relativePath, kind, fsOps) {
  const artifactPath = path.join(resourcesRoot, relativePath);
  if (!fsOps.statSync(artifactPath).isFile()) {
    throw new Error(`Prepared artifact is not a file: ${artifactPath}`);
  }
  return {
    relativePath,
    sha256: sha256(fsOps.readFileSync(artifactPath)),
    kind,
  };
}

function packAppAsar(plan, appSourceRoot, appAsarPath, options = {}) {
  const spawn = options.spawnSync ?? spawnSync;
  const result = spawn(
    "pnpm",
    ["dlx", "@electron/asar", "pack", appSourceRoot, appAsarPath],
    {
      cwd: plan.workspace,
      encoding: "utf8",
      env: options.env ?? plan.commandEnv ?? process.env,
      stdio: options.stdio ?? "pipe",
    },
  );
  assertSuccessfulSpawn(result, "pnpm dlx @electron/asar pack");
}

function resolveSourceCommit(workspace, options = {}) {
  const spawn = options.spawnSync ?? spawnSync;
  const result = spawn("git", ["rev-parse", "HEAD"], {
    cwd: workspace,
    encoding: "utf8",
    env: options.env ?? process.env,
    stdio: "pipe",
  });
  assertSuccessfulSpawn(result, "git rev-parse HEAD");
  const sourceCommit = normalizeString(result.stdout);
  if (!sourceCommit) {
    throw new Error("git rev-parse HEAD returned an empty source commit");
  }
  return sourceCommit;
}

function assertPreparedArtifactSources(plan, options = {}) {
  const statSync = options.statSync ?? fs.statSync;
  assertPathType(statSync, plan.sourceAppDir, "directory", "source app");
  assertPathType(statSync, plan.frontendDistPath, "directory", "frontend dist");
  assertPathType(
    statSync,
    plan.appServerBinaryPath,
    "file",
    "release app-server binary",
  );
  assertPathType(
    statSync,
    plan.defaultCompactPromptSourcePath,
    "file",
    "default compact prompt",
  );
}

function assertPathType(statSync, targetPath, expectedType, label) {
  let stat;
  try {
    stat = statSync(targetPath);
  } catch (error) {
    throw new Error(`Missing ${label}: ${targetPath}`, { cause: error });
  }
  const matches =
    expectedType === "directory" ? stat.isDirectory() : stat.isFile();
  if (!matches) {
    throw new Error(`Expected ${label} to be a ${expectedType}: ${targetPath}`);
  }
}

function preparedAppSourceFilter(source) {
  const name = path.basename(source);
  return (
    name !== "dist-app" &&
    name !== "dist-package-resources" &&
    !name.startsWith(PREPARED_ARTIFACT_PREFIX)
  );
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

function resolveInstalledArtifactUpdatePlanInWorker(options = {}) {
  const env = options.env ?? { ...process.env };
  const commandEnv = {
    ...(options.commandEnv ??
      buildDesktopEnvironment(env, options.desktopEnvironmentOptions)),
  };
  return runInstalledArtifactWorker(
    "resolvePlan",
    {
      cargoCwd: options.cargoCwd,
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
        return;
      }
      settle(reject, new Error(message?.error?.message ?? "Artifact worker failed"));
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
  if (installedArtifactWorkerBundlePath) {
    return installedArtifactWorkerBundlePath;
  }
  const bundlePath = (options.mkdtempSync ?? fs.mkdtempSync)(
    path.join(os.tmpdir(), "morpheus-installed-artifact-worker-"),
  );
  try {
    for (const fileName of INSTALLED_ARTIFACT_WORKER_FILES) {
      (options.copyFileSync ?? fs.copyFileSync)(
        path.join(__dirname, fileName),
        path.join(bundlePath, fileName),
      );
    }
  } catch (error) {
    (options.rmSync ?? fs.rmSync)(bundlePath, { force: true, recursive: true });
    throw error;
  }
  installedArtifactWorkerBundlePath = bundlePath;
  process.once("exit", () => {
    try {
      fs.rmSync(bundlePath, { force: true, recursive: true });
    } catch {}
  });
  return bundlePath;
}

function sha256(buffer) {
  return crypto.createHash("sha256").update(buffer).digest("hex");
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
  DEFAULT_CONFIG_RELATIVE_PATH,
  PREPARED_ARTIFACT_SCHEMA_VERSION,
  PREPARED_ARTIFACT_OWNER_FILE,
  cleanupOwnedPreparedArtifactsWithLauncher,
  garbageCollectOwnedPreparedArtifacts,
  packAppAsar,
  prepareInstalledArtifacts,
  releaseOwnedPreparedArtifactLease,
  removeOwnedPreparedArtifact,
  resolvePreparedArtifactsRoot,
  listElectronShellSourceRelativePaths,
  listInstalledElectronShellRelativePaths,
  resolveElectronShellUpdate,
  resolveCargoTargetDirectory,
  resolveInstalledArtifactUpdatePlan,
  resolveInstalledArtifactUpdatePlanInWorker,
  materializeInstalledArtifactWorkerBundle,
  runInstalledArtifactWorker,
  updateInstalledArtifacts,
  updateInstalledArtifactsInWorker,
};
