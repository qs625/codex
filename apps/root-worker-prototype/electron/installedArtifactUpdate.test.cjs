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
  removeInstalledArtifactTree,
  resolveInstalledArtifactUpdatePlan,
  resolveInstalledArtifactFileSystem,
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

test("Electron Runtime Capsule operations use original-fs while Node falls back", () => {
  const patchedFs = { patched: true };
  const originalFs = { original: true };
  assert.equal(
    resolveInstalledArtifactFileSystem({
      defaultFsOps: patchedFs,
      isElectron: true,
      loadOriginalFileSystem: () => originalFs,
    }),
    originalFs,
  );
  assert.equal(
    resolveInstalledArtifactFileSystem({
      defaultFsOps: patchedFs,
      isElectron: false,
    }),
    patchedFs,
  );
  assert.equal(
    resolveInstalledArtifactFileSystem({
      fsOps: patchedFs,
      isElectron: true,
      loadOriginalFileSystem() {
        throw new Error("explicit fsOps must win");
      },
    }),
    patchedFs,
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
    const appAsarSuffix = path.join("Contents", "Resources", "app.asar");
    const patchedFs = new Proxy(fs, {
      get(target, property) {
        if (property === "lstatSync") {
          return (targetPath) => {
            if (targetPath.endsWith(appAsarSuffix)) {
              return {
                isDirectory: () => true,
                isFile: () => false,
                isSymbolicLink: () => false,
                mode: 0o755,
                nlink: 1,
              };
            }
            return target.lstatSync(targetPath);
          };
        }
        return target[property];
      },
    });
    let rawAppAsarReads = 0;
    const rawFs = new Proxy(fs, {
      get(target, property) {
        if (property === "readFileSync") {
          return (targetPath, ...args) => {
            if (targetPath.endsWith(appAsarSuffix)) {
              rawAppAsarReads += 1;
            }
            return target.readFileSync(targetPath, ...args);
          };
        }
        return target[property];
      },
    });
    let packagerArgs = null;
    const result = updateInstalledArtifacts(plan, {
      activationId: "activation-test",
      defaultFsOps: patchedFs,
      isElectron: true,
      loadOriginalFileSystem: () => rawFs,
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
          fs.mkdirSync(
            path.join(payload, "Contents", "Resources"),
            { recursive: true },
          );
          fs.writeFileSync(
            path.join(payload, "Contents", "Resources", "app.asar"),
            "asar",
          );
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
    const appAsarEntry = result.manifest.entries.find(
      (entry) =>
        entry.path ===
        `payload/${APP_NAME}.app/Contents/Resources/app.asar`,
    );
    assert.equal(appAsarEntry?.type, "file");
    assert.match(appAsarEntry?.sha256, /^[0-9a-f]{64}$/);
    assert.ok(rawAppAsarReads > 0);
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("Electron producer uses raw filesystem for regular app.asar and failure cleanup", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "capsule-raw-fs-"));
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
    const appAsarSuffix = path.join("Contents", "Resources", "app.asar");
    const patchedFs = new Proxy(fs, {
      get(target, property) {
        if (property === "lstatSync") {
          return (targetPath) => {
            if (targetPath.endsWith(appAsarSuffix)) {
              return {
                isDirectory: () => true,
                isFile: () => false,
                isSymbolicLink: () => false,
                mode: 0o755,
                nlink: 1,
              };
            }
            return target.lstatSync(targetPath);
          };
        }
        if (property === "rmSync") {
          return (targetPath, options) => {
            if (
              targetPath.includes("activation-raw-failure") &&
              fs.existsSync(path.join(
                targetPath,
                "payload",
                `${APP_NAME}.app`,
                appAsarSuffix,
              ))
            ) {
              const error = new Error(`not a directory, rmdir '${path.join(targetPath, appAsarSuffix)}'`);
              error.code = "ENOTDIR";
              throw error;
            }
            return target.rmSync(targetPath, options);
          };
        }
        return target[property];
      },
    });
    let rawAppAsarStats = 0;
    const rawFs = new Proxy(fs, {
      get(target, property) {
        if (property === "lstatSync") {
          return (targetPath) => {
            if (targetPath.endsWith(appAsarSuffix)) {
              rawAppAsarStats += 1;
            }
            return target.lstatSync(targetPath);
          };
        }
        return target[property];
      },
    });

    assert.throws(
      () =>
        updateInstalledArtifacts(plan, {
          activationId: "activation-raw-failure",
          defaultFsOps: patchedFs,
          isElectron: true,
          loadOriginalFileSystem: () => rawFs,
          sourceCommit: "deadbeef",
          runCommand(command, args) {
            if (command === "pnpm" && args.includes("@electron/packager")) {
              const out = args.find((arg) => arg.startsWith("--out=")).slice(6);
              const payload = path.join(
                out,
                `${APP_NAME}-darwin-arm64`,
                `${APP_NAME}.app`,
              );
              const executable = path.join(payload, "Contents", "MacOS", APP_NAME);
              fs.mkdirSync(path.dirname(executable), { recursive: true });
              fs.writeFileSync(executable, "#!/bin/sh\n", { mode: 0o755 });
              fs.mkdirSync(path.join(payload, "Contents", "Resources"), {
                recursive: true,
              });
              fs.writeFileSync(
                path.join(payload, "Contents", "Resources", "app.asar"),
                "asar",
              );
            }
            if (command === "codesign" && args[0] === "--verify") {
              throw new Error("signature verification failed");
            }
          },
        }),
      /signature verification failed/,
    );
    assert.equal(
      fs.existsSync(
        path.join(plan.stateRoot, "incoming", "activation-raw-failure"),
      ),
      false,
    );
    assert.ok(rawAppAsarStats > 0);
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("producer preserves the primary failure when incoming cleanup also fails", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "capsule-cleanup-failure-"));
  const fsOps = new Proxy(fs, {
    get(target, property) {
      if (property === "rmSync") {
        return (targetPath, options) => {
          if (targetPath.endsWith("activation-cleanup-failure")) {
            throw new Error("secondary cleanup failure");
          }
          return target.rmSync(targetPath, options);
        };
      }
      return target[property];
    },
  });
  try {
    const sourceAppDir = path.join(root, "source-app");
    const codexRsDir = path.join(root, "codex-rs");
    fs.mkdirSync(sourceAppDir, { recursive: true });
    fs.mkdirSync(codexRsDir, { recursive: true });
    assert.throws(
      () =>
        updateInstalledArtifacts(
          {
            appServerBinaryPath: path.join(root, "missing-app-server"),
            codexRsDir,
            commandEnv: {},
            defaultCompactPromptSourcePath: path.join(root, "missing-compact"),
            sourceAppDir,
            stateRoot: path.join(root, "state"),
            workspace: root,
          },
          {
            activationId: "activation-cleanup-failure",
            fsOps,
            runCommand() {
              throw new Error("primary build failure");
            },
          },
        ),
      (error) => {
        assert.match(error.message, /^primary build failure;/);
        assert.match(error.message, /secondary cleanup failure/);
        assert.equal(error.cleanupError?.message, "secondary cleanup failure");
        return true;
      },
    );
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("prepared candidate removal uses Electron original-fs", async () => {
  const calls = [];
  await removeInstalledArtifactTree("/tmp/candidate", {
    defaultFsOps: {
      promises: {
        async rm() {
          throw new Error("patched fs should not be used");
        },
      },
    },
    isElectron: true,
    loadOriginalFileSystem: () => ({
      promises: {
        async rm(targetPath, options) {
          calls.push([targetPath, options]);
        },
      },
    }),
  });
  assert.deepEqual(calls, [
    ["/tmp/candidate", { force: true, recursive: true }],
  ]);
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
