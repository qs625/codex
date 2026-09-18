"use strict";

const assert = require("node:assert/strict");
const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const {
  APP_NAME,
  COMPUTER_USE_HELPER_APP_RELATIVE_PATH,
  COMPUTER_USE_HELPER_BUNDLE_IDENTIFIER,
  COMPUTER_USE_HELPER_EXECUTABLE_RELATIVE_PATH,
  COMPUTER_USE_NATIVE_HELPER_EXECUTABLE_RELATIVE_PATH,
  COMPUTER_USE_PACKAGED_MCP_CONFIG_RELATIVE_PATH,
  COMPUTER_USE_NATIVE_RESOURCE_RELATIVE_PATH,
  GENERATED_SOURCE_DIR_NAMES,
  PAYLOAD_EXECUTABLE_RELATIVE_PATH,
  materializeInstalledArtifactWorkerBundle,
  normalizeRuntimeCapsuleTree,
  removeInstalledArtifactTree,
  resolveInstalledArtifactUpdatePlan,
  resolveInstalledArtifactFileSystem,
  resolveRuntimeLauncherStateRoot,
  runInstalledArtifactWorker,
  stagePayloadResources,
  updateInstalledArtifacts,
} = require("./installedArtifactUpdate.cjs");

function writeComputerUseHelperSources(workspace, sourceAppDir) {
  const scriptsDir = path.join(workspace, "scripts");
  const electronDir = path.join(sourceAppDir, "electron");
  fs.mkdirSync(scriptsDir, { recursive: true });
  fs.mkdirSync(electronDir, { recursive: true });
  fs.writeFileSync(
    path.join(scriptsDir, "morpheus-computer-use-mcp.mjs"),
    "export {}\n",
  );
  fs.writeFileSync(
    path.join(electronDir, "computerUse.cjs"),
    "module.exports = {}\n",
  );
  fs.writeFileSync(
    path.join(electronDir, "computerUseMacNative.swift"),
    "// native bridge",
  );
}

function compileFakeNativeHelper({ fsOps = fs, sourcePath, targetPath }) {
  assert.match(sourcePath, /computerUseMacNative\.swift$/);
  fsOps.mkdirSync(path.dirname(targetPath), { recursive: true });
  fsOps.writeFileSync(targetPath, "native-helper", { mode: 0o755 });
  fsOps.chmodSync(targetPath, 0o755);
}

function handleFakeSwiftc(command, args) {
  if (command !== "swiftc") {
    return false;
  }
  assert.ok(args.some((arg) => arg.endsWith("computerUseMacNative.swift")));
  assert.ok(args.includes("-framework"));
  assert.ok(args.includes("AppKit"));
  assert.ok(args.includes("ApplicationServices"));
  assert.ok(args.includes("ScreenCaptureKit"));
  const outputIndex = args.indexOf("-o");
  assert.notEqual(outputIndex, -1);
  const targetPath = args[outputIndex + 1];
  fs.mkdirSync(path.dirname(targetPath), { recursive: true });
  fs.writeFileSync(targetPath, "native-helper", { mode: 0o755 });
  return true;
}

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

test("worker bundle preserves a source read failure and cleans the raw destination", () => {
  const bundleRoots = [];
  const removedRoots = [];
  const readFailure = new Error("packaged worker source read failed");
  const sourceFs = {
    readFileSync(sourcePath) {
      if (sourcePath.endsWith("installedArtifactUpdate.cjs")) {
        throw readFailure;
      }
      return fs.readFileSync(
        path.join(__dirname, path.basename(sourcePath)),
      );
    },
  };
  const destinationFs = new Proxy(fs, {
    get(target, property) {
      if (property === "mkdtempSync") {
        return (prefix) => {
          const bundleRoot = target.mkdtempSync(prefix);
          bundleRoots.push(bundleRoot);
          return bundleRoot;
        };
      }
      if (property === "rmSync") {
        return (targetPath, options) => {
          removedRoots.push(targetPath);
          return target.rmSync(targetPath, options);
        };
      }
      return target[property];
    },
  });

  assert.throws(
    () =>
      materializeInstalledArtifactWorkerBundle({
        destinationFsOps: destinationFs,
        sourceFsOps: sourceFs,
        workerSourceDirectory:
          "/Applications/Morpheus.app/Contents/Resources/app.asar/electron",
      }),
    (error) => error === readFailure,
  );
  assert.equal(bundleRoots.length, 1);
  assert.deepEqual(removedRoots, bundleRoots);
  assert.equal(fs.existsSync(bundleRoots[0]), false);
});

test("worker bundle keeps a destination write failure when raw cleanup also fails", () => {
  const writeFailure = new Error("raw worker destination write failed");
  const cleanupFailure = new Error("raw worker destination cleanup failed");
  let writeCount = 0;
  let bundleRoot = null;
  const destinationFs = new Proxy(fs, {
    get(target, property) {
      if (property === "mkdtempSync") {
        return (prefix) => {
          bundleRoot = target.mkdtempSync(prefix);
          return bundleRoot;
        };
      }
      if (property === "writeFileSync") {
        return (...args) => {
          writeCount += 1;
          if (writeCount === 2) {
            throw writeFailure;
          }
          return target.writeFileSync(...args);
        };
      }
      if (property === "rmSync") {
        return () => {
          throw cleanupFailure;
        };
      }
      return target[property];
    },
  });

  try {
    assert.throws(
      () =>
        materializeInstalledArtifactWorkerBundle({
          destinationFsOps: destinationFs,
          sourceFsOps: fs,
        }),
      (error) => {
        assert.equal(error, writeFailure);
        assert.match(error.message, /^raw worker destination write failed;/);
        assert.match(error.message, /raw worker destination cleanup failed/);
        assert.equal(error.cleanupError, cleanupFailure);
        return true;
      },
    );
    assert.equal(writeCount, 2);
    assert.ok(bundleRoot);
    assert.equal(fs.readdirSync(bundleRoot).length, 1);
  } finally {
    if (bundleRoot) {
      fs.rmSync(bundleRoot, { force: true, recursive: true });
    }
  }
});

test("worker bundle defaults work from an ordinary Node source tree", () => {
  const modulePath = path.join(__dirname, "installedArtifactUpdate.cjs");
  const script = `
    const path = require("node:path");
    const {
      materializeInstalledArtifactWorkerBundle,
      runInstalledArtifactWorker,
    } = require(${JSON.stringify(modulePath)});
    const bundlePath = materializeInstalledArtifactWorkerBundle();
    runInstalledArtifactWorker(
      "resolvePlan",
      {
        env: {},
        isPackaged: false,
        platform: "linux",
        resourcesPath: null,
      },
      {
        workerPath: path.join(
          bundlePath,
          "installedArtifactUpdateWorker.cjs",
        ),
      },
    ).then(
      (result) => {
        if (result !== null) {
          throw new Error("unexpected worker result");
        }
      },
      (error) => {
        console.error(error);
        process.exitCode = 1;
      },
    );
  `;
  const result = spawnSync(process.execPath, ["-e", script], {
    encoding: "utf8",
  });
  assert.equal(result.status, 0, result.stderr);
});

test("Electron worker bundle reads packaged sources and writes a raw loadable bundle", async () => {
  const packagedSourceDirectory =
    "/Applications/Morpheus.app/Contents/Resources/app.asar/electron";
  const sourceReads = [];
  const destinationCalls = [];
  const sourceFs = {
    readFileSync(sourcePath) {
      assert.ok(sourcePath.startsWith(`${packagedSourceDirectory}${path.sep}`));
      sourceReads.push(sourcePath);
      return fs.readFileSync(
        path.join(__dirname, path.basename(sourcePath)),
      );
    },
  };
  const destinationFs = new Proxy(fs, {
    get(target, property) {
      if (property === "readFileSync") {
        return (targetPath, ...args) => {
          if (targetPath.includes(`${path.sep}app.asar${path.sep}`)) {
            const error = new Error(`not a directory, open '${targetPath}'`);
            error.code = "ENOTDIR";
            throw error;
          }
          return target.readFileSync(targetPath, ...args);
        };
      }
      if (
        property === "mkdtempSync" ||
        property === "writeFileSync" ||
        property === "rmSync"
      ) {
        return (...args) => {
          destinationCalls.push([property, args[0]]);
          return target[property](...args);
        };
      }
      return target[property];
    },
  });
  const bundlePath = materializeInstalledArtifactWorkerBundle({
    sourceFsOps: sourceFs,
    isElectron: true,
    loadOriginalFileSystem: () => destinationFs,
    workerSourceDirectory: packagedSourceDirectory,
  });

  assert.equal(sourceReads.length, 5);
  assert.equal(
    destinationCalls.filter(([operation]) => operation === "writeFileSync")
      .length,
    5,
  );
  assert.ok(
    destinationCalls.every(
      ([operation, targetPath]) =>
        operation === "mkdtempSync" ||
        operation === "rmSync" ||
        targetPath.startsWith(`${bundlePath}${path.sep}`),
    ),
  );
  for (const sourcePath of sourceReads) {
    const fileName = path.basename(sourcePath);
    assert.deepEqual(
      fs.readFileSync(path.join(bundlePath, fileName)),
      fs.readFileSync(path.join(__dirname, fileName)),
    );
    assert.equal(
      fs.statSync(path.join(bundlePath, fileName)).mode & 0o777,
      0o600,
    );
  }
  assert.equal(
    await runInstalledArtifactWorker(
      "resolvePlan",
      {
        env: {},
        isPackaged: false,
        platform: "linux",
        resourcesPath: null,
      },
      {
        workerPath: path.join(
          bundlePath,
          "installedArtifactUpdateWorker.cjs",
        ),
      },
    ),
    null,
  );
});

test("stagePayloadResources installs app-server, defaults, native bridge, and Computer Use helper", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "capsule-resources-"));
  try {
    const workspace = path.join(root, "source");
    const sourceAppDir = path.join(workspace, "apps", "root-worker-prototype");
    const appServer = path.join(root, "source-app-server");
    const compact = path.join(root, "source-compact.md");
    const native = path.join(sourceAppDir, "electron", "computerUseMacNative.swift");
    fs.writeFileSync(appServer, "binary", { mode: 0o755 });
    fs.writeFileSync(compact, "prompt");
    writeComputerUseHelperSources(workspace, sourceAppDir);
    const target = path.join(root, "resources");
    stagePayloadResources(
      {
        appServerBinaryPath: appServer,
        computerUseNativeScriptSourcePath: native,
        defaultCompactPromptSourcePath: compact,
        sourceAppDir,
        workspace,
      },
      target,
      { compileNativeHelper: compileFakeNativeHelper, fsOps: fs },
    );
    assert.equal(fs.readFileSync(path.join(target, "bin", "app-server"), "utf8"), "binary");
    assert.equal(
      fs.readFileSync(
        path.join(target, "default-config", "compact", "COMPACT.md"),
        "utf8",
      ),
      "prompt",
    );
    assert.equal(
      fs.readFileSync(
        path.join(
          target,
          ...COMPUTER_USE_NATIVE_RESOURCE_RELATIVE_PATH.split(path.sep),
        ),
        "utf8",
      ),
      "// native bridge",
    );
    const config = fs.readFileSync(
      path.join(
        target,
        ...COMPUTER_USE_PACKAGED_MCP_CONFIG_RELATIVE_PATH.split(path.sep),
      ),
      "utf8",
    );
    assert.match(config, /\[mcp_servers\.computer_use\]/);
    assert.match(config, /MORPHEUS_COMPUTER_USE_HELPER_EXECUTABLE/);
    assert.doesNotMatch(config, /computerUseMacNative\.swift/);
    const helperExecutable = path.join(
      target,
      ...COMPUTER_USE_HELPER_EXECUTABLE_RELATIVE_PATH.split(path.sep),
    );
    const nativeHelperExecutable = path.join(
      target,
      ...COMPUTER_USE_NATIVE_HELPER_EXECUTABLE_RELATIVE_PATH.split(path.sep),
    );
    assert.ok(fs.statSync(helperExecutable).isFile());
    assert.equal(fs.statSync(helperExecutable).mode & 0o111, 0o111);
    assert.ok(fs.statSync(nativeHelperExecutable).isFile());
    assert.equal(fs.statSync(nativeHelperExecutable).mode & 0o111, 0o111);
    assert.equal(fs.readFileSync(nativeHelperExecutable, "utf8"), "native-helper");
    const helperLauncher = fs.readFileSync(helperExecutable, "utf8");
    assert.match(helperLauncher, /ELECTRON_RUN_AS_NODE=1/);
    assert.match(helperLauncher, /PAYLOAD_ELECTRON=/);
    assert.match(helperLauncher, /HELPER_BUNDLE_DIR=/);
    assert.match(helperLauncher, /MORPHEUS_COMPUTER_USE_NATIVE_HELPER_EXECUTABLE/);
    assert.match(helperLauncher, /morpheus-computer-use-native/);
    assert.doesNotMatch(helperLauncher, /exec node/);
    assert.match(
      fs.readFileSync(
        path.join(
          target,
          ...COMPUTER_USE_HELPER_APP_RELATIVE_PATH.split(path.sep),
          "Contents",
          "Info.plist",
        ),
        "utf8",
      ),
      new RegExp(COMPUTER_USE_HELPER_BUNDLE_IDENTIFIER),
    );
    assert.equal(
      fs.readFileSync(
        path.join(
          target,
          ...COMPUTER_USE_HELPER_APP_RELATIVE_PATH.split(path.sep),
          "Contents",
          "Resources",
          "server",
          "computerUse.cjs",
        ),
        "utf8",
      ),
      "module.exports = {}\n",
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
    writeComputerUseHelperSources(path.join(root, "source"), sourceAppDir);
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
        if (handleFakeSwiftc(command, args)) {
          return;
        }
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
    assert.ok(
      packagerArgs.includes(
        `--extra-resource=${path.join(
          root,
          "state",
          "incoming",
          "activation-test",
          ".resources",
          "computer-use-helper",
        )}`,
      ),
    );
    assert.ok(
      packagerArgs.includes(
        `--extra-resource=${path.join(
          root,
          "state",
          "incoming",
          "activation-test",
          ".resources",
          "native",
        )}`,
      ),
    );
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
    writeComputerUseHelperSources(path.join(root, "source"), sourceAppDir);
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
            if (handleFakeSwiftc(command, args)) {
              return;
            }
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
