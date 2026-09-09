const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const APP_NAME = "Root Worker Prototype";
const DIST_DIR_NAME = "dist-app";
const RESOURCE_STAGING_DIR_NAME = "dist-package-resources";

const PLATFORM_CONFIGS = {
  linux: {
    electronPlatform: "linux",
    arch: "x64",
    appServerBinaryName: "app-server",
    appPlatformDir: "Root Worker Prototype-linux-x64",
    artifactName: "Root Worker Prototype-linux-x64.tar.gz",
  },
  win32: {
    electronPlatform: "win32",
    arch: "x64",
    appServerBinaryName: "app-server.exe",
    appPlatformDir: "Root Worker Prototype-win32-x64",
    artifactName: "Root Worker Prototype-win32-x64.zip",
  },
};

function buildPlatformPackagePlan({
  cwd = process.cwd(),
  platform = process.platform,
  distDirName = DIST_DIR_NAME,
  resourceStagingDirName = RESOURCE_STAGING_DIR_NAME,
} = {}) {
  const config = platformConfig(platform);
  const repoRoot = path.resolve(cwd, "..", "..");
  const codexRsDir = path.join(repoRoot, "codex-rs");
  const resourceStagingDir = path.join(cwd, resourceStagingDirName);
  const binResourceDir = path.join(resourceStagingDir, "bin");
  const defaultConfigResourceDir = path.join(resourceStagingDir, "default-config");
  const defaultCompactResourceDir = path.join(defaultConfigResourceDir, "compact");
  return {
    appBundlePath: path.join(cwd, distDirName, config.appPlatformDir),
    appServerBinaryPath: path.join(
      codexRsDir,
      "target",
      "release",
      config.appServerBinaryName,
    ),
    appServerResourcePath: path.join(binResourceDir, config.appServerBinaryName),
    artifactPath: path.join(cwd, distDirName, config.artifactName),
    binResourceDir,
    codexRsCargoManifestPath: path.join(codexRsDir, "Cargo.toml"),
    defaultCompactPromptResourcePath: path.join(
      defaultCompactResourceDir,
      "COMPACT.md",
    ),
    defaultCompactPromptSourcePath: path.join(
      codexRsDir,
      "thread-service",
      "templates",
      "compact",
      "prompt.md",
    ),
    defaultConfigResourceDir,
    distDir: path.join(cwd, distDirName),
    electronArch: config.arch,
    electronPlatform: config.electronPlatform,
    repoRoot,
    resourceStagingDir,
  };
}

function buildElectronPackagerArgs({
  appName = APP_NAME,
  cwd = process.cwd(),
  distDirName = DIST_DIR_NAME,
  electronArch,
  electronPlatform,
  binResourceDir,
  defaultConfigResourceDir,
} = {}) {
  return [
    ".",
    appName,
    `--platform=${electronPlatform}`,
    `--arch=${electronArch}`,
    `--out=${distDirName}`,
    "--overwrite",
    "--ignore=^/dist-app($|/)",
    "--ignore=^/dist-package-resources($|/)",
    "--no-prune",
    `--extra-resource=${path.relative(cwd, binResourceDir)}`,
    `--extra-resource=${path.relative(cwd, defaultConfigResourceDir)}`,
  ];
}

function preparePlatformPackageResources(plan) {
  fs.rmSync(plan.resourceStagingDir, { force: true, recursive: true });
  fs.mkdirSync(plan.binResourceDir, { recursive: true });
  fs.mkdirSync(path.dirname(plan.defaultCompactPromptResourcePath), {
    recursive: true,
  });
  fs.copyFileSync(plan.appServerBinaryPath, plan.appServerResourcePath);
  fs.copyFileSync(
    plan.defaultCompactPromptSourcePath,
    plan.defaultCompactPromptResourcePath,
  );
}

function buildArchiveCommand(plan) {
  if (plan.electronPlatform === "win32") {
    return {
      command: "powershell",
      args: [
        "-NoProfile",
        "-Command",
        `Compress-Archive -Path '${escapePowerShellPath(path.join(plan.appBundlePath, "*"))}' -DestinationPath '${escapePowerShellPath(plan.artifactPath)}' -Force`,
      ],
    };
  }
  if (plan.electronPlatform === "linux") {
    return {
      command: "tar",
      args: [
        "-czf",
        plan.artifactPath,
        "-C",
        plan.distDir,
        path.basename(plan.appBundlePath),
      ],
    };
  }
  throw new Error(`Unsupported desktop archive platform: ${plan.electronPlatform}`);
}

function packagePlatformApp({
  cwd = process.cwd(),
  platform = process.platform,
} = {}) {
  assertSupportedPackagingPlatform(platform, process.platform);
  const plan = buildPlatformPackagePlan({ cwd, platform });
  fs.rmSync(plan.distDir, { force: true, recursive: true });
  run("rtk", ["pnpm", "build"], { cwd });
  run(
    "rtk",
    [
      "cargo",
      "build",
      "--manifest-path",
      path.relative(cwd, plan.codexRsCargoManifestPath),
      "-p",
      "app-server",
      "--bin",
      "app-server",
      "--release",
    ],
    { cwd },
  );
  preparePlatformPackageResources(plan);
  run(
    "rtk",
    [
      "pnpm",
      "dlx",
      "@electron/packager",
      ...buildElectronPackagerArgs({
        cwd,
        electronArch: plan.electronArch,
        electronPlatform: plan.electronPlatform,
        binResourceDir: plan.binResourceDir,
        defaultConfigResourceDir: plan.defaultConfigResourceDir,
      }),
    ],
    { cwd },
  );
  const archive = buildArchiveCommand(plan);
  run(archive.command, archive.args, { cwd });
  return plan;
}

function assertSupportedPackagingPlatform(targetPlatform, hostPlatform) {
  if (!PLATFORM_CONFIGS[targetPlatform]) {
    throw new Error(`Unsupported desktop packaging platform: ${targetPlatform}`);
  }
  if (targetPlatform !== hostPlatform) {
    throw new Error(
      `${targetPlatform} desktop packaging must run on a ${targetPlatform} runner.`,
    );
  }
}

function platformConfig(platform) {
  const config = PLATFORM_CONFIGS[platform];
  if (!config) {
    throw new Error(`Unsupported desktop packaging platform: ${platform}`);
  }
  return config;
}

function escapePowerShellPath(value) {
  return value.replaceAll("'", "''");
}

function run(command, args, options) {
  const invocation = buildRunInvocation(command, args);
  const result = spawnSync(invocation.command, invocation.args, {
    cwd: options.cwd,
    stdio: "inherit",
    ...invocation.spawnOptions,
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} exited with ${result.status}`);
  }
}

function buildRunInvocation(command, args, platform = process.platform) {
  return {
    command,
    args,
    spawnOptions: command === "rtk" && platform === "win32" ? { shell: true } : {},
  };
}

if (require.main === module) {
  packagePlatformApp();
}

module.exports = {
  assertSupportedPackagingPlatform,
  buildArchiveCommand,
  buildElectronPackagerArgs,
  buildPlatformPackagePlan,
  buildRunInvocation,
  escapePowerShellPath,
  packagePlatformApp,
  preparePlatformPackageResources,
};
