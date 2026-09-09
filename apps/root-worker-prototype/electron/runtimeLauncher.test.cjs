"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const test = require("node:test");
const {
  createRuntimeLauncher,
  invokeLauncher,
} = require("./runtimeLauncher.cjs");

test("runtime launcher adapter prepares a Capsule with status CAS fields", () => {
  const calls = [];
  let request = null;
  const launcher = createRuntimeLauncher({
    env: {
      RUNTIME_CAPSULE_LAUNCHER_PATH: "/Applications/Test.app/Contents/MacOS/MorpheusLauncher",
      RUNTIME_CAPSULE_LAUNCHER_HOME: "/tmp/launcher-state",
    },
    spawnSync(command, args) {
      calls.push({ command, args });
      if (args.includes("status")) {
        return {
          status: 0,
          stdout:
            '{"ok":true,"result":{"control":{"revision":7,"executorEpoch":3}}}',
          stderr: "",
        };
      }
      const requestIndex = args.indexOf("--request");
      request = JSON.parse(fs.readFileSync(args[requestIndex + 1], "utf8"));
      return {
        status: 0,
        stdout: JSON.stringify({
          ok: true,
          result: {
            disposition: "prepared",
            activationId: "activation-1",
            releaseId: `sha256:${"a".repeat(64)}`,
            control: { activation: { phase: "prepared" } },
          },
        }),
        stderr: "",
      };
    },
  });

  const result = launcher.prepareActivation({
    activationId: "activation-1",
    releaseId: `sha256:${"a".repeat(64)}`,
    manifest: { target: { os: "darwin", arch: "arm64" } },
    reason: "update",
  });

  assert.equal(result.disposition, "prepared");
  assert.equal(result.control.activation.phase, "prepared");
  assert.deepEqual(request, {
    schemaVersion: 1,
    activationId: "activation-1",
    releaseId: `sha256:${"a".repeat(64)}`,
    expectedRevision: 7,
    expectedExecutorEpoch: 3,
    target: { os: "darwin", arch: "arm64" },
    reason: "update",
  });
  assert.equal(calls.length, 2);
  assert.ok(calls[1].args.includes("prepare-activation"));
});

test("runtime launcher mutation commands also use current CAS fields", () => {
  let request = null;
  const launcher = createRuntimeLauncher({
    env: {
      RUNTIME_CAPSULE_LAUNCHER_PATH: "/launcher",
      RUNTIME_CAPSULE_LAUNCHER_HOME: "/state",
    },
    spawnSync(_command, args) {
      if (args.includes("status")) {
        return {
          status: 0,
          stdout:
            '{"ok":true,"result":{"control":{"revision":9,"executorEpoch":4}}}',
          stderr: "",
        };
      }
      request = JSON.parse(
        fs.readFileSync(args[args.indexOf("--request") + 1], "utf8"),
      );
      return { status: 0, stdout: '{"ok":true,"result":{}}', stderr: "" };
    },
  });
  launcher.requestRollback("activation-2", "health failure");
  assert.deepEqual(request, {
    activationId: "activation-2",
    expectedRevision: 9,
    expectedExecutorEpoch: 4,
    reason: "health failure",
  });
});

test("runtime launcher rejects prepare results for another activation", () => {
  const launcher = createRuntimeLauncher({
    env: {
      RUNTIME_CAPSULE_LAUNCHER_PATH: "/launcher",
      RUNTIME_CAPSULE_LAUNCHER_HOME: "/state",
    },
    spawnSync(_command, args) {
      if (args.includes("status")) {
        return {
          status: 0,
          stdout:
            '{"ok":true,"result":{"control":{"revision":9,"executorEpoch":4}}}',
          stderr: "",
        };
      }
      return {
        status: 0,
        stdout: JSON.stringify({
          ok: true,
          result: {
            disposition: "prepared",
            activationId: "another-activation",
            releaseId: `sha256:${"a".repeat(64)}`,
            control: {},
          },
        }),
        stderr: "",
      };
    },
  });

  assert.throws(
    () =>
      launcher.prepareActivation({
        activationId: "activation-1",
        releaseId: `sha256:${"a".repeat(64)}`,
        manifest: { target: { os: "darwin", arch: "arm64" } },
        reason: "update",
      }),
    /does not match the requested activation/,
  );
});

test("runtime launcher rejects unknown prepare dispositions", () => {
  const launcher = createRuntimeLauncher({
    env: {
      RUNTIME_CAPSULE_LAUNCHER_PATH: "/launcher",
      RUNTIME_CAPSULE_LAUNCHER_HOME: "/state",
    },
    spawnSync(_command, args) {
      if (args.includes("status")) {
        return {
          status: 0,
          stdout:
            '{"ok":true,"result":{"control":{"revision":9,"executorEpoch":4}}}',
          stderr: "",
        };
      }
      return {
        status: 0,
        stdout: JSON.stringify({
          ok: true,
          result: {
            disposition: "retry_later",
            activationId: "activation-1",
            releaseId: `sha256:${"a".repeat(64)}`,
            control: {},
          },
        }),
        stderr: "",
      };
    },
  });

  assert.throws(
    () =>
      launcher.prepareActivation({
        activationId: "activation-1",
        releaseId: `sha256:${"a".repeat(64)}`,
        manifest: { target: { os: "darwin", arch: "arm64" } },
        reason: "update",
      }),
    /unsupported disposition/,
  );
});

test("runtime launcher is unavailable without an injected control path", () => {
  const launcher = createRuntimeLauncher({
    env: { MORPHEUS_HOME: "/tmp/morpheus" },
  });
  assert.equal(launcher.supported, false);
  assert.equal(launcher.launcherPath, null);
});

test("runtime launcher surfaces structured command errors", () => {
  assert.throws(
    () =>
      invokeLauncher("/launcher", ["status"], {
        spawnSync: () => ({
          status: 1,
          stdout: '{"ok":false,"error":{"message":"state conflict"}}',
          stderr: "",
        }),
      }),
    /state conflict/,
  );
});
