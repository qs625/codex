"use strict";

const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const {
  PAYLOAD_EXECUTABLE_RELATIVE_PATH,
  PAYLOAD_RELATIVE_PATH,
  GENERATED_SOURCE_DIR_NAMES,
  clearExtendedAttributes,
  normalizeRuntimeCapsuleTree,
  stagePayloadResources,
} = require("../electron/installedArtifactUpdate.cjs");
const { createRuntimeCapsule } = require("../electron/runtimeCapsule.cjs");

const APP_NAME = "Root Worker Prototype";
const PAYLOAD_APP_NAME = "Root Worker Runtime";
const APP_PLATFORM_DIR = "Root Worker Prototype-darwin-arm64";
const LAUNCHER_BINARY_NAME = "runtime-capsule-launcher";
const LAUNCHER_EXECUTABLE_NAME = "MorpheusLauncher";
const DIST_DIR_NAME = "dist-app";
const RESOURCE_STAGING_DIR_NAME = "dist-package-resources";
const PAYLOAD_STAGING_DIR_NAME = "dist-capsule-payload";
const SEED_STAGING_DIR_NAME = "dist-seed-capsule";
const SEED_CAPSULE_DIR_NAME = "seed-capsule";
const OUTER_BUNDLE_IDENTIFIER = "com.openai.root-worker-prototype.dev";

function buildMacAppPackagePlan({
  cwd = process.cwd(),
  appName = APP_NAME,
  distDirName = DIST_DIR_NAME,
  resourceStagingDirName = RESOURCE_STAGING_DIR_NAME,
} = {}) {
  const repoRoot = path.resolve(cwd, "..", "..");
  const codexRsDir = path.join(repoRoot, "codex-rs");
  const resourceStagingDir = path.join(cwd, resourceStagingDirName);
  const appBundlePath = path.join(
    cwd,
    distDirName,
    APP_PLATFORM_DIR,
    `${appName}.app`,
  );
  const payloadStagingDir = path.join(cwd, PAYLOAD_STAGING_DIR_NAME);
  const payloadBundlePath = path.join(
    payloadStagingDir,
    `${PAYLOAD_APP_NAME}-darwin-arm64`,
    `${PAYLOAD_APP_NAME}.app`,
  );
  const seedStagingDir = path.join(cwd, SEED_STAGING_DIR_NAME);
  const seedPayloadPath = path.join(
    seedStagingDir,
    ...PAYLOAD_RELATIVE_PATH.split(path.sep),
  );
  return {
    appBundleInfoPlistPath: path.join(appBundlePath, "Contents", "Info.plist"),
    appBundlePath,
    appServerBinaryPath: path.join(
      codexRsDir,
      "target",
      "release",
      "app-server",
    ),
    binResourceDir: path.join(resourceStagingDir, "bin"),
    codexRsCargoManifestPath: path.join(codexRsDir, "Cargo.toml"),
    codexRsDir,
    defaultCompactPromptSourcePath: path.join(
      codexRsDir,
      "thread-service",
      "templates",
      "compact",
      "prompt.md",
    ),
    defaultConfigResourceDir: path.join(resourceStagingDir, "default-config"),
    distDir: path.join(cwd, distDirName),
    launcherBinaryPath: path.join(
      codexRsDir,
      "target",
      "release",
      LAUNCHER_BINARY_NAME,
    ),
    launcherExecutablePath: path.join(
      appBundlePath,
      "Contents",
      "MacOS",
      LAUNCHER_EXECUTABLE_NAME,
    ),
    outerInfoPlistSourcePath: path.join(cwd, "electron", "Info.plist"),
    payloadBundlePath,
    payloadStagingDir,
    repoRoot,
    resourceStagingDir,
    seedCapsuleDir: path.join(
      appBundlePath,
      "Contents",
      "Resources",
      SEED_CAPSULE_DIR_NAME,
    ),
    seedPayloadPath,
    seedStagingDir,
    sourceAppDir: cwd,
  };
}

function buildElectronPackagerArgs({
  cwd = process.cwd(),
  payloadStagingDir = path.join(cwd, PAYLOAD_STAGING_DIR_NAME),
  binResourceDir,
  defaultConfigResourceDir,
} = {}) {
  return [
    ".",
    PAYLOAD_APP_NAME,
    "--platform=darwin",
    "--arch=arm64",
    `--out=${payloadStagingDir}`,
    "--overwrite",
    "--app-bundle-id=com.openai.root-worker-prototype.runtime.dev",
    "--app-category-type=public.app-category.developer-tools",
    "--extend-info=electron/PayloadInfo.plist",
    "--asar",
    ...GENERATED_SOURCE_DIR_NAMES.map(
      (name) => `--ignore=^/${name}($|/)`,
    ),
    "--no-prune",
    `--extra-resource=${path.relative(cwd, binResourceDir)}`,
    `--extra-resource=${path.relative(cwd, defaultConfigResourceDir)}`,
  ];
}

function prepareMacAppResources(plan, fsOps = fs) {
  fsOps.rmSync(plan.resourceStagingDir, { force: true, recursive: true });
  stagePayloadResources(plan, plan.resourceStagingDir, fsOps);
}

function prepareSeedCapsule(
  plan,
  {
    fsOps = fs,
    runCommand = run,
    sourceCommit,
  } = {},
) {
  fsOps.rmSync(plan.seedStagingDir, { force: true, recursive: true });
  fsOps.mkdirSync(path.dirname(plan.seedPayloadPath), {
    recursive: true,
    mode: 0o755,
  });
  fsOps.renameSync(plan.payloadBundlePath, plan.seedPayloadPath);
  normalizeRuntimeCapsuleTree(plan.seedStagingDir, fsOps);
  clearExtendedAttributes(plan.seedStagingDir, { runCommand });
  runCommand(
    "codesign",
    ["--force", "--deep", "--sign", "-", plan.seedPayloadPath],
    { cwd: plan.sourceAppDir },
  );
  runCommand(
    "codesign",
    ["--verify", "--deep", "--strict", plan.seedPayloadPath],
    { cwd: plan.sourceAppDir },
  );
  return createRuntimeCapsule(plan.seedStagingDir, {
    arch: "arm64",
    executable: PAYLOAD_EXECUTABLE_RELATIVE_PATH.split(path.sep).join("/"),
    metadata: { sourceCommit },
    os: "darwin",
    readinessTimeoutMs: 30_000,
    fsOps,
  });
}

function buildLauncher(plan, seedReleaseId, { runCommand = run } = {}) {
  runCommand(
    "cargo",
    [
      "build",
      "--manifest-path",
      plan.codexRsCargoManifestPath,
      "-p",
      "runtime-launcher",
      "--bin",
      LAUNCHER_BINARY_NAME,
      "--release",
    ],
    {
      cwd: plan.repoRoot,
      env: {
        ...process.env,
        RUNTIME_CAPSULE_SEED_RELEASE_ID: seedReleaseId,
        RUNTIME_CAPSULE_BUNDLE_ID: OUTER_BUNDLE_IDENTIFIER,
      },
    },
  );
}

function assembleOuterApp(plan, { fsOps = fs } = {}) {
  fsOps.rmSync(plan.appBundlePath, { force: true, recursive: true });
  fsOps.mkdirSync(path.dirname(plan.launcherExecutablePath), {
    recursive: true,
    mode: 0o755,
  });
  fsOps.mkdirSync(path.dirname(plan.seedCapsuleDir), {
    recursive: true,
    mode: 0o755,
  });
  fsOps.copyFileSync(
    plan.outerInfoPlistSourcePath,
    plan.appBundleInfoPlistPath,
  );
  fsOps.writeFileSync(
    path.join(plan.appBundlePath, "Contents", "PkgInfo"),
    "APPL????",
    { encoding: "ascii", mode: 0o644 },
  );
  fsOps.copyFileSync(plan.launcherBinaryPath, plan.launcherExecutablePath);
  fsOps.chmodSync(plan.launcherExecutablePath, 0o755);
  fsOps.cpSync(plan.seedStagingDir, plan.seedCapsuleDir, {
    recursive: true,
    dereference: false,
    verbatimSymlinks: true,
  });
}

function assertMacRuntimeLayout(plan, manifest, fsOps = fs) {
  for (const targetPath of [
    plan.launcherExecutablePath,
    plan.appBundleInfoPlistPath,
    path.join(plan.seedCapsuleDir, "capsule.json"),
    path.join(
      plan.seedCapsuleDir,
      ...PAYLOAD_EXECUTABLE_RELATIVE_PATH.split(path.sep),
    ),
  ]) {
    if (!fsOps.lstatSync(targetPath).isFile()) {
      throw new Error(`Required packaged artifact is missing: ${targetPath}`);
    }
  }
  if (!/^sha256:[0-9a-f]{64}$/.test(manifest.releaseId)) {
    throw new Error("Seed Capsule releaseId is invalid");
  }
  for (const forbidden of ["Frameworks", "app.asar", "bin", "default-config"]) {
    if (
      fsOps.existsSync(
        path.join(plan.appBundlePath, "Contents", "Resources", forbidden),
      ) ||
      fsOps.existsSync(path.join(plan.appBundlePath, "Contents", forbidden))
    ) {
      throw new Error(
        `Electron payload escaped the Seed Capsule: ${forbidden}`,
      );
    }
  }
}

function finalizeMacRuntimeBundle(
  plan,
  manifest,
  { fsOps = fs, runCommand = run } = {},
) {
  assertMacRuntimeLayout(plan, manifest, fsOps);
  runCommand(
    "codesign",
    ["--force", "--sign", "-", plan.appBundlePath],
    { cwd: plan.sourceAppDir },
  );
  runCommand(
    "codesign",
    ["--verify", "--deep", "--strict", plan.appBundlePath],
    { cwd: plan.sourceAppDir },
  );
  return manifest;
}

function packageMacApp({ cwd = process.cwd(), platform = process.platform } = {}) {
  if (platform !== "darwin") {
    throw new Error("macOS app packaging requires codesign and must run on macOS.");
  }
  const plan = buildMacAppPackagePlan({ cwd });
  for (const target of [
    plan.distDir,
    plan.payloadStagingDir,
    plan.seedStagingDir,
  ]) {
    fs.rmSync(target, { force: true, recursive: true });
  }
  try {
    run("pnpm", ["build"], { cwd });
    run(
      "cargo",
      [
        "build",
        "--manifest-path",
        plan.codexRsCargoManifestPath,
        "-p",
        "app-server",
        "--bin",
        "app-server",
        "--release",
      ],
      { cwd: plan.repoRoot },
    );
    prepareMacAppResources(plan);
    run("pnpm", ["dlx", "@electron/packager", ...buildElectronPackagerArgs({
      cwd,
      payloadStagingDir: plan.payloadStagingDir,
      binResourceDir: plan.binResourceDir,
      defaultConfigResourceDir: plan.defaultConfigResourceDir,
    })], { cwd });
    const sourceCommit = capture("git", ["rev-parse", "HEAD"], { cwd }).trim();
    const manifest = prepareSeedCapsule(plan, { sourceCommit });
    buildLauncher(plan, manifest.releaseId);
    assembleOuterApp(plan);
    finalizeMacRuntimeBundle(plan, manifest);
    return { manifest, plan };
  } finally {
    fs.rmSync(plan.payloadStagingDir, { force: true, recursive: true });
    fs.rmSync(plan.seedStagingDir, { force: true, recursive: true });
    fs.rmSync(plan.resourceStagingDir, { force: true, recursive: true });
  }
}

function capture(command, args, options) {
  return run(command, args, { ...options, capture: true });
}

function run(command, args, options = {}) {
  const result = (options.spawnSync ?? spawnSync)(command, args, {
    cwd: options.cwd,
    encoding: "utf8",
    env: options.env ?? process.env,
    stdio: options.capture ? "pipe" : "inherit",
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} exited with ${result.status}`);
  }
  return result.stdout ?? "";
}

if (require.main === module) {
  packageMacApp();
}

module.exports = {
  APP_NAME,
  LAUNCHER_BINARY_NAME,
  LAUNCHER_EXECUTABLE_NAME,
  PAYLOAD_APP_NAME,
  assembleOuterApp,
  assertMacRuntimeLayout,
  buildElectronPackagerArgs,
  buildLauncher,
  buildMacAppPackagePlan,
  finalizeMacRuntimeBundle,
  packageMacApp,
  prepareMacAppResources,
  prepareSeedCapsule,
};
