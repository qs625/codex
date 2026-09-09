const test = require("node:test");
const assert = require("node:assert/strict");

const {
  createAppRelaunchAdapter,
  createClientRelaunchNotificationHandler,
  createInstalledArtifactUpdateLifecycleAdapter,
  createRendererReloadLifecycleAdapter,
  isClientRelaunchNotification,
  observeClientRelaunchResult,
} = require("./appLifecycle.cjs");

test("client lifecycle recognizes narrow restart notifications", () => {
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

test("full activation is prepared by launcher then exits with coordination code 75", async () => {
  const calls = [];
  const adapter = createInstalledArtifactUpdateLifecycleAdapter({
    appExit: (code) => calls.push(["exit", code]),
    appServerStop: {
      requestStop: async () => {
        calls.push(["stop"]);
        return { ok: true };
      },
    },
    cleanupPreparedArtifact: async () => calls.push(["cleanup"]),
    releasePreparedArtifactLease: async () =>
      calls.push(["release-lease"]),
    runtimeLauncher: {
      supported: true,
      prepareFull: async (request) => {
        calls.push(["prepare-full", request]);
        return { accepted: true };
      },
    },
    resolvePlan: async () => ({
      appBundlePath: "/Applications/Morpheus.app",
      requiresFullRelaunch: true,
    }),
    updateArtifacts: async () => preparedUpdate(),
  });

  const result = await adapter.requestUpdateAndRelaunch(
    "runtime update",
    "full",
    "request-1",
    "thread-1",
  );

  assert.equal(result.ok, true);
  assert.equal(result.relaunching, true);
  assert.deepEqual(calls.map(([kind]) => kind), [
    "prepare-full",
    "release-lease",
    "stop",
    "exit",
  ]);
  assert.equal(calls[0][1].requestedByThreadId, "thread-1");
  assert.deepEqual(calls.at(-1), ["exit", 75]);
});

test("activation request preserves missing, null, and valid thread provenance", async () => {
  for (const { requesterArgs, expected } of [
    { requesterArgs: [], expected: null },
    { requesterArgs: [null], expected: null },
    { requesterArgs: ["  thread-1  "], expected: "thread-1" },
  ]) {
    let activationRequest;
    const adapter = createInstalledArtifactUpdateLifecycleAdapter({
      appExit: () => {},
      appServerStop: { requestStop: async () => ({ ok: true }) },
      runtimeLauncher: {
        supported: true,
        prepareFull: async (request) => {
          activationRequest = request;
          return { ok: true };
        },
      },
      resolvePlan: async () => ({
        appBundlePath: "/Applications/Morpheus.app",
        requiresFullRelaunch: true,
      }),
      updateArtifacts: async () => preparedUpdate(),
    });

    const result = await adapter.requestUpdateAndRelaunch(
      "runtime update",
      "full",
      "request-1",
      ...requesterArgs,
    );

    assert.equal(result.ok, true);
    assert.equal(activationRequest.requestedByThreadId, expected);
  }
});

test("installed activation rejects present-invalid thread provenance before preparation", async () => {
  for (const requestedByThreadId of [
    undefined,
    "",
    "   ",
    42,
    false,
    {},
  ]) {
    const calls = [];
    const adapter = createInstalledArtifactUpdateLifecycleAdapter({
      resolvePlan: async () => calls.push("resolve-plan"),
      updateArtifacts: async () => calls.push("prepare"),
      runtimeLauncher: {
        prepareFull: async () => calls.push("activate"),
      },
    });

    const result = await adapter.requestUpdateAndRelaunch(
      "runtime update",
      "full",
      "request-1",
      requestedByThreadId,
    );

    assert.equal(result.ok, false);
    assert.equal(result.mode, "full");
    assert.equal(result.requestId, "request-1");
    assert.match(result.reason, /Invalid requestedByThreadId/);
    assert.deepEqual(calls, []);
  }
});

test("hot activation releases its producer-owned prepared root after launcher staging", async () => {
  const calls = [];
  const update = preparedUpdate();
  const adapter = createInstalledArtifactUpdateLifecycleAdapter({
    appServerRestart: {
      requestRestart: async () => {
        calls.push("restart-server");
        return { ok: true };
      },
    },
    cleanupPreparedArtifact: async (preparedRoot) => {
      calls.push(`cleanup:${preparedRoot}`);
    },
    reloadWindows: async () => {
      calls.push("reload-renderer");
      return { windowsReloaded: 1 };
    },
    runtimeLauncher: {
      supported: true,
      activateHot: async () => {
        calls.push("activate-hot");
        return { ok: true };
      },
      commitHot: async () => {
        calls.push("commit-hot");
        return { ok: true };
      },
    },
    resolvePlan: async () => ({
      appBundlePath: update.appBundlePath,
      requiresFullRelaunch: false,
    }),
    updateArtifacts: async () => update,
  });

  assert.equal(
    (await adapter.requestUpdateAndRelaunch(
      "runtime update",
      "hot",
      "request-1",
    )).ok,
    true,
  );
  assert.deepEqual(calls, [
    "activate-hot",
    `cleanup:${update.preparedRoot}`,
    "restart-server",
    "reload-renderer",
    "commit-hot",
  ]);
});

test("hot activation commits only after backend restart and renderer reload", async () => {
  const calls = [];
  const adapter = createInstalledArtifactUpdateLifecycleAdapter({
    appServerRestart: {
      requestRestart: async () => {
        calls.push("restart-server");
        return { ok: true };
      },
    },
    reloadWindows: async () => {
      calls.push("reload-renderer");
      return { windowsReloaded: 1 };
    },
    runtimeLauncher: {
      supported: true,
      activateHot: async () => {
        calls.push("activate-hot");
        return { ok: true };
      },
      commitHot: async () => {
        calls.push("commit-hot");
        return { ok: true };
      },
      rollbackHot: async () => {
        calls.push("rollback-hot");
        return { ok: true };
      },
    },
    resolvePlan: async () => ({
      appBundlePath: "/Applications/Morpheus.app",
      requiresFullRelaunch: false,
    }),
    updateArtifacts: async () => preparedUpdate(),
  });

  const result = await adapter.requestUpdateAndRelaunch(
    "runtime update",
    "hot",
    "request-1",
    "thread-1",
  );
  assert.equal(result.ok, true);
  assert.deepEqual(calls, [
    "activate-hot",
    "restart-server",
    "reload-renderer",
    "commit-hot",
  ]);
});

test("full activation aborts prepared transaction when backend stop fails", async () => {
  const calls = [];
  const adapter = createInstalledArtifactUpdateLifecycleAdapter({
    appServerStop: {
      requestStop: async () => ({ ok: false, reason: "stop failed" }),
    },
    runtimeLauncher: {
      supported: true,
      prepareFull: async () => ({ ok: true }),
      abortFull: async (transactionId) => {
        calls.push(["abort-full", transactionId]);
        return { ok: true };
      },
    },
    resolvePlan: async () => ({
      appBundlePath: "/Applications/Morpheus.app",
      requiresFullRelaunch: true,
    }),
    updateArtifacts: async () => preparedUpdate(),
  });

  const result = await adapter.requestUpdateAndRelaunch(
    "runtime update",
    "full",
    "request-1",
    "thread-1",
  );
  assert.equal(result.ok, false);
  assert.deepEqual(calls, [["abort-full", "transaction-1"]]);
});

test("hot activation rejects main or preload changes before calling launcher", async () => {
  let activated = false;
  const adapter = createInstalledArtifactUpdateLifecycleAdapter({
    runtimeLauncher: {
      supported: true,
      activateHot: async () => {
        activated = true;
        return { ok: true };
      },
    },
    resolvePlan: async () => ({
      appBundlePath: "/Applications/Morpheus.app",
      requiresFullRelaunch: false,
    }),
    updateArtifacts: async () => ({
      ...preparedUpdate(),
      manifest: { changes: { main: false, preload: true } },
    }),
  });

  const result = await adapter.requestUpdateAndRelaunch(
    "runtime update",
    "hot",
    "request-1",
    "thread-1",
  );
  assert.equal(result.ok, false);
  assert.match(result.reason, /main or preload changed/);
  assert.equal(activated, false);
});

test("hot failure invokes launcher rollback and reports partial update", async () => {
  const calls = [];
  const adapter = createInstalledArtifactUpdateLifecycleAdapter({
    appServerRestart: {
      requestRestart: async () => ({ ok: false, reason: "restart failed" }),
    },
    runtimeLauncher: {
      supported: true,
      activateHot: async () => ({ ok: true }),
      rollbackHot: async (transactionId) => {
        calls.push(["rollback", transactionId]);
        return { ok: true };
      },
    },
    resolvePlan: async () => ({
      appBundlePath: "/Applications/Morpheus.app",
      requiresFullRelaunch: false,
    }),
    updateArtifacts: async () => preparedUpdate(),
  });

  const result = await adapter.requestUpdateAndRelaunch(
    "runtime update",
    "hot",
    "request-1",
    "thread-1",
  );
  assert.equal(result.ok, false);
  assert.equal(result.partial, true);
  assert.deepEqual(calls, [["rollback", "transaction-1"]]);
});

test("notification handler preserves only missing, null, and valid thread provenance", async () => {
  for (const { params, expected } of [
    {
      params: { mode: "hot", requestId: "request-1", reason: "update" },
      expected: null,
    },
    {
      params: {
        mode: "hot",
        requestId: "request-1",
        requestedByThreadId: null,
        reason: "update",
      },
      expected: null,
    },
    {
      params: {
        mode: "hot",
        requestId: "request-1",
        requestedByThreadId: "  thread-1  ",
        reason: "update",
      },
      expected: "thread-1",
    },
  ]) {
    let received;
    const handler = createClientRelaunchNotificationHandler({
      installedArtifactUpdate: {
        requestUpdateAndRelaunch: async (...args) => {
          received = args;
          return { ok: true, mode: "hot" };
        },
      },
    });

    const result = await handler({
      method: "client/relaunch/requested",
      params,
    });

    assert.equal(result.ok, true);
    assert.deepEqual(received, ["update", "hot", "request-1", expected]);
  }
});

test("notification handler rejects present-invalid thread provenance without side effects", async () => {
  for (const requestedByThreadId of [
    undefined,
    "",
    "   ",
    42,
    false,
    {},
  ]) {
    const calls = [];
    const handler = createClientRelaunchNotificationHandler({
      installedArtifactUpdate: {
        requestUpdateAndRelaunch: async () => calls.push("update"),
      },
      rendererReload: {
        requestHotReload: async () => calls.push("reload"),
      },
      fullRelaunch: {
        requestRelaunch: async () => calls.push("relaunch"),
      },
    });

    const result = await handler({
      method: "client/relaunch/requested",
      params: {
        mode: "hot",
        requestId: "request-1",
        requestedByThreadId,
      },
    });

    assert.equal(result.ok, false);
    assert.equal(result.mode, "hot");
    assert.equal(result.requestId, "request-1");
    assert.match(result.reason, /Invalid requestedByThreadId/);
    assert.deepEqual(calls, []);
  }
});

test("installed activation coalesces concurrent requests with the same mode", async () => {
  let releaseUpdate;
  const blockedUpdate = new Promise((resolve) => {
    releaseUpdate = () => resolve(preparedUpdate());
  });
  let updates = 0;
  const adapter = createInstalledArtifactUpdateLifecycleAdapter({
    appServerRestart: {
      requestRestart: async () => ({ ok: true }),
    },
    reloadWindows: async () => ({ windowsReloaded: 1 }),
    runtimeLauncher: {
      supported: true,
      activateHot: async () => ({ ok: true }),
      commitHot: async () => ({ ok: true }),
    },
    resolvePlan: async () => ({
      appBundlePath: "/Applications/Morpheus.app",
      requiresFullRelaunch: false,
    }),
    updateArtifacts: async () => {
      updates += 1;
      return blockedUpdate;
    },
  });

  const first = adapter.requestUpdateAndRelaunch("one", "hot", "request-1");
  const second = adapter.requestUpdateAndRelaunch("two", "hot", "request-2");
  releaseUpdate();

  assert.equal((await first).ok, true);
  const coalesced = await second;
  assert.equal(coalesced.ok, true);
  assert.equal(coalesced.alreadyRequested, true);
  assert.equal(updates, 1);
});

test("installed activation rejects a conflicting mode while work is in flight", async () => {
  let releaseUpdate;
  const blockedUpdate = new Promise((resolve) => {
    releaseUpdate = () => resolve(preparedUpdate());
  });
  const adapter = createInstalledArtifactUpdateLifecycleAdapter({
    appServerRestart: {
      requestRestart: async () => ({ ok: true }),
    },
    reloadWindows: async () => ({ windowsReloaded: 1 }),
    runtimeLauncher: {
      supported: true,
      activateHot: async () => ({ ok: true }),
      commitHot: async () => ({ ok: true }),
    },
    resolvePlan: async () => ({
      appBundlePath: "/Applications/Morpheus.app",
      requiresFullRelaunch: false,
    }),
    updateArtifacts: async () => blockedUpdate,
  });

  const hot = adapter.requestUpdateAndRelaunch("one", "hot", "request-1");
  const conflict = await adapter.requestUpdateAndRelaunch(
    "two",
    "full",
    "request-2",
  );
  assert.equal(conflict.conflict, true);
  assert.equal(conflict.executingMode, "hot");
  releaseUpdate();
  assert.equal((await hot).ok, true);
});

test("hot rollback records typed failure evidence in the running host", async () => {
  const recovered = [];
  let restarts = 0;
  const adapter = createInstalledArtifactUpdateLifecycleAdapter({
    appServerRestart: {
      requestRestart: async () => {
        restarts += 1;
        return restarts === 1
          ? { ok: false, reason: "candidate restart failed" }
          : { ok: true };
      },
    },
    reloadWindows: async () => ({ windowsReloaded: 1 }),
    recoverLauncherFailure: async (failure) => {
      recovered.push(failure);
      return { ok: true };
    },
    runtimeLauncher: {
      supported: true,
      activateHot: async () => ({ ok: true }),
      rollbackHot: async () => ({
        ok: true,
        evidencePath: "/state/failure-evidence.json",
        failureEvidence: {
          transactionId: "transaction-1",
          requestId: "request-1",
        },
      }),
    },
    resolvePlan: async () => ({
      appBundlePath: "/Applications/Morpheus.app",
      requiresFullRelaunch: false,
    }),
    updateArtifacts: async () => preparedUpdate(),
  });

  const result = await adapter.requestUpdateAndRelaunch(
    "runtime update",
    "hot",
    "request-1",
    "thread-1",
  );
  assert.equal(result.ok, false);
  assert.deepEqual(recovered, [
    {
      evidencePath: "/state/failure-evidence.json",
      evidence: {
        transactionId: "transaction-1",
        requestId: "request-1",
      },
    },
  ]);
});

test("activate-hot internal rollback records typed evidence without rollback-hot", async () => {
  const calls = [];
  const activationError = new Error("signature verification failed");
  activationError.name = "RuntimeLauncherError";
  activationError.rolledBack = true;
  activationError.launcherResult = {
    ok: false,
    rolledBack: true,
    transactionId: "transaction-1",
    evidencePath: "/state/failure-evidence.json",
  };
  activationError.evidencePath = "/state/failure-evidence.json";
  const adapter = createInstalledArtifactUpdateLifecycleAdapter({
    appExit: (code) => calls.push(["exit", code]),
    recoverLauncherFailure: async (failure) => {
      calls.push(["recover", failure]);
      return { ok: true };
    },
    runtimeLauncher: {
      supported: true,
      activateHot: async () => {
        throw activationError;
      },
      rollbackHot: async () => {
        calls.push(["rollback-hot"]);
        return { ok: true };
      },
      status: async () => ({
        evidencePath: "/state/failure-evidence.json",
        result: {
          failureEvidence: {
            transactionId: "transaction-1",
            requestId: "request-1",
          },
        },
      }),
    },
    resolvePlan: async () => ({
      appBundlePath: "/Applications/Morpheus.app",
      requiresFullRelaunch: false,
    }),
    updateArtifacts: async () => preparedUpdate(),
    logger: { error: () => {} },
  });

  const result = await adapter.requestUpdateAndRelaunch(
    "runtime update",
    "hot",
    "request-1",
    "thread-1",
  );

  assert.equal(result.ok, false);
  assert.equal(result.relaunching, true);
  assert.deepEqual(calls, [
    [
      "recover",
      {
        evidencePath: "/state/failure-evidence.json",
        evidence: {
          transactionId: "transaction-1",
          requestId: "request-1",
        },
      },
    ],
    ["exit", 76],
  ]);
});

test("backend recovery failure keeps evidence for coordinated supervisor restart", async () => {
  const calls = [];
  const adapter = createInstalledArtifactUpdateLifecycleAdapter({
    appExit: (code) => calls.push(["exit", code]),
    appServerRestart: {
      requestRestart: async () => {
        calls.push(["restart"]);
        return { ok: false, reason: "app-server unavailable" };
      },
    },
    recoverLauncherFailure: async () => {
      calls.push(["recover"]);
      return { ok: true };
    },
    runtimeLauncher: {
      supported: true,
      activateHot: async () => ({ ok: true }),
      rollbackHot: async () => {
        calls.push(["rollback"]);
        return {
          ok: true,
          evidencePath: "/state/failure-evidence.json",
          failureEvidence: {
            transactionId: "transaction-1",
            requestId: "request-1",
          },
        };
      },
    },
    resolvePlan: async () => ({
      appBundlePath: "/Applications/Morpheus.app",
      requiresFullRelaunch: false,
    }),
    updateArtifacts: async () => preparedUpdate(),
    logger: { error: () => {} },
  });

  const result = await adapter.requestUpdateAndRelaunch(
    "runtime update",
    "hot",
    "request-1",
    "thread-1",
  );

  assert.equal(result.ok, false);
  assert.equal(result.relaunching, true);
  assert.deepEqual(calls, [
    ["restart"],
    ["rollback"],
    ["restart"],
    ["exit", 76],
  ]);
});

test("rollback command failure preserves evidence for supervised restart", async () => {
  const calls = [];
  const rollbackError = new Error("rollback verification failed");
  rollbackError.name = "RuntimeLauncherError";
  rollbackError.rolledBack = false;
  rollbackError.evidencePath = "/state/failure-evidence.json";
  const adapter = createInstalledArtifactUpdateLifecycleAdapter({
    appExit: (code) => calls.push(["exit", code]),
    appServerRestart: {
      requestRestart: async () => {
        calls.push(["restart"]);
        return { ok: false, reason: "candidate restart failed" };
      },
    },
    recoverLauncherFailure: async () => {
      calls.push(["recover"]);
      return { ok: true };
    },
    runtimeLauncher: {
      supported: true,
      activateHot: async () => ({ ok: true }),
      rollbackHot: async () => {
        calls.push(["rollback"]);
        throw rollbackError;
      },
    },
    resolvePlan: async () => ({
      appBundlePath: "/Applications/Morpheus.app",
      requiresFullRelaunch: false,
    }),
    updateArtifacts: async () => preparedUpdate(),
    logger: { error: () => {} },
  });

  const result = await adapter.requestUpdateAndRelaunch(
    "runtime update",
    "hot",
    "request-1",
    "thread-1",
  );

  assert.equal(result.ok, false);
  assert.equal(result.relaunching, true);
  assert.deepEqual(calls, [["restart"], ["rollback"], ["exit", 76]]);
});

test("restored renderer reload failure preserves evidence for supervised restart", async () => {
  const calls = [];
  const adapter = createInstalledArtifactUpdateLifecycleAdapter({
    appExit: (code) => calls.push(["exit", code]),
    appServerRestart: {
      requestRestart: async () => {
        calls.push(["restart"]);
        return { ok: true };
      },
    },
    reloadWindows: async () => {
      calls.push(["reload"]);
      throw new Error("renderer unavailable");
    },
    recoverLauncherFailure: async () => {
      calls.push(["recover"]);
      return { ok: true };
    },
    runtimeLauncher: {
      supported: true,
      activateHot: async () => ({ ok: true }),
      rollbackHot: async () => {
        calls.push(["rollback"]);
        return {
          ok: true,
          evidencePath: "/state/failure-evidence.json",
          failureEvidence: {
            transactionId: "transaction-1",
            requestId: "request-1",
          },
        };
      },
    },
    resolvePlan: async () => ({
      appBundlePath: "/Applications/Morpheus.app",
      requiresFullRelaunch: false,
    }),
    updateArtifacts: async () => preparedUpdate(),
    logger: { error: () => {} },
  });

  const result = await adapter.requestUpdateAndRelaunch(
    "runtime update",
    "hot",
    "request-1",
    "thread-1",
  );

  assert.equal(result.ok, false);
  assert.equal(result.relaunching, true);
  assert.deepEqual(calls, [
    ["restart"],
    ["reload"],
    ["rollback"],
    ["restart"],
    ["reload"],
    ["exit", 76],
  ]);
});

test("generic renderer reload remains available outside installed packaging", async () => {
  const adapter = createRendererReloadLifecycleAdapter({
    reloadWindows: async () => ({ windowsReloaded: 2 }),
  });
  assert.deepEqual(await adapter.requestHotReload("manual"), {
    ok: true,
    inPlace: true,
    relaunching: false,
    reloaded: true,
    alreadyRequested: false,
    windowsReloaded: 2,
    mode: "hot",
    reason: "manual",
  });
});

test("missing and invalid modes are rejected without side effects", async () => {
  for (const mode of [undefined, "auto"]) {
    const calls = [];
    const handler = createClientRelaunchNotificationHandler({
      installedArtifactUpdate: {
        requestUpdateAndRelaunch: async () => calls.push("update"),
      },
      rendererReload: {
        requestReload: async () => calls.push("reload"),
      },
    });

    const result = await handler({
      method: "client/relaunch/requested",
      params: { mode, requestId: "request-1" },
    });

    assert.equal(result.ok, false);
    assert.match(result.reason, /Invalid client relaunch mode/);
    assert.deepEqual(calls, []);
  }
});

test("installed artifact planning failure is typed and does not activate", async () => {
  let activated = false;
  const adapter = createInstalledArtifactUpdateLifecycleAdapter({
    resolvePlan: async () => {
      throw new Error("plan failed");
    },
    runtimeLauncher: {
      activateHot: async () => {
        activated = true;
      },
    },
    logger: { error: () => {} },
  });

  const result = await adapter.requestUpdateAndRelaunch(
    "runtime update",
    "hot",
    "request-1",
    "thread-1",
  );

  assert.equal(result.ok, false);
  assert.equal(result.requestId, "request-1");
  assert.match(result.reason, /plan failed/);
  assert.equal(activated, false);
});

test("installed artifact update ok false does not call launcher", async () => {
  let activated = false;
  const adapter = createInstalledArtifactUpdateLifecycleAdapter({
    resolvePlan: async () => ({ requiresFullRelaunch: false }),
    updateArtifacts: async () => ({ ok: false, reason: "prepare failed" }),
    runtimeLauncher: {
      activateHot: async () => {
        activated = true;
      },
    },
    logger: { error: () => {} },
  });

  const result = await adapter.requestUpdateAndRelaunch(
    "runtime update",
    "hot",
    "request-1",
    "thread-1",
  );
  assert.equal(result.ok, false);
  assert.match(result.reason, /prepare failed/);
  assert.equal(activated, false);
});

test("client relaunch observer preserves failed mode and request id", async () => {
  const statuses = [];
  const result = await observeClientRelaunchResult(
    Promise.resolve({
      ok: false,
      mode: "hot",
      requestId: "request-1",
      reason: "activation failed",
    }),
    { broadcastStatus: (status) => statuses.push(status) },
  );

  assert.equal(result.ok, false);
  assert.equal(statuses[0].lifecycle.mode, "hot");
  assert.equal(statuses[0].lifecycle.requestId, "request-1");
  assert.equal(statuses[0].lifecycle.phase, "failed");
});

test("client relaunch observer converts rejected handlers into a failed result", async () => {
  const statuses = [];
  const logs = [];
  const result = await observeClientRelaunchResult(
    Promise.reject(new Error("notification handler failed")),
    {
      broadcastStatus: (status) => statuses.push(status),
      logger: { error: (...args) => logs.push(args) },
      requestId: "request-1",
    },
  );

  assert.equal(result.ok, false);
  assert.equal(result.requestId, "request-1");
  assert.match(result.reason, /notification handler failed/);
  assert.match(logs[0][0], /client relaunch notification failed/);
  assert.equal(statuses[0].lifecycle.phase, "failed");
});

test("renderer reload adapter coalesces duplicate requests", async () => {
  let releaseReload;
  let reloads = 0;
  const adapter = createRendererReloadLifecycleAdapter({
    reloadWindows: async () => {
      reloads += 1;
      return new Promise((resolve) => {
        releaseReload = () => resolve({ windowsReloaded: 1 });
      });
    },
  });

  const first = adapter.requestReload("one");
  const second = adapter.requestReload("two");
  releaseReload();

  assert.equal((await first).ok, true);
  assert.equal((await second).alreadyRequested, true);
  assert.equal(reloads, 1);
});

test("generic app relaunch remains single-shot and cleans up before exit", async () => {
  const calls = [];
  let scheduled;
  const adapter = createAppRelaunchAdapter({
    app: {
      relaunch: () => calls.push("relaunch"),
      exit: (code) => calls.push(["exit", code]),
    },
    beforeExit: async (reason) => calls.push(["cleanup", reason]),
    setTimeout: (callback) => {
      scheduled = callback;
    },
  });

  assert.equal(adapter.requestRelaunch("fallback").ok, true);
  assert.equal(adapter.requestRelaunch("duplicate").alreadyRequested, true);
  await scheduled();
  assert.deepEqual(calls, [
    "relaunch",
    ["cleanup", "fallback"],
    ["exit", 0],
  ]);
  assert.equal(createAppRelaunchAdapter().requestRelaunch().ok, false);
});

test("notification handler falls back when installed updates are unsupported", async () => {
  const calls = [];
  const handler = createClientRelaunchNotificationHandler({
    installedArtifactUpdate: {
      requestUpdateAndRelaunch: async (_reason, mode) => {
        calls.push(["update", mode]);
        return { ok: false, unsupported: true };
      },
    },
    rendererReload: {
      requestHotReload: async () => {
        calls.push(["hot-reload"]);
        return { ok: true, mode: "hot", reloaded: true };
      },
    },
    fullRelaunch: {
      requestRelaunch: async () => {
        calls.push(["full-relaunch"]);
        return { ok: true, relaunching: true };
      },
    },
  });

  assert.equal(
    (
      await handler({
        method: "client/relaunch/requested",
        params: { mode: "hot" },
      })
    ).reloaded,
    true,
  );
  assert.equal(
    (
      await handler({
        method: "client/relaunch/requested",
        params: { mode: "full" },
      })
    ).relaunching,
    true,
  );
  assert.deepEqual(calls, [
    ["update", "hot"],
    ["hot-reload"],
    ["update", "full"],
    ["full-relaunch"],
  ]);
});

test("notification handler coalesces same mode and rejects both cross-mode directions", async () => {
  for (const [executingMode, requestedMode] of [
    ["hot", "full"],
    ["full", "hot"],
  ]) {
    let release;
    const pending = new Promise((resolve) => {
      release = () => resolve({ ok: true, mode: executingMode });
    });
    const handler = createClientRelaunchNotificationHandler({
      installedArtifactUpdate: {
        requestUpdateAndRelaunch: async () => pending,
      },
    });
    const first = handler({
      method: "client/relaunch/requested",
      params: { mode: executingMode },
    });
    const conflict = await handler({
      method: "client/relaunch/requested",
      params: { mode: requestedMode },
    });
    const coalesced = handler({
      method: "client/relaunch/requested",
      params: { mode: executingMode },
    });
    assert.equal(conflict.conflict, true);
    assert.equal(conflict.executingMode, executingMode);
    release();
    assert.equal((await first).ok, true);
    assert.equal((await coalesced).alreadyRequested, true);
  }
});

test("renderer lifecycle broadcasts success and distinguishes generic fallback from hot failure", async () => {
  const statuses = [];
  const reasons = [];
  const successful = createRendererReloadLifecycleAdapter({
    broadcastStatus: (status) => statuses.push(status),
    fullRelaunch: {
      requestRelaunch: () => {
        throw new Error("full fallback should not run");
      },
    },
    reloadWindows: async ({ reason }) => {
      reasons.push(reason);
      return { windowsReloaded: 2 };
    },
  });
  assert.equal((await successful.requestReload("refresh")).ok, true);
  assert.deepEqual(
    statuses.map((status) => status.lifecycle.phase),
    ["reloading", "reloaded"],
  );
  assert.deepEqual(reasons, ["refresh"]);

  const fallback = createRendererReloadLifecycleAdapter({
    fullRelaunch: {
      requestRelaunch: () => ({ ok: true, relaunching: true }),
    },
    reloadWindows: async () => {
      throw new Error("reload failed");
    },
  });
  assert.equal((await fallback.requestReload("generic")).relaunching, true);
  const hotFailure = await fallback.requestHotReload("hot");
  assert.equal(hotFailure.ok, false);
  assert.equal(hotFailure.relaunching, false);
});

test("preparation exceptions preserve request identity and cause no activation side effects", async () => {
  for (const mode of ["hot", "full"]) {
    const calls = [];
    const statuses = [];
    const adapter = createInstalledArtifactUpdateLifecycleAdapter({
      appExit: () => calls.push("exit"),
      appServerRestart: { requestRestart: () => calls.push("restart") },
      appServerStop: { requestStop: () => calls.push("stop") },
      reloadWindows: () => calls.push("reload"),
      resolvePlan: async () => ({ requiresFullRelaunch: mode === "full" }),
      runtimeLauncher: {
        activateHot: () => calls.push("activate-hot"),
        prepareFull: () => calls.push("prepare-full"),
      },
      updateArtifacts: async () => {
        throw new Error("prepare exploded");
      },
      broadcastStatus: (status) => statuses.push(status),
      logger: { error: () => {} },
    });
    const result = await adapter.requestUpdateAndRelaunch(
      "update",
      mode,
      "request-prepare",
      "thread-1",
    );

    assert.equal(result.ok, false);
    assert.equal(result.requestId, "request-prepare");
    assert.deepEqual(calls, []);
    assert.equal(statuses.at(-1).lifecycle.phase, "failed");
    assert.equal(statuses.at(-1).lifecycle.requestId, "request-prepare");
  }
});

test("installed activation broadcasts typed hot and full lifecycle phases", async () => {
  const hotStatuses = [];
  const hot = createInstalledArtifactUpdateLifecycleAdapter({
    appServerRestart: { requestRestart: async () => ({ ok: true }) },
    reloadWindows: async () => ({ windowsReloaded: 1 }),
    resolvePlan: async () => ({ requiresFullRelaunch: false }),
    runtimeLauncher: {
      activateHot: async () => ({ ok: true }),
      commitHot: async () => ({ ok: true }),
    },
    updateArtifacts: async () => preparedUpdate(),
    broadcastStatus: (status) => hotStatuses.push(status),
  });
  assert.equal(
    (await hot.requestUpdateAndRelaunch("update", "hot", "request-hot")).ok,
    true,
  );
  assert.deepEqual(
    hotStatuses.map((status) => status.lifecycle.phase),
    ["preparing", "updated", "reloading", "reloaded"],
  );
  assert.ok(
    hotStatuses.every(
      (status) => status.lifecycle.requestId === "request-hot",
    ),
  );

  const fullStatuses = [];
  const full = createInstalledArtifactUpdateLifecycleAdapter({
    appExit: () => {},
    appServerStop: { requestStop: async () => ({ ok: true }) },
    resolvePlan: async () => ({ requiresFullRelaunch: true }),
    runtimeLauncher: {
      supported: true,
      prepareFull: async () => ({ ok: true }),
    },
    updateArtifacts: async () => preparedUpdate(),
    broadcastStatus: (status) => fullStatuses.push(status),
  });
  assert.equal(
    (await full.requestUpdateAndRelaunch("update", "full", "request-full")).ok,
    true,
  );
  assert.deepEqual(
    fullStatuses.map((status) => status.lifecycle.phase),
    ["preparing", "updated", "relaunching", "relaunching"],
  );
});

test("relaunch observer preserves completed full and mode-conflict display facts", async () => {
  const statuses = [];
  await observeClientRelaunchResult(
    Promise.resolve({
      ok: true,
      mode: "full",
      requestId: "request-full",
      relaunching: true,
    }),
    { broadcastStatus: (status) => statuses.push(status) },
  );
  await observeClientRelaunchResult(
    Promise.resolve({
      ok: false,
      conflict: true,
      mode: null,
      requestedMode: "hot",
      executingMode: "full",
    }),
    { broadcastStatus: (status) => statuses.push(status) },
  );

  assert.equal(statuses[0].lifecycle.phase, "completed");
  assert.equal(statuses[0].lifecycle.mode, "full");
  assert.equal(statuses[1].lifecycle.phase, "failed");
  assert.equal(statuses[1].lifecycle.mode, null);
  assert.equal(statuses[1].relaunch.conflict, true);
});

function preparedUpdate() {
  return {
    ok: true,
    updated: false,
    transactionId: "transaction-1",
    buildId: "build-1",
    sourceCommit: "commit-1",
    preparedRoot: "/tmp/prepared",
    appBundlePath: "/Applications/Morpheus.app",
    manifest: { changes: { main: false, preload: false } },
  };
}
