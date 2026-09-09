const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const APP_NAME = "Root Worker Prototype";
const APP_PLATFORM_DIR = "Root Worker Prototype-darwin-arm64";
const HOST_EXECUTABLE_NAME = "Root Worker Runtime";
const LAUNCHER_EXECUTABLE_NAME = "MorpheusLauncher";
const DIST_DIR_NAME = "dist-app";
const RESOURCE_STAGING_DIR_NAME = "dist-package-resources";

function buildMacAppPackagePlan({
  cwd = process.cwd(),
  appName = APP_NAME,
  distDirName = DIST_DIR_NAME,
  resourceStagingDirName = RESOURCE_STAGING_DIR_NAME,
} = {}) {
  const repoRoot = path.resolve(cwd, "..", "..");
  const codexRsDir = path.join(repoRoot, "codex-rs");
  const resourceStagingDir = path.join(cwd, resourceStagingDirName);
  const binResourceDir = path.join(resourceStagingDir, "bin");
  const defaultConfigResourceDir = path.join(resourceStagingDir, "default-config");
  const defaultCompactResourceDir = path.join(
    defaultConfigResourceDir,
    "compact",
  );
  const appBundlePath = path.join(
    cwd,
    distDirName,
    APP_PLATFORM_DIR,
    `${appName}.app`,
  );
  const macOsDir = path.join(appBundlePath, "Contents", "MacOS");
  return {
    appBundlePath,
    appBundleInfoPlistPath: path.join(appBundlePath, "Contents", "Info.plist"),
    appBundlePackagerExecutablePath: path.join(macOsDir, appName),
    appServerBinaryPath: path.join(
      codexRsDir,
      "target",
      "release",
      "app-server",
    ),
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
    hostExecutablePath: path.join(macOsDir, HOST_EXECUTABLE_NAME),
    launcherBinaryPath: path.join(
      codexRsDir,
      "target",
      "release",
      LAUNCHER_EXECUTABLE_NAME,
    ),
    launcherExecutablePath: path.join(macOsDir, LAUNCHER_EXECUTABLE_NAME),
    repoRoot,
    resourceStagingDir,
    runtimeManifestPath: path.join(
      appBundlePath,
      "Contents",
      "Resources",
      ".morpheus-runtime-manifest.json",
    ),
  };
}

function buildElectronPackagerArgs({
  cwd = process.cwd(),
  appName = APP_NAME,
  distDirName = DIST_DIR_NAME,
  binResourceDir,
  defaultConfigResourceDir,
} = {}) {
  return [
    ".",
    appName,
    "--platform=darwin",
    "--arch=arm64",
    `--out=${distDirName}`,
    "--overwrite",
    "--app-bundle-id=com.openai.root-worker-prototype.dev",
    "--app-category-type=public.app-category.developer-tools",
    "--extend-info=electron/Info.plist",
    "--asar",
    "--ignore=^/dist-app($|/)",
    "--ignore=^/dist-package-resources($|/)",
    "--no-prune",
    `--extra-resource=${path.relative(cwd, binResourceDir)}`,
    `--extra-resource=${path.relative(cwd, defaultConfigResourceDir)}`,
  ];
}

function prepareMacAppResources(plan) {
  fs.rmSync(plan.resourceStagingDir, { force: true, recursive: true });
  fs.mkdirSync(plan.binResourceDir, { recursive: true });
  fs.mkdirSync(path.dirname(plan.defaultCompactPromptResourcePath), {
    recursive: true,
  });
  fs.copyFileSync(
    plan.appServerBinaryPath,
    path.join(plan.binResourceDir, "app-server"),
  );
  fs.copyFileSync(
    plan.defaultCompactPromptSourcePath,
    plan.defaultCompactPromptResourcePath,
  );
}

function installMacRuntimeExecutables(plan, fsOps = fs) {
  fsOps.renameSync(
    plan.appBundlePackagerExecutablePath,
    plan.hostExecutablePath,
  );
  fsOps.copyFileSync(plan.launcherBinaryPath, plan.launcherExecutablePath);
  fsOps.chmodSync(plan.hostExecutablePath, 0o755);
  fsOps.chmodSync(plan.launcherExecutablePath, 0o755);
}

function writeInstalledRuntimeManifest(
  plan,
  { sourceCommit, fsOps = fs } = {},
) {
  if (typeof sourceCommit !== "string" || !sourceCommit.trim()) {
    throw new Error("Packaged runtime manifest requires a source commit");
  }
  const resourcesRoot = path.join(plan.appBundlePath, "Contents", "Resources");
  const relativePaths = [
    "app.asar",
    path.join("bin", "app-server"),
    path.join("default-config", "compact", "COMPACT.md"),
  ];
  const artifacts = relativePaths.map((relativePath) => ({
    relativePath,
    sha256: crypto
      .createHash("sha256")
      .update(fsOps.readFileSync(path.join(resourcesRoot, relativePath)))
      .digest("hex"),
  }));
  const buildId = crypto
    .createHash("sha256")
    .update(JSON.stringify({ sourceCommit, artifacts }))
    .digest("hex")
    .slice(0, 24);
  const manifest = {
    schemaVersion: 1,
    buildId,
    sourceCommit,
    entrypoint: "app.asar",
    artifacts,
  };
  fsOps.writeFileSync(
    plan.runtimeManifestPath,
    `${JSON.stringify(manifest, null, 2)}\n`,
    { encoding: "utf8", mode: 0o600 },
  );
  return manifest;
}

function assertMacRuntimeLayout(plan, fsOps = fs) {
  for (const targetPath of [
    plan.hostExecutablePath,
    plan.launcherExecutablePath,
    plan.runtimeManifestPath,
    path.join(plan.appBundlePath, "Contents", "Resources", "app.asar"),
    path.join(
      plan.appBundlePath,
      "Contents",
      "Resources",
      "bin",
      "app-server",
    ),
    path.join(
      plan.appBundlePath,
      "Contents",
      "Resources",
      "default-config",
      "compact",
      "COMPACT.md",
    ),
  ]) {
    if (!fsOps.statSync(targetPath).isFile()) {
      throw new Error(`Required packaged runtime artifact is missing: ${targetPath}`);
    }
  }
  if (fsOps.existsSync(plan.appBundlePackagerExecutablePath)) {
    throw new Error(
      `Packager executable was not renamed: ${plan.appBundlePackagerExecutablePath}`,
    );
  }
}

function finalizeMacRuntimeBundle(
  plan,
  { fsOps = fs, runCommand = run, sourceCommit } = {},
) {
  const cwd = path.dirname(plan.distDir);
  const appServerPath = path.join(
    plan.appBundlePath,
    "Contents",
    "Resources",
    "bin",
    "app-server",
  );
  runCommand(
    "codesign",
    ["--force", "--sign", "-", appServerPath],
    { cwd },
  );
  const manifest = writeInstalledRuntimeManifest(plan, {
    fsOps,
    sourceCommit,
  });
  runCommand(
    "/usr/libexec/PlistBuddy",
    [
      "-c",
      `Set :CFBundleExecutable ${LAUNCHER_EXECUTABLE_NAME}`,
      plan.appBundleInfoPlistPath,
    ],
    { cwd },
  );
  assertMacRuntimeLayout(plan, fsOps);
  runCommand(
    "codesign",
    ["--force", "--deep", "--sign", "-", plan.appBundlePath],
    { cwd },
  );
  runCommand(
    "codesign",
    ["--verify", "--deep", "--strict", plan.appBundlePath],
    { cwd },
  );
  return manifest;
}

function packageMacApp({ cwd = process.cwd(), platform = process.platform } = {}) {
  if (platform !== "darwin") {
    throw new Error("macOS app packaging requires codesign and must run on macOS.");
  }
  const plan = buildMacAppPackagePlan({ cwd });
  fs.rmSync(plan.distDir, { force: true, recursive: true });
  run("pnpm", ["build"], { cwd });
  run(
    "cargo",
    [
      "build",
      "--manifest-path",
      path.relative(cwd, plan.codexRsCargoManifestPath),
      "-p",
      "app-server",
      "--bin",
      "app-server",
      "-p",
      "runtime-launcher",
      "--bin",
      LAUNCHER_EXECUTABLE_NAME,
      "--release",
    ],
    { cwd },
  );
  prepareMacAppResources(plan);
  run(
    "pnpm",
    [
      "dlx",
      "@electron/packager",
      ...buildElectronPackagerArgs({
        cwd,
        binResourceDir: plan.binResourceDir,
        defaultConfigResourceDir: plan.defaultConfigResourceDir,
      }),
    ],
    { cwd },
  );
  installMacRuntimeExecutables(plan);
  const sourceCommit = capture("git", ["rev-parse", "HEAD"], { cwd }).trim();
  finalizeMacRuntimeBundle(plan, { sourceCommit });
}

function capture(command, args, options) {
  const result = spawnSync(command, args, {
    cwd: options.cwd,
    encoding: "utf8",
    stdio: "pipe",
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} exited with ${result.status}`);
  }
  return result.stdout;
}

function run(command, args, options) {
  const result = spawnSync(command, args, {
    cwd: options.cwd,
    stdio: "inherit",
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} exited with ${result.status}`);
  }
}

if (require.main === module) {
  packageMacApp();
}

module.exports = {
  assertMacRuntimeLayout,
  buildElectronPackagerArgs,
  buildMacAppPackagePlan,
  finalizeMacRuntimeBundle,
  installMacRuntimeExecutables,
  packageMacApp,
  prepareMacAppResources,
  writeInstalledRuntimeManifest,
};
