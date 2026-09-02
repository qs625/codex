const fs = require("node:fs");
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
const DEFAULT_CONFIG_RELATIVE_PATH = "default-config";
const SIGNATURE_RELATIVE_PATH = path.join("Contents", "_CodeSignature");

function resolveInstalledArtifactUpdatePlan({
  env = process.env,
  platform = process.platform,
  resourcesPath = currentResourcesPath(),
  workspace,
  appName = APP_NAME,
  appPlatformDir = APP_PLATFORM_DIR,
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

  const appBundlePath = path.dirname(path.dirname(resourcesPath));
  const sourceAppDir = path.join(resolvedWorkspace, SOURCE_APP_RELATIVE_PATH);
  const stagedAppBundlePath = path.join(
    sourceAppDir,
    "dist-app",
    appPlatformDir,
    `${appName}.app`,
  );
  const stagedResourcesPath = path.join(
    stagedAppBundlePath,
    "Contents",
    "Resources",
  );

  return {
    appBundlePath,
    resourcesPath,
    sourceAppDir,
    stagedAppBundlePath,
    stagedResourcesPath,
    workspace: resolvedWorkspace,
    artifacts: [
      { kind: "file", relativePath: APP_ASAR_RELATIVE_PATH },
      { kind: "file", relativePath: APP_SERVER_RELATIVE_PATH },
      { kind: "directory", relativePath: DEFAULT_CONFIG_RELATIVE_PATH },
    ],
  };
}

function updateInstalledArtifacts(plan, options = {}) {
  const spawn = options.spawnSync ?? spawnSync;
  const logger = options.logger ?? console;
  const runPackage = options.runPackage ?? runPackageMacApp;
  const replaceArtifacts =
    options.replaceArtifacts ?? replaceInstalledArtifactsSync;
  const codesign = options.codesign ?? codesignInstalledApp;

  assertSourceWorkspace(plan, options);
  assertInstalledTargetsWritable(plan, options);

  runPackage(plan, { spawnSync: spawn, logger });
  assertStagedArtifacts(plan, options);
  const replacement = replaceArtifacts(plan, {
    ...options,
    keepBackup: true,
  });
  let signatureBackup = null;
  try {
    signatureBackup = backupSignatureMetadataSync(plan, {
      ...options,
      fsOps: replacement.fsOps,
      updateId: replacement.updateId,
    });
    codesign(plan, { spawnSync: spawn, logger });
  } catch (error) {
    restoreBackups(plan, replacement.backupDir, replacement.fsOps);
    if (signatureBackup) {
      restoreSignatureMetadataSync(signatureBackup);
    }
    throw error;
  } finally {
    if (signatureBackup) {
      cleanupSignatureBackupSync(signatureBackup);
    }
    cleanupPath(replacement.backupDir, replacement.fsOps);
  }

  return {
    ok: true,
    updated: true,
    workspace: plan.workspace,
    appBundlePath: plan.appBundlePath,
  };
}

function runPackageMacApp(plan, options = {}) {
  const spawn = options.spawnSync ?? spawnSync;
  const result = spawn(
    "rtk",
    ["pnpm", "--dir", plan.sourceAppDir, "package:mac:app"],
    {
      cwd: plan.workspace,
      encoding: "utf8",
      stdio: options.stdio ?? "pipe",
    },
  );
  assertSuccessfulSpawn(
    result,
    "rtk pnpm --dir apps/root-worker-prototype package:mac:app",
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
  assertSuccessfulSpawn(result, "rtk codesign --force --deep --sign - <app>");
}

function assertSuccessfulSpawn(result, label) {
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    const stderr = result.stderr ? String(result.stderr).trim() : "";
    throw new Error(
      `${label} exited with ${result.status}${stderr ? `: ${stderr}` : ""}`,
    );
  }
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
      throw new Error(`Packaged artifact has unexpected type: ${artifactPath}`);
    }
  }
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
  const stagingDir = path.join(
    plan.resourcesPath,
    `.morpheus-update-staging-${updateId}`,
  );
  const backupDir = path.join(
    plan.resourcesPath,
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
  const backupPath = path.join(
    plan.appBundlePath,
    "Contents",
    `.morpheus-signature-backup-${options.updateId ?? "current"}`,
  );
  cleanupPath(backupPath, fsOps);
  if (!existsSync(signaturePath)) {
    return { backupPath, existed: false, fsOps, signaturePath };
  }
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
  resolveInstalledArtifactUpdatePlan,
  runPackageMacApp,
  updateInstalledArtifacts,
  replaceInstalledArtifactsSync,
  restoreSignatureMetadataSync,
};
