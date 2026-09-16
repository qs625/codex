"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const {
  LAUNCHER_BINARY_NAME,
  assembleOuterApp,
  assertMacRuntimeLayout,
  buildElectronPackagerArgs,
  buildLauncher,
  buildMacAppPackagePlan,
  finalizeMacRuntimeBundle,
  prepareSeedCapsule,
} = require("./package-mac-app.cjs");

test("package plan separates outer app, payload staging, and Seed Capsule", () => {
  const plan = buildMacAppPackagePlan({ cwd: "/repo/apps/root-worker-prototype" });
  assert.equal(
    plan.appBundlePath,
    "/repo/apps/root-worker-prototype/dist-app/Root Worker Prototype-darwin-arm64/Root Worker Prototype.app",
  );
  assert.match(plan.payloadBundlePath, /dist-capsule-payload/);
  assert.match(plan.seedCapsuleDir, /Contents\/Resources\/seed-capsule$/);
  assert.equal(LAUNCHER_BINARY_NAME, "runtime-capsule-launcher");
  assert.match(
    plan.launcherBinaryPath,
    /target\/release\/runtime-capsule-launcher$/,
  );
  assert.match(plan.launcherExecutablePath, /Contents\/MacOS\/MorpheusLauncher$/);
  assert.match(plan.nativeResourceDir, /dist-package-resources\/native$/);
});

test("Electron packager creates the complete inner Runtime app", () => {
  const args = buildElectronPackagerArgs({
    cwd: "/repo/apps/root-worker-prototype",
    payloadStagingDir: "/repo/payload",
    binResourceDir: "/repo/resources/bin",
    defaultConfigResourceDir: "/repo/resources/default-config",
    nativeResourceDir: "/repo/resources/native",
  });
  assert.deepEqual(args.slice(0, 2), [".", "Root Worker Runtime"]);
  assert.ok(args.includes("--extend-info=electron/PayloadInfo.plist"));
  assert.ok(args.includes("--asar"));
  for (const generated of [
    "dist-app",
    "dist-package-resources",
    "dist-capsule-payload",
    "dist-seed-capsule",
  ]) {
    assert.ok(args.includes(`--ignore=^/${generated}($|/)`));
  }
  assert.ok(args.includes("--extra-resource=../../resources/bin"));
  assert.ok(args.includes("--extra-resource=../../resources/default-config"));
  assert.ok(args.includes("--extra-resource=../../resources/native"));
});

test("Launcher build uses the generic Cargo bin and embeds the Seed release", () => {
  const commands = [];
  buildLauncher(
    {
      codexRsCargoManifestPath: "/repo/codex-rs/Cargo.toml",
      repoRoot: "/repo",
    },
    `sha256:${"a".repeat(64)}`,
    {
      runCommand(command, args, options) {
        commands.push([command, args, options]);
      },
    },
  );
  assert.equal(commands[0][0], "cargo");
  assert.ok(commands[0][1].includes("runtime-capsule-launcher"));
  assert.equal(
    commands[0][2].env.RUNTIME_CAPSULE_SEED_RELEASE_ID,
    `sha256:${"a".repeat(64)}`,
  );
  assert.equal(
    commands[0][2].env.RUNTIME_CAPSULE_BUNDLE_ID,
    "com.openai.root-worker-prototype.dev",
  );
});

test("Seed Capsule contains the signed complete payload and outer app contains only Launcher plus Seed", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "seed-package-"));
  try {
    const cwd = path.join(root, "apps", "root-worker-prototype");
    fs.mkdirSync(cwd, { recursive: true });
    const plan = buildMacAppPackagePlan({ cwd });
    const executable = path.join(
      plan.payloadBundlePath,
      "Contents",
      "MacOS",
      "Root Worker Runtime",
    );
    fs.mkdirSync(path.dirname(executable), { recursive: true });
    fs.writeFileSync(executable, "#!/bin/sh\n", { mode: 0o755 });
    fs.mkdirSync(path.dirname(plan.launcherBinaryPath), { recursive: true });
    fs.writeFileSync(plan.launcherBinaryPath, "launcher", { mode: 0o755 });
    fs.mkdirSync(path.dirname(plan.outerInfoPlistSourcePath), {
      recursive: true,
    });
    fs.writeFileSync(plan.outerInfoPlistSourcePath, "<plist/>");
    const commands = [];
    const manifest = prepareSeedCapsule(plan, {
      sourceCommit: "deadbeef",
      runCommand(command, args) {
        commands.push([command, args]);
      },
    });
    assembleOuterApp(plan);
    assertMacRuntimeLayout(plan, manifest);
    assert.match(manifest.releaseId, /^sha256:[0-9a-f]{64}$/);
    assert.equal(commands.filter(([command]) => command === "codesign").length, 2);
    assert.equal(
      fs.existsSync(path.join(plan.appBundlePath, "Contents", "Frameworks")),
      false,
    );
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("outer signing does not recursively mutate the sealed Seed Capsule", () => {
  const commands = [];
  const plan = {
    appBundleInfoPlistPath: "/tmp/Outer.app/Contents/Info.plist",
    appBundlePath: "/tmp/Outer.app",
    launcherExecutablePath: "/tmp/Outer.app/Contents/MacOS/MorpheusLauncher",
    seedCapsuleDir: "/tmp/Outer.app/Contents/Resources/seed-capsule",
    sourceAppDir: "/repo",
  };
  finalizeMacRuntimeBundle(
    plan,
    { releaseId: `sha256:${"a".repeat(64)}` },
    {
      fsOps: {
        lstatSync() {
          return { isFile: () => true };
        },
        existsSync() {
          return false;
        },
      },
      runCommand(command, args) {
        commands.push([command, args]);
      },
    },
  );
  assert.deepEqual(commands[0], [
    "codesign",
    ["--force", "--sign", "-", "/tmp/Outer.app"],
  ]);
  assert.deepEqual(commands[1], [
    "codesign",
    ["--verify", "--deep", "--strict", "/tmp/Outer.app"],
  ]);
});
