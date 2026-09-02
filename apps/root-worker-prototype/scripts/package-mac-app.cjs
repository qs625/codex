const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const APP_NAME = "Root Worker Prototype";
const APP_PLATFORM_DIR = "Root Worker Prototype-darwin-arm64";
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

function packageMacApp({ cwd = process.cwd(), platform = process.platform } = {}) {
  if (platform !== "darwin") {
    throw new Error("macOS app packaging requires codesign and must run on macOS.");
  }
  const plan = buildMacAppPackagePlan({ cwd });
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
  prepareMacAppResources(plan);
  run(
    "rtk",
    [
      "pnpm",
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
  run(
    "rtk",
    ["codesign", "--force", "--deep", "--sign", "-", plan.appBundlePath],
    { cwd },
  );
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
  buildElectronPackagerArgs,
  buildMacAppPackagePlan,
  packageMacApp,
  prepareMacAppResources,
};
