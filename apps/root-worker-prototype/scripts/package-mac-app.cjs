const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const APP_NAME = "Root Worker Prototype";
const APP_PLATFORM_DIR = "Root Worker Prototype-darwin-arm64";
const DIST_DIR_NAME = "dist-app";
const RESOURCE_STAGING_DIR_NAME = "dist-package-resources";
const SOURCE_RESOURCE_DIR_NAME = "source";
const SOURCE_EXCLUDED_DIR_NAMES = new Set([
  ".git",
  ".morpheus",
  "dist-app",
  "dist-package-resources",
  "node_modules",
  "target",
]);
const SOURCE_EXCLUDED_FILE_NAMES = new Set([
  ".DS_Store",
  ".env",
  "local.properties",
]);
const SOURCE_EXCLUDED_FILE_SUFFIXES = [
  ".key",
  ".mobileprovision",
  ".p12",
  ".pem",
];

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
  const sourceResourceDir = path.join(
    resourceStagingDir,
    SOURCE_RESOURCE_DIR_NAME,
  );
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
    sourceResourceDir,
  };
}

function buildElectronPackagerArgs({
  cwd = process.cwd(),
  appName = APP_NAME,
  distDirName = DIST_DIR_NAME,
  binResourceDir,
  defaultConfigResourceDir,
  sourceResourceDir,
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
    `--extra-resource=${path.relative(cwd, sourceResourceDir)}`,
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
  stageSourceSnapshot(plan);
}

function stageSourceSnapshot(plan, options = {}) {
  fs.rmSync(plan.sourceResourceDir, { force: true, recursive: true });
  fs.mkdirSync(plan.sourceResourceDir, { recursive: true });
  const files = listSourceSnapshotFiles(plan.repoRoot, options);
  for (const relativePath of files) {
    const sourcePath = path.join(plan.repoRoot, relativePath);
    const targetPath = path.join(plan.sourceResourceDir, relativePath);
    const stat = fs.lstatSync(sourcePath);
    if (!stat.isFile()) {
      continue;
    }
    fs.mkdirSync(path.dirname(targetPath), { recursive: true });
    fs.copyFileSync(sourcePath, targetPath);
  }
}

function listSourceSnapshotFiles(repoRoot, options = {}) {
  const trackedFiles =
    options.trackedFiles ?? listGitTrackedFiles(repoRoot, options);
  return trackedFiles
    .map(normalizeSourceSnapshotRelativePath)
    .filter((relativePath) => shouldIncludeSourceSnapshotPath(relativePath))
    .sort();
}

function listGitTrackedFiles(repoRoot, options = {}) {
  const spawn = options.spawnSync ?? spawnSync;
  const result = spawn("rtk", ["git", "ls-files", "-z"], {
    cwd: repoRoot,
    encoding: "buffer",
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    const stderr = result.stderr ? String(result.stderr) : "";
    throw new Error(
      `rtk git ls-files exited with ${result.status}${stderr ? `: ${stderr}` : ""}`,
    );
  }
  return Buffer.from(result.stdout ?? Buffer.alloc(0))
    .toString("utf8")
    .split("\0")
    .filter(Boolean);
}

function normalizeSourceSnapshotRelativePath(relativePath) {
  return relativePath.split(path.sep).join("/");
}

function shouldIncludeSourceSnapshotPath(relativePath) {
  if (
    !relativePath ||
    path.isAbsolute(relativePath) ||
    relativePath.includes("\0")
  ) {
    return false;
  }
  const normalized = normalizeSourceSnapshotRelativePath(relativePath);
  const segments = normalized.split("/");
  if (
    segments.some(
      (segment) =>
        !segment ||
        segment === "." ||
        segment === ".." ||
        SOURCE_EXCLUDED_DIR_NAMES.has(segment),
    )
  ) {
    return false;
  }
  const fileName = segments.at(-1) ?? "";
  if (
    SOURCE_EXCLUDED_FILE_NAMES.has(fileName) ||
    fileName.startsWith(".env") ||
    fileName.endsWith(".local") ||
    fileName.includes(".secret.") ||
    SOURCE_EXCLUDED_FILE_SUFFIXES.some((suffix) => fileName.endsWith(suffix))
  ) {
    return false;
  }
  return true;
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
        sourceResourceDir: plan.sourceResourceDir,
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
  listSourceSnapshotFiles,
  packageMacApp,
  prepareMacAppResources,
  shouldIncludeSourceSnapshotPath,
  stageSourceSnapshot,
};
