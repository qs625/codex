"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const {
  createClientRelaunchNotificationHandler,
  createInstalledArtifactUpdateLifecycleAdapter,
  observeClientRelaunchResult,
} = require("./appLifecycle.cjs");

test("generic lifecycle notification requests one complete Capsule restart", async () => {
  const calls = [];
  const handler = createClientRelaunchNotificationHandler({
    installedArtifactUpdate: {
      async requestUpdateAndRelaunch(reason, requestId) {
        calls.push({ reason, requestId });
        return { ok: true, updated: true, requestId };
      },
    },
  });
  const result = await handler({
    method: "client/relaunch/requested",
    params: { requestId: "call-1", reason: "new runtime" },
  });
  assert.equal(result.ok, true);
  assert.deepEqual(calls, [{ reason: "new runtime", requestId: "call-1" }]);
});

test("legacy hot notification is rejected without side effects", async () => {
  let called = false;
  const handler = createClientRelaunchNotificationHandler({
    installedArtifactUpdate: {
      async requestUpdateAndRelaunch() {
        called = true;
      },
    },
  });
  const result = await handler({
    method: "client/relaunch/requested",
    params: { mode: "hot", requestId: "legacy-hot" },
  });
  assert.equal(result.legacyModeRejected, true);
  assert.equal(called, false);
});

test("installed update prepares Capsule, stops backend, and exits 75", async () => {
  const calls = [];
  const adapter = createInstalledArtifactUpdateLifecycleAdapter({
    appExit(code) {
      calls.push(["exit", code]);
    },
    appServerStop: {
      async requestStop(reason) {
        calls.push(["stop", reason]);
        return { ok: true };
      },
    },
    resolvePlan: async () => ({ workspace: "/source" }),
    runtimeLauncher: {
      supported: true,
      async prepareActivation(request) {
        calls.push(["prepare", request]);
        return {
          disposition: "prepared",
          activationId: request.activationId,
          releaseId: request.releaseId,
          control: { activation: { phase: "prepared" } },
        };
      },
    },
    updateArtifacts: async () => ({
      ok: true,
      activationId: "activation-1",
      releaseId: `sha256:${"a".repeat(64)}`,
      manifest: { target: { os: "darwin", arch: "arm64" } },
    }),
  });
  const result = await adapter.requestUpdateAndRelaunch(
    "apply update",
    "call-1",
  );
  assert.equal(result.ok, true);
  assert.equal(result.relaunch.exitCode, 75);
  assert.deepEqual(calls.map(([name]) => name), ["prepare", "stop", "exit"]);
});

test("backend stop failure cancels prepared activation and does not exit", async () => {
  const calls = [];
  const adapter = createInstalledArtifactUpdateLifecycleAdapter({
    appExit() {
      calls.push("exit");
    },
    appServerStop: {
      async requestStop() {
        return { ok: false, reason: "busy" };
      },
    },
    resolvePlan: async () => ({}),
    runtimeLauncher: {
      supported: true,
      async prepareActivation() {
        return {
          disposition: "prepared",
          activationId: "activation-2",
          releaseId: `sha256:${"b".repeat(64)}`,
          control: { activation: { phase: "prepared" } },
        };
      },
      async cancelActivation(activationId) {
        calls.push(`cancel:${activationId}`);
        return { ok: true };
      },
    },
    updateArtifacts: async () => ({
      ok: true,
      activationId: "activation-2",
      releaseId: `sha256:${"b".repeat(64)}`,
      manifest: { target: { os: "darwin", arch: "arm64" } },
    }),
    logger: { error() {}, warn() {} },
  });
  const result = await adapter.requestUpdateAndRelaunch("update");
  assert.equal(result.ok, false);
  assert.deepEqual(calls, ["cancel:activation-2"]);
});

test("terminal failed prepare does not stop backend or exit", async () => {
  const calls = [];
  const adapter = createInstalledArtifactUpdateLifecycleAdapter({
    appExit() {
      calls.push("exit");
    },
    appServerStop: {
      async requestStop() {
        calls.push("stop");
        return { ok: true };
      },
    },
    resolvePlan: async () => ({}),
    runtimeLauncher: {
      supported: true,
      async prepareActivation(request) {
        return {
          disposition: "terminal_failed",
          activationId: request.activationId,
          releaseId: request.releaseId,
          control: {
            activation: { receipt: { outcome: "rolled_back" } },
          },
        };
      },
    },
    updateArtifacts: async () => ({
      ok: true,
      activationId: "activation-terminal",
      releaseId: `sha256:${"c".repeat(64)}`,
      manifest: { target: { os: "darwin", arch: "arm64" } },
    }),
    logger: { error() {}, warn() {} },
  });

  const result = await adapter.requestUpdateAndRelaunch("update");
  assert.equal(result.ok, false);
  assert.match(result.reason, /terminally unavailable/);
  assert.deepEqual(calls, []);
});

test("prepare identity mismatch does not stop backend or exit", async () => {
  const calls = [];
  const adapter = createInstalledArtifactUpdateLifecycleAdapter({
    appExit() {
      calls.push("exit");
    },
    appServerStop: {
      async requestStop() {
        calls.push("stop");
        return { ok: true };
      },
    },
    resolvePlan: async () => ({}),
    runtimeLauncher: {
      supported: true,
      async prepareActivation(request) {
        return {
          disposition: "prepared",
          activationId: `${request.activationId}-wrong`,
          releaseId: request.releaseId,
          control: { activation: { phase: "prepared" } },
        };
      },
    },
    updateArtifacts: async () => ({
      ok: true,
      activationId: "activation-mismatch",
      releaseId: `sha256:${"e".repeat(64)}`,
      manifest: { target: { os: "darwin", arch: "arm64" } },
    }),
    logger: { error() {}, warn() {} },
  });

  const result = await adapter.requestUpdateAndRelaunch("update");
  assert.equal(result.ok, false);
  assert.match(result.reason, /does not match the produced candidate/);
  assert.deepEqual(calls, []);
});

test("already committed prepare reports success without another exit", async () => {
  const calls = [];
  const adapter = createInstalledArtifactUpdateLifecycleAdapter({
    appExit() {
      calls.push("exit");
    },
    appServerStop: {
      async requestStop() {
        calls.push("stop");
        return { ok: true };
      },
    },
    cleanupPreparedArtifact: async (incomingRoot) => {
      calls.push(`cleanup:${incomingRoot}`);
    },
    resolvePlan: async () => ({}),
    runtimeLauncher: {
      supported: true,
      async prepareActivation(request) {
        return {
          disposition: "already_committed",
          activationId: request.activationId,
          releaseId: request.releaseId,
          control: {
            activation: { receipt: { outcome: "committed" } },
          },
        };
      },
    },
    updateArtifacts: async () => ({
      ok: true,
      activationId: "activation-committed",
      incomingRoot: "/tmp/incoming-committed",
      releaseId: `sha256:${"d".repeat(64)}`,
      manifest: { target: { os: "darwin", arch: "arm64" } },
    }),
  });

  const result = await adapter.requestUpdateAndRelaunch("update");
  assert.equal(result.ok, true);
  assert.equal(result.alreadyCommitted, true);
  assert.equal(result.relaunching, false);
  assert.deepEqual(calls, ["cleanup:/tmp/incoming-committed"]);
});

test("missing source workspace returns typed unavailable result", async () => {
  const adapter = createInstalledArtifactUpdateLifecycleAdapter({
    resolvePlan: async () => ({
      disabled: true,
      reason: "source workspace absent",
    }),
  });
  const result = await adapter.requestUpdateAndRelaunch("update");
  assert.equal(result.disabled, true);
  assert.match(result.reason, /source workspace absent/);
});

test("observer broadcasts mode-free lifecycle completion", async () => {
  const statuses = [];
  const result = await observeClientRelaunchResult(
    Promise.resolve({ ok: true, requestId: "call-3", updated: true }),
    { broadcastStatus: (status) => statuses.push(status) },
  );
  assert.equal(result.ok, true);
  assert.deepEqual(statuses[0].lifecycle, {
    type: "clientRelaunch",
    phase: "completed",
    requestId: "call-3",
    reason: null,
  });
  assert.equal("mode" in statuses[0].lifecycle, false);
});
