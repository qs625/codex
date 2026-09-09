const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const {
  assertMacRuntimeLayout,
  assertStableLauncherHash,
  buildElectronPackagerArgs,
  buildMacAppPackagePlan,
  collectMacNestedCodeTargets,
  installMacRuntimeLauncher,
  signMacAppBundle,
} = require("./package-mac-app.cjs");

test("mac app package plan stages release app-server and compact seed resources", () => {
  const cwd = "/repo/apps/root-worker-prototype";

  assert.deepEqual(buildMacAppPackagePlan({ cwd }), {
    appBundlePath:
      "/repo/apps/root-worker-prototype/dist-app/Root Worker Prototype-darwin-arm64/Root Worker Prototype.app",
    appServerBinaryPath: "/repo/codex-rs/target/release/app-server",
    launcherBinaryPath:
      "/repo/codex-rs/target/release/morpheus-runtime-launcher",
    launcherExecutablePath:
      "/repo/apps/root-worker-prototype/dist-app/Root Worker Prototype-darwin-arm64/Root Worker Prototype.app/Contents/MacOS/MorpheusLauncher",
    packagedElectronExecutablePath:
      "/repo/apps/root-worker-prototype/dist-app/Root Worker Prototype-darwin-arm64/Root Worker Prototype.app/Contents/MacOS/Root Worker Prototype",
    runtimeExecutablePath:
      "/repo/apps/root-worker-prototype/dist-app/Root Worker Prototype-darwin-arm64/Root Worker Prototype.app/Contents/MacOS/Root Worker Runtime",
    infoPlistPath:
      "/repo/apps/root-worker-prototype/dist-app/Root Worker Prototype-darwin-arm64/Root Worker Prototype.app/Contents/Info.plist",
    packagedAppAsarPath:
      "/repo/apps/root-worker-prototype/dist-app/Root Worker Prototype-darwin-arm64/Root Worker Prototype.app/Contents/Resources/app.asar",
    packagedAppServerPath:
      "/repo/apps/root-worker-prototype/dist-app/Root Worker Prototype-darwin-arm64/Root Worker Prototype.app/Contents/Resources/bin/app-server",
    packagedCompactPromptPath:
      "/repo/apps/root-worker-prototype/dist-app/Root Worker Prototype-darwin-arm64/Root Worker Prototype.app/Contents/Resources/default-config/compact/COMPACT.md",
    binResourceDir: "/repo/apps/root-worker-prototype/dist-package-resources/bin",
    codexRsCargoManifestPath: "/repo/codex-rs/Cargo.toml",
    defaultCompactPromptResourcePath:
      "/repo/apps/root-worker-prototype/dist-package-resources/default-config/compact/COMPACT.md",
    defaultCompactPromptSourcePath:
      "/repo/codex-rs/thread-service/templates/compact/prompt.md",
    defaultConfigResourceDir:
      "/repo/apps/root-worker-prototype/dist-package-resources/default-config",
    distDir: "/repo/apps/root-worker-prototype/dist-app",
    resourceStagingDir:
      "/repo/apps/root-worker-prototype/dist-package-resources",
    repoRoot: "/repo",
  });
});

test("electron packager args include app-server and default config resources", () => {
  const cwd = "/repo/apps/root-worker-prototype";
  const args = buildElectronPackagerArgs({
    cwd,
    binResourceDir: path.join(cwd, "dist-package-resources/bin"),
    defaultConfigResourceDir: path.join(
      cwd,
      "dist-package-resources/default-config",
    ),
  });

  assert.ok(args.includes("--extra-resource=dist-package-resources/bin"));
  assert.ok(args.includes("--asar"));
  assert.ok(
    args.includes("--extra-resource=dist-package-resources/default-config"),
  );
  assert.equal(
    args.some((arg) => arg === "--extra-resource=dist-package-resources/source"),
    false,
  );
  assert.ok(args.includes("--ignore=^/dist-package-resources($|/)"));
  assert.ok(args.includes("--no-prune"));
});

test("mac package moves Electron runtime and installs stable launcher", () => {
  const root = fs.mkdtempSync("/tmp/morpheus-package-layout-");
  const macosDir = path.join(root, "Morpheus.app/Contents/MacOS");
  fs.mkdirSync(macosDir, { recursive: true });
  const plan = {
    packagedElectronExecutablePath: path.join(macosDir, "Root Worker Prototype"),
    runtimeExecutablePath: path.join(macosDir, "Root Worker Runtime"),
    launcherBinaryPath: path.join(root, "morpheus-runtime-launcher"),
    launcherExecutablePath: path.join(macosDir, "MorpheusLauncher"),
  };
  fs.writeFileSync(plan.packagedElectronExecutablePath, "electron");
  fs.writeFileSync(plan.launcherBinaryPath, "launcher");

  installMacRuntimeLauncher(plan);

  assert.equal(fs.readFileSync(plan.runtimeExecutablePath, "utf8"), "electron");
  assert.equal(fs.readFileSync(plan.launcherExecutablePath, "utf8"), "launcher");
  assert.equal(fs.existsSync(plan.packagedElectronExecutablePath), false);
});

test("final mac bundle layout resolves launcher, runtime and resources at real paths", () => {
  const root = fs.mkdtempSync("/tmp/morpheus-final-layout-");
  const plan = buildMacAppPackagePlan({
    cwd: path.join(root, "repo/apps/root-worker-prototype"),
  });
  for (const filePath of [
    plan.launcherExecutablePath,
    plan.runtimeExecutablePath,
    plan.packagedAppAsarPath,
    plan.packagedAppServerPath,
    plan.packagedCompactPromptPath,
  ]) {
    fs.mkdirSync(path.dirname(filePath), { recursive: true });
    fs.writeFileSync(filePath, "artifact");
  }
  fs.mkdirSync(path.dirname(plan.infoPlistPath), { recursive: true });
  fs.writeFileSync(
    plan.infoPlistPath,
    "<plist><dict><key>CFBundleExecutable</key><string>MorpheusLauncher</string></dict></plist>",
  );

  assert.notEqual(plan.launcherExecutablePath, plan.runtimeExecutablePath);
  assert.doesNotThrow(() => assertMacRuntimeLayout(plan));
});

test("mac signing orders nested executables and components before the app bundle without deep", () => {
  const root = fs.mkdtempSync("/tmp/morpheus-sign-order-");
  const plan = buildMacAppPackagePlan({
    cwd: path.join(root, "repo/apps/root-worker-prototype"),
  });
  const helperApp = path.join(
    plan.appBundlePath,
    "Contents/Frameworks/Morpheus Helper.app",
  );
  const helperExecutable = path.join(
    helperApp,
    "Contents/MacOS/Morpheus Helper",
  );
  const framework = path.join(
    plan.appBundlePath,
    "Contents/Frameworks/Electron Framework.framework",
  );
  for (const filePath of [
    plan.launcherExecutablePath,
    plan.runtimeExecutablePath,
    plan.packagedAppServerPath,
    helperExecutable,
    path.join(framework, "Versions/A/Electron Framework"),
  ]) {
    fs.mkdirSync(path.dirname(filePath), { recursive: true });
    fs.writeFileSync(filePath, "code");
    fs.chmodSync(filePath, 0o755);
  }
  const targets = collectMacNestedCodeTargets(plan);
  const calls = [];

  signMacAppBundle(plan, {
    targets,
    hashFile: () => "stable-launcher-hash",
    runCommand: (command, args) => calls.push([command, ...args]),
  });

  const signedTargets = calls
    .filter((args) => args.includes("--sign"))
    .map((args) => args.at(-1));
  assert.equal(signedTargets.at(-1), plan.appBundlePath);
  assert.equal(signedTargets.includes(plan.launcherExecutablePath), false);
  assert.ok(signedTargets.indexOf(helperExecutable) < signedTargets.indexOf(helperApp));
  assert.ok(
    signedTargets.indexOf(
      path.join(framework, "Versions/A/Electron Framework"),
    ) < signedTargets.indexOf(framework),
  );
  assert.equal(calls.some((args) => args.includes("--deep")), false);
  assert.deepEqual(calls.at(-1), [
    "codesign",
    "--verify",
    "--strict",
    plan.appBundlePath,
  ]);
});

test("mac signing records the final launcher hash after a legal top-level re-sign", () => {
  const plan = {
    appBundlePath: "/Applications/Morpheus.app",
    launcherExecutablePath:
      "/Applications/Morpheus.app/Contents/MacOS/MorpheusLauncher",
  };
  let launcherContents = "unsigned-launcher";
  const signing = signMacAppBundle(plan, {
    targets: [],
    hashFile: () => launcherContents,
    runCommand: (_command, args) => {
      if (args.at(-1) === plan.appBundlePath && args.includes("--sign")) {
        launcherContents = "signed-launcher";
      }
    },
  });

  assert.equal(signing.launcherHash, "signed-launcher");
  assert.doesNotThrow(() =>
    assertStableLauncherHash(plan, signing.launcherHash, {
      hashFile: () => launcherContents,
    }),
  );
  launcherContents = "tampered-launcher";
  assert.throws(
    () =>
      assertStableLauncherHash(plan, signing.launcherHash, {
        hashFile: () => launcherContents,
      }),
    /no longer matches the signed package baseline/,
  );
});

test("mac app Info.plist declares permission usage descriptions", () => {
  const plist = fs.readFileSync(
    path.join(__dirname, "..", "electron", "Info.plist"),
    "utf8",
  );

  assert.match(plist, /<key>NSMicrophoneUsageDescription<\/key>/);
  assert.match(plist, /<key>NSScreenCaptureUsageDescription<\/key>/);
  assert.match(plist, /<key>NSAppleEventsUsageDescription<\/key>/);
  assert.match(
    plist,
    /<key>CFBundleExecutable<\/key>\s*<string>MorpheusLauncher<\/string>/,
  );
});
