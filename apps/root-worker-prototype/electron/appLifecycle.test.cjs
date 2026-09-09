const test = require("node:test");
const assert = require("node:assert/strict");

const {
  createAppRelaunchAdapter,
  createClientRelaunchNotificationHandler,
  createInstalledArtifactUpdateLifecycleAdapter,
  isClientRelaunchNotification,
} = require("./appLifecycle.cjs");

function preparedUpdate(changes = { main: false, preload: false }) {
  return {
    ok: true,
    updated: false,
    appBundlePath: "/Applications/Root Worker Prototype.app",
    buildId: "build-1",
    changes,
    preparedRoot: "/tmp/candidate",
    sourceCommit: "abc123",
    transactionId: "tx-1",
  };
}

function lifecycle(options = {}) {
  return createInstalledArtifactUpdateLifecycleAdapter({
    appExit: options.appExit,
    appServerRestart: options.appServerRestart ?? {
      requestRestart: async () => ({ ok: true, restarted: true, pid: 42 }),
    },
    appServerStop: options.appServerStop ?? {
      requestStop: async () => ({ ok: true, stopped: true }),
    },
    cleanupPreparedArtifact:
      options.cleanupPreparedArtifact ?? (async () => {}),
    reloadWindows:
      options.reloadWindows ?? (async () => ({ windowsReloaded: 1 })),
    resolvePlan:
      options.resolvePlan ??
      (async () => ({ requiresFullRelaunch: false })),
    runtimeLauncher:
      options.runtimeLauncher ??
      ({
        supported: true,
        activateHot: async () => ({ ok: true }),
        commitHot: async () => ({ ok: true }),
        prepareFull: async () => ({ ok: true }),
        rollbackHot: async () => ({ ok: true }),
      }),
    updateArtifacts:
      options.updateArtifacts ?? (async () => preparedUpdate()),
    broadcastStatus: options.broadcastStatus,
    logger: { error() {}, warn() {} },
  });
}

test("app relaunch adapter schedules one legacy full relaunch", async () => {
  const calls = [];
  const timers = [];
  const adapter = createAppRelaunchAdapter({
    app: {
      relaunch: () => calls.push("relaunch"),
      exit: (code) => calls.push(["exit", code]),
    },
    beforeExit: async (reason) => calls.push(["stop", reason]),
    setTimeout: (callback) => timers.push(callback),
  });

  assert.equal(adapter.requestRelaunch("manual").ok, true);
  assert.equal(adapter.requestRelaunch("duplicate").alreadyRequested, true);
  await timers[0]();
  assert.deepEqual(calls, [
    "relaunch",
    ["stop", "manual"],
    ["exit", 0],
  ]);
});

test("app relaunch adapter reports unsupported environments", () => {
  assert.deepEqual(createAppRelaunchAdapter({ app: {} }).requestRelaunch(), {
    ok: false,
    relaunching: false,
    reason: "Application relaunch is unavailable in this environment",
  });
});

test("client relaunch notification recognizes only lifecycle restart methods", () => {
  assert.equal(
    isClientRelaunchNotification({ method: "client/relaunch/requested" }),
    true,
  );
  assert.equal(
    isClientRelaunchNotification({
      method: "client/lifecycle/actionRequested",
      params: { action: "restart" },
    }),
    true,
  );
  assert.equal(
    isClientRelaunchNotification({
      method: "client/lifecycle/actionRequested",
      params: { action: "showDialog" },
    }),
    false,
  );
});

test("client relaunch handler rejects missing activation mode", async () => {
  const handler = createClientRelaunchNotificationHandler({});
  const result = await handler({
    method: "client/relaunch/requested",
    params: {},
  });

  assert.equal(result.ok, false);
  assert.match(result.reason, /Invalid client relaunch mode/);
});

test("installed update reports unsupported without preparing or activating", async () => {
  const calls = [];
  const adapter = lifecycle({
    resolvePlan: async () => null,
    updateArtifacts: async () => calls.push("prepare"),
    runtimeLauncher: {
      supported: true,
      activateHot: async () => calls.push("activate"),
    },
  });

  assert.deepEqual(
    await adapter.requestUpdateAndRelaunch("unsupported", "hot"),
    { ok: false, unsupported: true },
  );
  assert.deepEqual(calls, []);
});

test("planning failure is typed, retains request id, and has no side effects", async () => {
  const calls = [];
  const adapter = lifecycle({
    resolvePlan: async () => {
      throw new Error("planning failed");
    },
    updateArtifacts: async () => calls.push("prepare"),
    runtimeLauncher: {
      supported: true,
      activateHot: async () => calls.push("activate"),
    },
  });

  const result = await adapter.requestUpdateAndRelaunch(
    "developer request",
    "hot",
    "request-1",
  );

  assert.equal(result.ok, false);
  assert.equal(result.requestId, "request-1");
  assert.equal(result.partial, false);
  assert.match(result.reason, /planning failed/);
  assert.deepEqual(calls, []);
});

test("hot activation copies into Launcher state, cleans producer, restarts, reloads, and commits", async () => {
  const calls = [];
  let request;
  const adapter = lifecycle({
    runtimeLauncher: {
      supported: true,
      activateHot: async (value) => {
        request = value;
        calls.push("activate");
        return { ok: true };
      },
      commitHot: async (transactionId) => {
        calls.push(["commit", transactionId]);
        return { ok: true };
      },
      rollbackHot: async () => {
        throw new Error("rollback should not run");
      },
    },
    cleanupPreparedArtifact: async (preparedRoot) =>
      calls.push(["cleanup", preparedRoot]),
    appServerRestart: {
      requestRestart: async () => {
        calls.push("restart");
        return { ok: true, restarted: true, pid: 42 };
      },
    },
    reloadWindows: async () => {
      calls.push("reload");
      return { windowsReloaded: 1 };
    },
  });

  const result = await adapter.requestUpdateAndRelaunch("developer request", "hot");

  assert.equal(result.ok, true);
  assert.equal(result.updated, true);
  assert.deepEqual(calls, [
    "activate",
    "restart",
    "reload",
    ["commit", "tx-1"],
    ["cleanup", "/tmp/candidate"],
  ]);
  assert.deepEqual(request, {
    schemaVersion: 1,
    transactionId: "tx-1",
    buildId: "build-1",
    sourceCommit: "abc123",
    preparedRoot: "/tmp/candidate",
    appBundlePath: "/Applications/Root Worker Prototype.app",
    reason: "developer request",
    changes: { main: false, preload: false },
  });
  assert.equal(Object.hasOwn(request, "mode"), false);
});

test("hot activation rejects main or preload changes before invoking Launcher", async () => {
  const calls = [];
  const adapter = lifecycle({
    updateArtifacts: async () => preparedUpdate({ main: false, preload: true }),
    runtimeLauncher: {
      supported: true,
      activateHot: async () => calls.push("activate"),
    },
    cleanupPreparedArtifact: async () => calls.push("cleanup"),
  });

  const result = await adapter.requestUpdateAndRelaunch("shell changed", "hot");

  assert.equal(result.ok, false);
  assert.match(result.reason, /main or preload changed/);
  assert.deepEqual(calls, ["cleanup"]);
});

test("hot activation failure rolls back and retains the old runtime", async () => {
  const calls = [];
  const adapter = lifecycle({
    runtimeLauncher: {
      supported: true,
      activateHot: async () => {
        calls.push("activate");
        throw new Error("activation failed");
      },
      rollbackHot: async (transactionId) => {
        calls.push(["rollback", transactionId]);
        return { ok: true };
      },
    },
    cleanupPreparedArtifact: async () => calls.push("cleanup"),
    appServerRestart: {
      requestRestart: async (reason) => {
        calls.push(["restart", reason]);
        return { ok: true, restarted: true };
      },
    },
    reloadWindows: async ({ reason }) => {
      calls.push(["reload", reason]);
      return { windowsReloaded: 1 };
    },
  });

  const result = await adapter.requestUpdateAndRelaunch("hot failure", "hot");

  assert.equal(result.ok, false);
  assert.match(result.reason, /activation failed/);
  assert.deepEqual(calls, [
    "activate",
    ["rollback", "tx-1"],
    ["restart", "rollback: hot failure"],
    ["reload", "rollback: hot failure"],
    "cleanup",
  ]);
});

test("commit failure rolls back the activated hot runtime", async () => {
  const calls = [];
  const adapter = lifecycle({
    runtimeLauncher: {
      supported: true,
      activateHot: async () => ({ ok: true }),
      commitHot: async () => {
        calls.push("commit");
        throw new Error("commit failed");
      },
      rollbackHot: async () => {
        calls.push("rollback");
        return { ok: true };
      },
    },
    appServerRestart: {
      requestRestart: async () => ({ ok: true, restarted: true }),
    },
  });

  const result = await adapter.requestUpdateAndRelaunch("commit", "hot");

  assert.equal(result.ok, false);
  assert.match(result.reason, /commit failed/);
  assert.deepEqual(calls, ["commit", "rollback"]);
});

test("full activation prepares Launcher ownership before stopping and exiting 75", async () => {
  const calls = [];
  let request;
  const adapter = lifecycle({
    runtimeLauncher: {
      supported: true,
      prepareFull: async (value) => {
        request = value;
        calls.push("prepare");
        return { ok: true };
      },
    },
    cleanupPreparedArtifact: async () => calls.push("cleanup"),
    appServerStop: {
      requestStop: async () => {
        calls.push("stop");
        return { ok: true, stopped: true };
      },
    },
    appExit: (code) => calls.push(["exit", code]),
  });

  const result = await adapter.requestUpdateAndRelaunch("full update", "full");

  assert.equal(result.ok, true);
  assert.equal(result.relaunching, true);
  assert.equal(result.updated, true);
  assert.deepEqual(calls, ["prepare", "cleanup", "stop", ["exit", 75]]);
  assert.equal(Object.hasOwn(request, "mode"), false);
});

test("full activation aborts the prepared transaction when app-server cannot stop", async () => {
  const calls = [];
  const adapter = lifecycle({
    runtimeLauncher: {
      supported: true,
      prepareFull: async () => {
        calls.push("prepare");
        return { ok: true };
      },
      abortFull: async (transactionId) => {
        calls.push(["abort", transactionId]);
        return { ok: true };
      },
    },
    cleanupPreparedArtifact: async () => calls.push("cleanup"),
    appServerStop: {
      requestStop: async () => {
        calls.push("stop");
        return { ok: false, stopped: false, reason: "stop failed" };
      },
    },
    appExit: () => calls.push("exit"),
  });

  const result = await adapter.requestUpdateAndRelaunch("full update", "full");

  assert.equal(result.ok, false);
  assert.match(result.reason, /stop failed/);
  assert.equal(result.abort.ok, true);
  assert.deepEqual(calls, [
    "prepare",
    "cleanup",
    "stop",
    ["abort", "tx-1"],
  ]);
});

test("same-mode requests coalesce while cross-mode requests conflict", async () => {
  let resolveUpdate;
  const adapter = lifecycle({
    updateArtifacts: () =>
      new Promise((resolve) => {
        resolveUpdate = resolve;
      }),
  });

  const first = adapter.requestUpdateAndRelaunch("first", "hot");
  const duplicate = adapter.requestUpdateAndRelaunch("second", "hot");
  const conflict = await adapter.requestUpdateAndRelaunch("third", "full");
  assert.equal(conflict.conflict, true);
  assert.equal(conflict.requestedMode, "full");
  assert.equal(conflict.executingMode, "hot");

  resolveUpdate(preparedUpdate());
  assert.equal((await first).ok, true);
  assert.equal((await duplicate).alreadyRequested, true);
});
