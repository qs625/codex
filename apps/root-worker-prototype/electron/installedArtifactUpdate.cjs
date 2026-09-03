const fs = require("node:fs");
const crypto = require("node:crypto");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const {
  isPackagedApp,
  resolveDefaultWorkspace,
} = require("./workspace.cjs");

const APP_NAME = "Root Worker Prototype";
const APP_PLATFORM_DIR = "Root Worker Prototype-darwin-arm64";
const SOURCE_APP_RELATIVE_PATH = path.join("apps", "root-worker-prototype");
const APP_ASAR_RELATIVE_PATH = "app.asar";
const APP_SERVER_RELATIVE_PATH = path.join("bin", "app-server");
const APP_SERVER_BINARY_NAME = "app-server";
const DEFAULT_CONFIG_RELATIVE_PATH = "default-config";
const SIGNATURE_RELATIVE_PATH = path.join("Contents", "_CodeSignature");
const DIRECT_REFRESH_STAGING_PREFIX = "morpheus-runtime-refresh-";
const ELECTRON_SHELL_RELATIVE_DIR = "electron";

function resolveInstalledArtifactUpdatePlan({
  env = process.env,
  platform = process.platform,
  resourcesPath = currentResourcesPath(),
  workspace,
  appName = APP_NAME,
  appPlatformDir = APP_PLATFORM_DIR,
  isPackaged = isPackagedApp({ resourcesPath }),
  spawnSync: spawn = spawnSync,
} = {}) {
  if (platform !== "darwin" || !isPackaged || !resourcesPath) {
    return null;
  }

  const resolvedWorkspace =
    workspace ?? resolveDefaultWorkspace(env, { isPackagedApp: true });
  if (!resolvedWorkspace) {
    return null;
  }

  const appBundlePath = path.dirname(path.dirname(resourcesPath));
  const sourceAppDir = path.join(resolvedWorkspace, SOURCE_APP_RELATIVE_PATH);
  const codexRsDir = path.join(resolvedWorkspace, "codex-rs");
  const codexRsCargoManifestPath = path.join(codexRsDir, "Cargo.toml");
  const cargoTargetDir = resolveCargoTargetDirectory({
    codexRsCargoManifestPath,
    codexRsDir,
    spawnSync: spawn,
  });

  const shellUpdate = resolveElectronShellUpdate({
    resourcesPath,
    sourceAppDir,
  });

  return {
    appBundlePath,
    appServerBinaryPath: path.join(
      cargoTargetDir,
      "release",
      APP_SERVER_BINARY_NAME,
    ),
    codexRsCargoManifestPath,
    defaultCompactPromptSourcePath: path.join(
      codexRsDir,
      "thread-service",
      "templates",
      "compact",
      "prompt.md",
    ),
    frontendDistPath: path.join(sourceAppDir, "dist"),
    resourcesPath,
    requiresFullRelaunch: shellUpdate.changed,
    runtimeUpdate: {
      electronShell: shellUpdate,
    },
    sourceAppDir,
    workspace: resolvedWorkspace,
    artifacts: [
      { kind: "file", relativePath: APP_ASAR_RELATIVE_PATH },
      { kind: "file", relativePath: APP_SERVER_RELATIVE_PATH },
      { kind: "directory", relativePath: DEFAULT_CONFIG_RELATIVE_PATH },
    ],
  };
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
    const sourcePath = path.join(sourceAppDir, relativePath);
    const installedPath = path.join(
      resourcesPath,
      APP_ASAR_RELATIVE_PATH,
      relativePath,
    );
    let sourceDigest;
    let installedDigest;
    try {
      sourceDigest = bufferDigest(readFileSync(sourcePath));
    } catch {
      missingSourcePaths.push(relativePath);
      changedPaths.push(relativePath);
      continue;
    }
    try {
      installedDigest = bufferDigest(readFileSync(installedPath));
    } catch {
      missingInstalledPaths.push(relativePath);
      changedPaths.push(relativePath);
      continue;
    }
    if (sourceDigest !== installedDigest) {
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
    collectElectronShellSourceFiles({
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

function collectElectronShellSourceFiles({
  currentPath,
  electronRoot,
  relativePaths,
  readdirSync,
}) {
  for (const entry of readdirSync(currentPath, { withFileTypes: true })) {
    const entryPath = path.join(currentPath, entry.name);
    if (entry.isDirectory()) {
      collectElectronShellSourceFiles({
        currentPath: entryPath,
        electronRoot,
        relativePaths,
        readdirSync,
      });
      continue;
    }
    if (!entry.isFile() || !entry.name.endsWith(".cjs")) {
      continue;
    }
    if (entry.name.endsWith(".test.cjs")) {
      continue;
    }
    relativePaths.push(
      path.join(
        ELECTRON_SHELL_RELATIVE_DIR,
        path.relative(electronRoot, entryPath),
      ),
    );
  }
}

function resolveCargoTargetDirectory({
  codexRsCargoManifestPath,
  codexRsDir,
  spawnSync: spawn = spawnSync,
} = {}) {
  const result = spawn(
    "rtk",
    [
      "cargo",
      "metadata",
      "--format-version=1",
      "--no-deps",
      "--manifest-path",
      codexRsCargoManifestPath,
    ],
    {
      cwd: codexRsDir,
      encoding: "utf8",
      stdio: "pipe",
    },
  );
  assertSuccessfulSpawn(
    result,
    "rtk cargo metadata --format-version=1 --no-deps --manifest-path <Cargo.toml>",
    { cwd: codexRsDir },
  );

  let metadata;
  try {
    metadata = JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(
      `Failed to parse cargo metadata for app-server target directory${formatCause(error)}`,
    );
  }
  if (
    !metadata ||
    typeof metadata.target_directory !== "string" ||
    metadata.target_directory.length === 0
  ) {
    throw new Error("Cargo metadata did not include target_directory.");
  }
  return metadata.target_directory;
}

function updateInstalledArtifacts(plan, options = {}) {
  const spawn = options.spawnSync ?? spawnSync;
  const logger = options.logger ?? console;
  const prepareArtifacts = options.prepareArtifacts ?? prepareDirectArtifacts;
  const replaceArtifacts =
    options.replaceArtifacts ?? replaceInstalledArtifactsSync;
  const codesign = options.codesign ?? codesignInstalledApp;

  assertSourceWorkspace(plan, options);
  assertInstalledTargetsWritable(plan, options);

  const prepared = prepareArtifacts(plan, {
    ...options,
    spawnSync: spawn,
    logger,
  });
  const stagedPlan = {
    ...plan,
    stagedResourcesPath: prepared.stagedResourcesPath,
  };
  let replacement = null;
  let signatureBackup = null;
  try {
    assertStagedArtifacts(stagedPlan, options);
    replacement = replaceArtifacts(stagedPlan, {
      ...options,
      keepBackup: true,
      replacementWorkRoot: prepared.stagingRoot,
    });
    assertInstalledArtifactsMatchStaged(stagedPlan, options);
    signatureBackup = backupSignatureMetadataSync(stagedPlan, {
      ...options,
      fsOps: replacement.fsOps,
      signatureBackupRoot: prepared.stagingRoot,
      updateId: replacement.updateId,
    });
    codesign(stagedPlan, { spawnSync: spawn, logger });
  } catch (error) {
    if (replacement) {
      restoreBackups(stagedPlan, replacement.backupDir, replacement.fsOps);
    }
    if (signatureBackup) {
      restoreSignatureMetadataSync(signatureBackup);
    }
    throw error;
  } finally {
    if (signatureBackup) {
      cleanupSignatureBackupSync(signatureBackup);
    }
    if (replacement) {
      cleanupPath(replacement.backupDir, replacement.fsOps);
    }
    cleanupPath(prepared.stagingRoot, prepared.fsOps);
  }

  return {
    ok: true,
    updated: true,
    workspace: plan.workspace,
    appBundlePath: plan.appBundlePath,
  };
}

function prepareDirectArtifacts(plan, options = {}) {
  const fsOps = {
    cpSync: options.cpSync ?? fs.cpSync,
    mkdirSync: options.mkdirSync ?? fs.mkdirSync,
    rmSync: options.rmSync ?? fs.rmSync,
  };
  assertDirectArtifactSources(plan, options);
  const stagingRoot =
    options.directStagingRoot ??
    fs.mkdtempSync(path.join(os.tmpdir(), DIRECT_REFRESH_STAGING_PREFIX));
  const stagedResourcesPath = path.join(stagingRoot, "resources");
  const appSourceStagingPath = path.join(stagingRoot, "app-source");
  const stagedAppAsarPath = path.join(
    stagedResourcesPath,
    APP_ASAR_RELATIVE_PATH,
  );
  const stagedAppServerPath = path.join(
    stagedResourcesPath,
    APP_SERVER_RELATIVE_PATH,
  );
  const stagedDefaultCompactPath = path.join(
    stagedResourcesPath,
    DEFAULT_CONFIG_RELATIVE_PATH,
    "compact",
    "COMPACT.md",
  );

  fsOps.rmSync(stagingRoot, { force: true, recursive: true });
  try {
    fsOps.mkdirSync(path.dirname(stagedAppAsarPath), { recursive: true });
    fsOps.mkdirSync(path.dirname(stagedAppServerPath), { recursive: true });
    fsOps.mkdirSync(path.dirname(stagedDefaultCompactPath), { recursive: true });
    fsOps.cpSync(plan.sourceAppDir, appSourceStagingPath, {
      recursive: true,
      filter: directAppSourceFilter,
    });
    packAppAsar(plan, appSourceStagingPath, stagedAppAsarPath, options);
    fsOps.cpSync(plan.appServerBinaryPath, stagedAppServerPath);
    fsOps.cpSync(plan.defaultCompactPromptSourcePath, stagedDefaultCompactPath);
  } catch (error) {
    cleanupPath(stagingRoot, fsOps);
    throw error;
  }

  return {
    fsOps,
    stagedResourcesPath,
    stagingRoot,
  };
}

function packAppAsar(plan, appSourceStagingPath, stagedAppAsarPath, options = {}) {
  const spawn = options.spawnSync ?? spawnSync;
  const result = spawn(
    "rtk",
    [
      "pnpm",
      "dlx",
      "@electron/asar",
      "pack",
      appSourceStagingPath,
      stagedAppAsarPath,
    ],
    {
      cwd: plan.workspace,
      encoding: "utf8",
      stdio: options.stdio ?? "pipe",
    },
  );
  assertSuccessfulSpawn(
    result,
    "rtk pnpm dlx @electron/asar pack <source> <app.asar>",
    { cwd: plan.workspace },
  );
}

function codesignInstalledApp(plan, options = {}) {
  const spawn = options.spawnSync ?? spawnSync;
  const result = spawn(
    "rtk",
    ["codesign", "--force", "--deep", "--sign", "-", plan.appBundlePath],
    {
      cwd: plan.workspace,
      encoding: "utf8",
      stdio: options.stdio ?? "pipe",
    },
  );
  assertSuccessfulSpawn(result, "rtk codesign --force --deep --sign - <app>", {
    cwd: plan.workspace,
  });
}

function assertSuccessfulSpawn(result, label, context = {}) {
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    const stdout = result.stdout ? String(result.stdout).trim() : "";
    const stderr = result.stderr ? String(result.stderr).trim() : "";
    const details = [
      `exited with ${result.status}`,
      context.cwd ? `cwd=${context.cwd}` : null,
      stdout ? `stdout=${truncateOutput(stdout)}` : null,
      stderr ? `stderr=${truncateOutput(stderr)}` : null,
    ]
      .filter(Boolean)
      .join("; ");
    throw new Error(`${label} ${details}`);
  }
}

function truncateOutput(value, maxLength = 4000) {
  if (value.length <= maxLength) {
    return value;
  }
  return `${value.slice(0, maxLength)}...<truncated>`;
}

function assertDirectArtifactSources(plan, options = {}) {
  const statSync = options.statSync ?? fs.statSync;
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
    throw new Error(`Missing ${label}: ${targetPath}${formatCause(error)}`);
  }
  const matches =
    expectedType === "directory" ? stat.isDirectory() : stat.isFile();
  if (!matches) {
    throw new Error(`Expected ${label} to be a ${expectedType}: ${targetPath}`);
  }
}

function directAppSourceFilter(source) {
  const name = path.basename(source);
  return (
    name !== "dist-app" &&
    name !== "dist-package-resources" &&
    !name.startsWith(DIRECT_REFRESH_STAGING_PREFIX)
  );
}

function assertSourceWorkspace(plan, options = {}) {
  const statSync = options.statSync ?? fs.statSync;
  if (!statSync(plan.sourceAppDir).isDirectory()) {
    throw new Error(
      `Morpheus source app directory is missing: ${plan.sourceAppDir}`,
    );
  }
}

function assertInstalledTargetsWritable(plan, options = {}) {
  const accessSync = options.accessSync ?? fs.accessSync;
  const constants = options.constants ?? fs.constants;
  accessSync(plan.resourcesPath, constants.W_OK);
  accessSync(plan.appBundlePath, constants.W_OK);
  for (const artifact of plan.artifacts) {
    accessSync(
      path.join(plan.resourcesPath, artifact.relativePath),
      constants.W_OK,
    );
  }
}

function assertStagedArtifacts(plan, options = {}) {
  const statSync = options.statSync ?? fs.statSync;
  for (const artifact of plan.artifacts) {
    const artifactPath = path.join(
      plan.stagedResourcesPath,
      artifact.relativePath,
    );
    const stat = statSync(artifactPath);
    if (artifact.kind === "directory" ? !stat.isDirectory() : !stat.isFile()) {
      throw new Error(
        `Prepared refresh artifact has unexpected type: ${artifactPath}`,
      );
    }
  }
}

function assertInstalledArtifactsMatchStaged(plan, options = {}) {
  const statSync = options.statSync ?? fs.statSync;
  for (const artifact of plan.artifacts) {
    const stagedPath = path.join(plan.stagedResourcesPath, artifact.relativePath);
    const installedPath = path.join(plan.resourcesPath, artifact.relativePath);
    if (artifact.kind === "directory") {
      assertDirectoryDigestsEqual(statSync, stagedPath, installedPath);
      continue;
    }
    assertFileDigestsEqual(stagedPath, installedPath, options);
  }
}

function assertDirectoryDigestsEqual(statSync, stagedPath, installedPath) {
  const stagedFiles = listDirectoryFiles(stagedPath);
  const installedFiles = listDirectoryFiles(installedPath);
  if (JSON.stringify(installedFiles) !== JSON.stringify(stagedFiles)) {
    throw new Error(
      `Installed directory files differ from staged artifact: ${installedPath}`,
    );
  }
  for (const relativePath of stagedFiles) {
    const stagedFile = path.join(stagedPath, relativePath);
    const installedFile = path.join(installedPath, relativePath);
    assertPathType(statSync, installedFile, "file", "installed artifact file");
    assertFileDigestsEqual(stagedFile, installedFile);
  }
}

function assertFileDigestsEqual(stagedPath, installedPath, options = {}) {
  const stagedHash = fileDigest(stagedPath, options);
  const installedHash = fileDigest(installedPath, options);
  if (stagedHash !== installedHash) {
    throw new Error(
      `Installed artifact does not match staged artifact: ${installedPath}`,
    );
  }
}

function fileDigest(filePath, options = {}) {
  const readFileSync = options.readFileSync ?? fs.readFileSync;
  return bufferDigest(readFileSync(filePath));
}

function bufferDigest(buffer) {
  return crypto.createHash("sha256").update(buffer).digest("hex");
}

function listDirectoryFiles(rootPath) {
  const files = [];
  collectDirectoryFiles(rootPath, rootPath, files);
  return files.sort();
}

function collectDirectoryFiles(rootPath, currentPath, files) {
  for (const entry of fs.readdirSync(currentPath, { withFileTypes: true })) {
    const entryPath = path.join(currentPath, entry.name);
    if (entry.isDirectory()) {
      collectDirectoryFiles(rootPath, entryPath, files);
      continue;
    }
    if (entry.isFile()) {
      files.push(path.relative(rootPath, entryPath));
    }
  }
}

function formatCause(error) {
  const message = error instanceof Error ? error.message : String(error);
  return message ? ` (${message})` : "";
}

function replaceInstalledArtifactsSync(plan, options = {}) {
  const fsOps = {
    cpSync: options.cpSync ?? fs.cpSync,
    mkdirSync: options.mkdirSync ?? fs.mkdirSync,
    renameSync: options.renameSync ?? fs.renameSync,
    rmSync: options.rmSync ?? fs.rmSync,
  };
  const updateId =
    options.updateId ??
    `${process.pid}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  const workRoot = options.replacementWorkRoot ?? os.tmpdir();
  const stagingDir = path.join(
    workRoot,
    `.morpheus-update-staging-${updateId}`,
  );
  const backupDir = path.join(
    workRoot,
    `.morpheus-update-backup-${updateId}`,
  );

  cleanupPath(stagingDir, fsOps);
  cleanupPath(backupDir, fsOps);
  fsOps.mkdirSync(stagingDir, { recursive: true });
  fsOps.mkdirSync(backupDir, { recursive: true });

  let completed = false;
  const backedUpArtifacts = [];
  try {
    for (const artifact of plan.artifacts) {
      fsOps.cpSync(
        path.join(plan.stagedResourcesPath, artifact.relativePath),
        path.join(stagingDir, artifact.relativePath),
        { recursive: artifact.kind === "directory" },
      );
    }

    for (const artifact of plan.artifacts) {
      fsOps.mkdirSync(path.dirname(path.join(backupDir, artifact.relativePath)), {
        recursive: true,
      });
      try {
        fsOps.renameSync(
          path.join(plan.resourcesPath, artifact.relativePath),
          path.join(backupDir, artifact.relativePath),
        );
        backedUpArtifacts.push(artifact);
      } catch (error) {
        restoreBackups(plan, backupDir, fsOps, backedUpArtifacts);
        throw error;
      }
    }

    try {
      for (const artifact of plan.artifacts) {
        fsOps.mkdirSync(
          path.dirname(path.join(plan.resourcesPath, artifact.relativePath)),
          {
            recursive: true,
          },
        );
        fsOps.renameSync(
          path.join(stagingDir, artifact.relativePath),
          path.join(plan.resourcesPath, artifact.relativePath),
        );
      }
    } catch (error) {
      restoreBackups(plan, backupDir, fsOps, backedUpArtifacts);
      throw error;
    }
    completed = true;
  } finally {
    cleanupPath(stagingDir, fsOps);
    if (!options.keepBackup || !completed) {
      cleanupPath(backupDir, fsOps);
    }
  }

  return { backupDir, fsOps, stagingDir, updateId };
}

function restoreBackups(plan, backupDir, fsOps, artifacts = plan.artifacts) {
  for (const artifact of artifacts) {
    const target = path.join(plan.resourcesPath, artifact.relativePath);
    const backup = path.join(backupDir, artifact.relativePath);
    cleanupPath(target, fsOps);
    try {
      fsOps.renameSync(backup, target);
    } catch {
      // Best-effort rollback; the original error is more useful to callers.
    }
  }
}

function backupSignatureMetadataSync(plan, options = {}) {
  const fsOps = options.fsOps ?? {
    cpSync: options.cpSync ?? fs.cpSync,
    mkdirSync: options.mkdirSync ?? fs.mkdirSync,
    renameSync: options.renameSync ?? fs.renameSync,
    rmSync: options.rmSync ?? fs.rmSync,
  };
  const existsSync = options.existsSync ?? fs.existsSync;
  const signaturePath = path.join(plan.appBundlePath, SIGNATURE_RELATIVE_PATH);
  const backupRoot =
    options.signatureBackupRoot ??
    path.join(
      os.tmpdir(),
      `${DIRECT_REFRESH_STAGING_PREFIX}signature-${options.updateId ?? "current"}`,
    );
  const backupPath = path.join(
    backupRoot,
    `.morpheus-signature-backup-${options.updateId ?? "current"}`,
  );
  cleanupPath(backupPath, fsOps);
  if (!existsSync(signaturePath)) {
    return { backupPath, existed: false, fsOps, signaturePath };
  }
  fsOps.mkdirSync(path.dirname(backupPath), { recursive: true });
  fsOps.cpSync(signaturePath, backupPath, { recursive: true });
  return { backupPath, existed: true, fsOps, signaturePath };
}

function restoreSignatureMetadataSync(signatureBackup) {
  cleanupPath(signatureBackup.signaturePath, signatureBackup.fsOps);
  if (signatureBackup.existed) {
    signatureBackup.fsOps.renameSync(
      signatureBackup.backupPath,
      signatureBackup.signaturePath,
    );
  }
}

function cleanupSignatureBackupSync(signatureBackup) {
  cleanupPath(signatureBackup.backupPath, signatureBackup.fsOps);
}

function cleanupPath(targetPath, fsOps) {
  fsOps.rmSync(targetPath, { recursive: true, force: true });
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
  SIGNATURE_RELATIVE_PATH,
  backupSignatureMetadataSync,
  packAppAsar,
  prepareDirectArtifacts,
  listInstalledElectronShellRelativePaths,
  listElectronShellSourceRelativePaths,
  resolveElectronShellUpdate,
  resolveCargoTargetDirectory,
  resolveInstalledArtifactUpdatePlan,
  updateInstalledArtifacts,
  replaceInstalledArtifactsSync,
  restoreSignatureMetadataSync,
};
