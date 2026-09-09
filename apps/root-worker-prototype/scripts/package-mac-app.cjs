const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const APP_NAME = "Root Worker Prototype";
const APP_PLATFORM_DIR = "Root Worker Prototype-darwin-arm64";
const DIST_DIR_NAME = "dist-app";
const RESOURCE_STAGING_DIR_NAME = "dist-package-resources";
const LAUNCHER_EXECUTABLE_NAME = "MorpheusLauncher";
const RUNTIME_EXECUTABLE_NAME = "Root Worker Runtime";

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
  return {
    appBundlePath: path.join(
      cwd,
      distDirName,
      APP_PLATFORM_DIR,
      `${appName}.app`,
    ),
    appServerBinaryPath: path.join(
      codexRsDir,
      "target",
      "release",
      "app-server",
    ),
    launcherBinaryPath: path.join(
      codexRsDir,
      "target",
      "release",
      "morpheus-runtime-launcher",
    ),
    launcherExecutablePath: path.join(
      cwd,
      distDirName,
      APP_PLATFORM_DIR,
      `${appName}.app`,
      "Contents",
      "MacOS",
      LAUNCHER_EXECUTABLE_NAME,
    ),
    packagedElectronExecutablePath: path.join(
      cwd,
      distDirName,
      APP_PLATFORM_DIR,
      `${appName}.app`,
      "Contents",
      "MacOS",
      appName,
    ),
    runtimeExecutablePath: path.join(
      cwd,
      distDirName,
      APP_PLATFORM_DIR,
      `${appName}.app`,
      "Contents",
      "MacOS",
      RUNTIME_EXECUTABLE_NAME,
    ),
    infoPlistPath: path.join(
      cwd,
      distDirName,
      APP_PLATFORM_DIR,
      `${appName}.app`,
      "Contents",
      "Info.plist",
    ),
    packagedAppAsarPath: path.join(
      cwd,
      distDirName,
      APP_PLATFORM_DIR,
      `${appName}.app`,
      "Contents",
      "Resources",
      "app.asar",
    ),
    packagedAppServerPath: path.join(
      cwd,
      distDirName,
      APP_PLATFORM_DIR,
      `${appName}.app`,
      "Contents",
      "Resources",
      "bin",
      "app-server",
    ),
    packagedCompactPromptPath: path.join(
      cwd,
      distDirName,
      APP_PLATFORM_DIR,
      `${appName}.app`,
      "Contents",
      "Resources",
      "default-config",
      "compact",
      "COMPACT.md",
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
    repoRoot,
    resourceStagingDir,
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
    "--asar",
    "--app-bundle-id=com.openai.root-worker-prototype.dev",
    "--app-category-type=public.app-category.developer-tools",
    "--extend-info=electron/Info.plist",
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

function installMacRuntimeLauncher(plan) {
  fs.renameSync(
    plan.packagedElectronExecutablePath,
    plan.runtimeExecutablePath,
  );
  fs.copyFileSync(plan.launcherBinaryPath, plan.launcherExecutablePath);
  fs.chmodSync(plan.launcherExecutablePath, 0o755);
}

function assertMacRuntimeLayout(plan) {
  if (
    path.resolve(plan.launcherExecutablePath) ===
    path.resolve(plan.runtimeExecutablePath)
  ) {
    throw new Error(
      "Stable launcher and replaceable Electron runtime must use different paths",
    );
  }
  for (const [label, filePath] of [
    ["launcher executable", plan.launcherExecutablePath],
    ["Electron runtime executable", plan.runtimeExecutablePath],
    ["packaged app.asar", plan.packagedAppAsarPath],
    ["packaged app-server", plan.packagedAppServerPath],
    ["packaged compact prompt", plan.packagedCompactPromptPath],
  ]) {
    if (!fs.statSync(filePath).isFile()) {
      throw new Error(`Missing ${label}: ${filePath}`);
    }
  }
  const plist = fs.readFileSync(plan.infoPlistPath, "utf8");
  if (
    !/<key>CFBundleExecutable<\/key>\s*<string>MorpheusLauncher<\/string>/.test(
      plist,
    )
  ) {
    throw new Error(
      `Packaged Info.plist does not select ${LAUNCHER_EXECUTABLE_NAME}`,
    );
  }
}

function collectMacNestedCodeTargets(plan, options = {}) {
  const readdirSync = options.readdirSync ?? fs.readdirSync;
  const lstatSync = options.lstatSync ?? fs.lstatSync;
  const stableLauncherPath = path.resolve(plan.launcherExecutablePath);
  const executableTargets = new Set([
    path.resolve(plan.runtimeExecutablePath),
    path.resolve(plan.packagedAppServerPath),
  ]);
  const componentTargets = new Set();
  const contentsPath = path.join(plan.appBundlePath, "Contents");

  function visit(directoryPath) {
    for (const entry of readdirSync(directoryPath, { withFileTypes: true })) {
      if (entry.name === "_CodeSignature") {
        continue;
      }
      const entryPath = path.join(directoryPath, entry.name);
      const stat = lstatSync(entryPath);
      if (stat.isSymbolicLink()) {
        continue;
      }
      if (stat.isDirectory()) {
        visit(entryPath);
        if (isMacCodeComponent(entry.name)) {
          componentTargets.add(path.resolve(entryPath));
        }
        continue;
      }
      if (
        stat.isFile() &&
        path.resolve(entryPath) !== stableLauncherPath &&
        ((stat.mode & 0o111) !== 0 || isMacNativeCodeFile(entry.name))
      ) {
        executableTargets.add(path.resolve(entryPath));
      }
    }
  }

  visit(contentsPath);
  const deepestFirst = (left, right) =>
    pathDepth(right) - pathDepth(left) || left.localeCompare(right);
  return [
    ...[...executableTargets].sort(deepestFirst),
    ...[...componentTargets].sort(deepestFirst),
  ];
}

function signMacAppBundle(plan, options = {}) {
  const runCommand = options.runCommand ?? run;
  const hashFile = options.hashFile ?? sha256File;
  const targets =
    options.targets ?? collectMacNestedCodeTargets(plan, options);
  for (const target of targets) {
    runCommand("codesign", ["--force", "--sign", "-", target], {
      cwd: options.cwd ?? path.dirname(plan.appBundlePath),
    });
  }
  runCommand(
    "codesign",
    ["--force", "--sign", "-", plan.appBundlePath],
    { cwd: options.cwd ?? path.dirname(plan.appBundlePath) },
  );
  runCommand(
    "codesign",
    ["--verify", "--strict", plan.appBundlePath],
    { cwd: options.cwd ?? path.dirname(plan.appBundlePath) },
  );
  const launcherHash = hashFile(plan.launcherExecutablePath);
  if (typeof launcherHash !== "string" || !launcherHash) {
    throw new Error(
      "Signed MorpheusLauncher did not produce a stable content hash baseline",
    );
  }
  return {
    launcherExecutablePath: plan.launcherExecutablePath,
    launcherHash,
  };
}

function assertStableLauncherHash(plan, expectedHash, options = {}) {
  const hashFile = options.hashFile ?? sha256File;
  const actualHash = hashFile(plan.launcherExecutablePath);
  if (actualHash !== expectedHash) {
    throw new Error(
      "Stable MorpheusLauncher content no longer matches the signed package baseline",
    );
  }
}

function isMacCodeComponent(name) {
  return [".app", ".appex", ".framework", ".xpc"].some((suffix) =>
    name.endsWith(suffix),
  );
}

function isMacNativeCodeFile(name) {
  return [".dylib", ".node", ".so"].some((suffix) => name.endsWith(suffix));
}

function pathDepth(targetPath) {
  return path.resolve(targetPath).split(path.sep).length;
}

function sha256File(filePath) {
  return crypto
    .createHash("sha256")
    .update(fs.readFileSync(filePath))
    .digest("hex");
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
      "--release",
    ],
    { cwd },
  );
  run(
    "cargo",
    [
      "build",
      "--manifest-path",
      path.relative(cwd, plan.codexRsCargoManifestPath),
      "-p",
      "runtime-launcher",
      "--bin",
      "morpheus-runtime-launcher",
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
  installMacRuntimeLauncher(plan);
  run(
    "/usr/libexec/PlistBuddy",
    [
      "-c",
      `Set :CFBundleExecutable ${LAUNCHER_EXECUTABLE_NAME}`,
      plan.infoPlistPath,
    ],
    { cwd },
  );
  assertMacRuntimeLayout(plan);
  const signing = signMacAppBundle(plan, { cwd });
  assertStableLauncherHash(plan, signing.launcherHash);
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
  assertStableLauncherHash,
  buildElectronPackagerArgs,
  buildMacAppPackagePlan,
  collectMacNestedCodeTargets,
  installMacRuntimeLauncher,
  packageMacApp,
  prepareMacAppResources,
  sha256File,
  signMacAppBundle,
};
