"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const {
  APP_NAME,
  GENERATED_SOURCE_DIR_NAMES,
  PAYLOAD_EXECUTABLE_RELATIVE_PATH,
  normalizeRuntimeCapsuleTree,
  resolveInstalledArtifactUpdatePlan,
  resolveRuntimeLauncherStateRoot,
  stagePayloadResources,
  updateInstalledArtifacts,
} = require("./installedArtifactUpdate.cjs");

test("candidate packager excludes every generated packaging directory", () => {
  assert.deepEqual(GENERATED_SOURCE_DIR_NAMES, [
    "dist-app",
    "dist-package-resources",
    "dist-capsule-payload",
    "dist-seed-capsule",
  ]);
});

test("packaged plan is typed unavailable without a source workspace", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "capsule-plan-"));
  try {
    const plan = resolveInstalledArtifactUpdatePlan({
      env: { MORPHEUS_HOME: root },
      isPackaged: true,
      platform: "darwin",
      resourcesPath: "/Applications/Test.app/Contents/Resources",
    });
    assert.equal(plan.disabled, true);
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("launcher state root uses Morpheus home by default", () => {
  assert.equal(
    resolveRuntimeLauncherStateRoot({ MORPHEUS_HOME: "/tmp/morpheus" }),
    "/tmp/morpheus/runtime-launcher",
  );
});

test("launcher state root prefers the generic Capsule home override", () => {
  assert.equal(
    resolveRuntimeLauncherStateRoot({
      RUNTIME_CAPSULE_LAUNCHER_HOME: "/tmp/runtime-capsule-launcher",
      MORPHEUS_RUNTIME_LAUNCHER_HOME: "/tmp/legacy-launcher",
    }),
    "/tmp/runtime-capsule-launcher",
  );
});

test("stagePayloadResources installs app-server and compact prompt", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "capsule-resources-"));
  try {
    const appServer = path.join(root, "source-app-server");
    const compact = path.join(root, "source-compact.md");
    fs.writeFileSync(appServer, "binary", { mode: 0o755 });
    fs.writeFileSync(compact, "prompt");
    const target = path.join(root, "resources");
    stagePayloadResources(
      {
        appServerBinaryPath: appServer,
        defaultCompactPromptSourcePath: compact,
      },
      target,
    );
    assert.equal(fs.readFileSync(path.join(target, "bin", "app-server"), "utf8"), "binary");
    assert.equal(
      fs.readFileSync(
        path.join(target, "default-config", "compact", "COMPACT.md"),
        "utf8",
      ),
      "prompt",
    );
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("producer writes a complete Electron app Capsule under incoming", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "capsule-producer-"));
  try {
    const sourceAppDir = path.join(root, "source", "apps", "root-worker-prototype");
    const codexRsDir = path.join(root, "source", "codex-rs");
    const appServerBinaryPath = path.join(root, "app-server");
    const compactPath = path.join(root, "COMPACT.md");
    fs.mkdirSync(sourceAppDir, { recursive: true });
    fs.mkdirSync(codexRsDir, { recursive: true });
    fs.writeFileSync(appServerBinaryPath, "server", { mode: 0o755 });
    fs.writeFileSync(compactPath, "compact");
    const plan = {
      appServerBinaryPath,
      codexRsDir,
      commandEnv: {},
      defaultCompactPromptSourcePath: compactPath,
      sourceAppDir,
      stateRoot: path.join(root, "state"),
      workspace: path.join(root, "source"),
    };
    let packagerArgs = null;
    const result = updateInstalledArtifacts(plan, {
      activationId: "activation-test",
      sourceCommit: "deadbeef",
      runCommand(command, args) {
        if (command === "pnpm" && args.includes("@electron/packager")) {
          packagerArgs = args;
          const out = args.find((arg) => arg.startsWith("--out=")).slice(6);
          const payload = path.join(
            out,
            `${APP_NAME}-darwin-arm64`,
            `${APP_NAME}.app`,
          );
          const executable = path.join(
            payload,
            "Contents",
            "MacOS",
            APP_NAME,
          );
          fs.mkdirSync(path.dirname(executable), { recursive: true });
          fs.writeFileSync(executable, "#!/bin/sh\n", { mode: 0o755 });
        }
      },
    });
    assert.equal(result.activationId, "activation-test");
    for (const generated of GENERATED_SOURCE_DIR_NAMES) {
      assert.ok(packagerArgs.includes(`--ignore=^/${generated}($|/)`));
    }
    assert.match(result.releaseId, /^sha256:[0-9a-f]{64}$/);
    assert.ok(
      fs.statSync(
        path.join(
          result.incomingRoot,
          ...PAYLOAD_EXECUTABLE_RELATIVE_PATH.split(path.sep),
        ),
      ).isFile(),
    );
    assert.ok(fs.statSync(path.join(result.incomingRoot, "capsule.json")).isFile());
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("normalization rejects hard links", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "capsule-hardlink-"));
  try {
    const first = path.join(root, "first");
    fs.writeFileSync(first, "same");
    fs.linkSync(first, path.join(root, "second"));
    assert.throws(
      () => normalizeRuntimeCapsuleTree(root),
      /must not be a hard link/,
    );
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
});
