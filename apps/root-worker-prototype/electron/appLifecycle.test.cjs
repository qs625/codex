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

test("app relaunch adapter schedules one full app relaunch", () => {
  const calls = [];
  const timers = [];
  const app = {
    relaunch: () => calls.push("relaunch"),
    exit: (code) => calls.push(["exit", code]),
  };
  const adapter = createAppRelaunchAdapter({
    app,
    setTimeout: (callback, delay) => {
      timers.push({ callback, delay });
    },
    exitDelayMs: 25,
  });

  assert.deepEqual(adapter.requestRelaunch("restart tool"), {
    ok: true,
    relaunching: true,
    alreadyRequested: false,
    reason: "restart tool",
  });
  assert.deepEqual(calls, ["relaunch"]);
  assert.equal(timers.length, 1);
  assert.equal(timers[0].delay, 25);

  assert.deepEqual(adapter.requestRelaunch("duplicate"), {
    ok: true,
    relaunching: true,
    alreadyRequested: true,
  });
  assert.deepEqual(calls, ["relaunch"]);

  timers[0].callback();
  assert.deepEqual(calls, ["relaunch", ["exit", 0]]);
});

test("app relaunch adapter reports unsupported environments", () => {
  const adapter = createAppRelaunchAdapter({ app: {} });

  assert.deepEqual(adapter.requestRelaunch(), {
    ok: false,
    relaunching: false,
    reason: "Application relaunch is unavailable in this environment",
  });
});

test("client relaunch notification accepts narrow lifecycle methods", () => {
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

test("renderer reload lifecycle adapter reloads windows without full app relaunch", async () => {
  const statuses = [];
  const reloads = [];
  const fullRelaunch = {
    requestRelaunch: () => {
      throw new Error("full relaunch should not be used");
    },
  };
  const adapter = createRendererReloadLifecycleAdapter({
    fullRelaunch,
    broadcastStatus: (status) => statuses.push(status),
    reloadWindows: async (payload) => {
      reloads.push(payload);
      return { windowsReloaded: 1 };
    },
  });

  assert.deepEqual(await adapter.requestReload("restart tool"), {
    ok: true,
    inPlace: true,
    relaunching: false,
    reloaded: true,
    alreadyRequested: false,
    windowsReloaded: 1,
    reason: "restart tool",
  });
  assert.deepEqual(reloads, [{ reason: "restart tool" }]);
  assert.deepEqual(statuses, [
    {
      lifecycle: {
        type: "rendererReload",
        phase: "reloading",
        reason: "restart tool",
      },
    },
    {
      lifecycle: {
        type: "rendererReload",
        phase: "reloaded",
        reason: "restart tool",
      },
    },
  ]);
});

test("client relaunch notification handler defaults to renderer reload", async () => {
  const reloads = [];
  const handler = createClientRelaunchNotificationHandler({
    rendererReload: {
      requestReload: async (reason) => {
        reloads.push(reason);
        return {
          ok: true,
          inPlace: true,
          relaunching: false,
          reloaded: true,
          reason,
        };
      },
    },
  });

  assert.deepEqual(
    await handler({
      method: "client/relaunch/requested",
      params: { reason: "restart tool" },
    }),
    {
      ok: true,
      inPlace: true,
      relaunching: false,
      reloaded: true,
      reason: "restart tool",
    },
  );
  assert.deepEqual(reloads, ["restart tool"]);
});

test("client relaunch notification handler updates installed artifacts before backend restart and renderer reload", async () => {
  const statuses = [];
  const calls = [];
  const plan = { appBundlePath: "/Moved App.app" };
  const handler = createClientRelaunchNotificationHandler({
    rendererReload: {
      requestReload: () => {
        throw new Error("renderer reload should not run for installed updates");
      },
    },
    installedArtifactUpdate: createInstalledArtifactUpdateLifecycleAdapter({
      appServerRestart: {
        requestRestart: async (reason) => {
          calls.push(["backendRestart", reason]);
          return { ok: true, restarted: true, pid: 42, reason };
        },
      },
      reloadWindows: async (payload) => {
        calls.push(["reload", payload]);
        return { windowsReloaded: 1 };
      },
      resolvePlan: () => plan,
      updateArtifacts: async (receivedPlan) => {
        calls.push(["update", receivedPlan]);
        return { ok: true, updated: true };
      },
      broadcastStatus: (status) => statuses.push(status),
      fullRelaunch: {
        requestRelaunch: () => {
          throw new Error("full relaunch should not run for installed updates");
        },
      },
    }),
  });

  assert.deepEqual(
    await handler({
      method: "client/relaunch/requested",
      params: { reason: "restart tool" },
    }),
    {
      ok: true,
      inPlace: true,
      relaunching: false,
      reloaded: true,
      updated: true,
      backendRestart: {
        ok: true,
        pending: false,
        restarted: true,
        pid: 42,
        reason: "restart tool",
      },
      mainProcessUpdate: "pendingAppRelaunch",
      preloadUpdate: "pendingWindowRecreateOrAppRelaunch",
      reason: "restart tool",
      reload: {
        ok: true,
        inPlace: true,
        relaunching: false,
        reloaded: true,
        windowsReloaded: 1,
        reason: "restart tool",
      },
    },
  );
  assert.deepEqual(calls, [
    ["update", plan],
    ["backendRestart", "restart tool"],
    ["reload", { reason: "restart tool" }],
  ]);
  assert.deepEqual(statuses.map((status) => status.lifecycle.phase), [
    "building",
    "updated",
    "reloading",
    "reloaded",
  ]);
});

test("installed artifact update failure does not reload stale renderer", async () => {
  const reloads = [];
  const statuses = [];
  const handler = createClientRelaunchNotificationHandler({
    rendererReload: {
      requestReload: () => {
        reloads.push("reload");
      },
    },
    installedArtifactUpdate: createInstalledArtifactUpdateLifecycleAdapter({
      resolvePlan: () => ({ appBundlePath: "/Moved App.app" }),
      updateArtifacts: async () => {
        throw new Error("build failed");
      },
      appServerRestart: {
        requestRestart: () => {
          throw new Error("backend restart should not run after failed update");
        },
      },
      reloadWindows: () => {
        throw new Error("renderer reload should not run after failed update");
      },
      logger: { error: () => {} },
      broadcastStatus: (status) => statuses.push(status),
    }),
  });

  assert.deepEqual(await handler({ method: "client/relaunch/requested" }), {
    ok: false,
    inPlace: false,
    partial: false,
    relaunching: false,
    reloaded: false,
    updated: false,
    backendRestart: undefined,
    reload: undefined,
    reason: "build failed",
  });
  assert.deepEqual(reloads, []);
  assert.deepEqual(statuses.at(-1), {
    lifecycle: {
      type: "installedArtifactUpdate",
      phase: "failed",
      reason: "build failed",
    },
  });
});

test("client relaunch observer broadcasts failed result from handler", async () => {
  const statuses = [];

  const result = await observeClientRelaunchResult(
    Promise.resolve({
      ok: false,
      inPlace: false,
      relaunching: false,
      reloaded: false,
      updated: false,
      reason: "pack failed",
    }),
    {
      broadcastStatus: (status) => statuses.push(status),
      logger: { error: () => {} },
      reason: "restart tool",
    },
  );

  assert.equal(result.ok, false);
  assert.deepEqual(statuses, [
    {
      lifecycle: {
        type: "clientRelaunch",
        phase: "failed",
        reason: "pack failed",
      },
      relaunch: {
        ok: false,
        inPlace: false,
        relaunching: false,
        reloaded: false,
        updated: false,
        reason: "pack failed",
      },
    },
  ]);
});

test("client relaunch observer catches rejected notification handlers", async () => {
  const logs = [];
  const statuses = [];

  const result = await observeClientRelaunchResult(
    Promise.reject(new Error("notification handler failed")),
    {
      broadcastStatus: (status) => statuses.push(status),
      logger: { error: (...args) => logs.push(args) },
      reason: "restart tool",
    },
  );

  assert.deepEqual(result, {
    ok: false,
    inPlace: false,
    relaunching: false,
    reloaded: false,
    updated: false,
    reason: "notification handler failed",
  });
  assert.match(logs[0][0], /client relaunch notification failed/);
  assert.deepEqual(statuses.at(-1), {
    lifecycle: {
      type: "clientRelaunch",
      phase: "failed",
      reason: "notification handler failed",
    },
  });
});

test("installed artifact update ok false does not relaunch", async () => {
  const reloads = [];
  const restarts = [];
  const handler = createClientRelaunchNotificationHandler({
    rendererReload: {
      requestReload: () => {
        throw new Error("renderer reload should not run");
      },
    },
    installedArtifactUpdate: createInstalledArtifactUpdateLifecycleAdapter({
      resolvePlan: () => ({ appBundlePath: "/Moved App.app" }),
      updateArtifacts: async () => ({
        ok: false,
        updated: false,
        reason: "update declined",
      }),
      appServerRestart: {
        requestRestart: () => {
          restarts.push("restart");
        },
      },
      reloadWindows: () => {
        reloads.push("reload");
      },
      logger: { error: () => {} },
      broadcastStatus: () => {},
    }),
  });

  assert.deepEqual(await handler({ method: "client/relaunch/requested" }), {
    ok: false,
    inPlace: false,
    partial: false,
    relaunching: false,
    reloaded: false,
    updated: false,
    backendRestart: undefined,
    reload: undefined,
    reason: "update declined",
  });
  assert.deepEqual(restarts, []);
  assert.deepEqual(reloads, []);
});

test("installed artifact update fails partial when backend restart fails", async () => {
  const reloads = [];
  const relaunches = [];
  const handler = createClientRelaunchNotificationHandler({
    rendererReload: {
      requestReload: () => {
        throw new Error("ordinary renderer reload should not run");
      },
    },
    installedArtifactUpdate: createInstalledArtifactUpdateLifecycleAdapter({
      appServerRestart: {
        requestRestart: async () => ({
          ok: false,
          restarted: false,
          reason: "backend restart failed",
        }),
      },
      reloadWindows: () => {
        reloads.push("reload");
      },
      resolvePlan: () => ({ appBundlePath: "/Moved App.app" }),
      updateArtifacts: async () => ({ ok: true, updated: true }),
      logger: { error: () => {} },
      broadcastStatus: () => {},
      fullRelaunch: {
        requestRelaunch: () => {
          relaunches.push("relaunch");
        },
      },
    }),
  });

  assert.deepEqual(await handler({ method: "client/relaunch/requested" }), {
    ok: false,
    inPlace: false,
    partial: true,
    relaunching: false,
    reloaded: false,
    updated: true,
    backendRestart: {
      ok: false,
      pending: false,
      restarted: false,
      pid: null,
      reason: "backend restart failed",
    },
    reload: undefined,
    reason: "backend restart failed",
  });
  assert.deepEqual(reloads, []);
  assert.deepEqual(relaunches, []);
});

test("installed artifact update fails partial without backend restart adapter", async () => {
  const reloads = [];
  const handler = createClientRelaunchNotificationHandler({
    rendererReload: {
      requestReload: () => {
        throw new Error("ordinary renderer reload should not run");
      },
    },
    installedArtifactUpdate: createInstalledArtifactUpdateLifecycleAdapter({
      reloadWindows: () => {
        reloads.push("reload");
      },
      resolvePlan: () => ({ appBundlePath: "/Moved App.app" }),
      updateArtifacts: async () => ({ ok: true, updated: true }),
      logger: { error: () => {} },
      broadcastStatus: () => {},
    }),
  });

  assert.deepEqual(await handler({ method: "client/relaunch/requested" }), {
    ok: false,
    inPlace: false,
    partial: true,
    relaunching: false,
    reloaded: false,
    updated: true,
    backendRestart: {
      ok: false,
      restarted: false,
      reason:
        "App-server restart adapter is unavailable after installed artifact update.",
    },
    reload: undefined,
    reason:
      "App-server restart adapter is unavailable after installed artifact update.",
  });
  assert.deepEqual(reloads, []);
});

test("installed artifact update fails partial when renderer reload fails", async () => {
  const relaunches = [];
  const handler = createClientRelaunchNotificationHandler({
    rendererReload: {
      requestReload: () => {
        throw new Error("ordinary renderer reload should not run");
      },
    },
    installedArtifactUpdate: createInstalledArtifactUpdateLifecycleAdapter({
      appServerRestart: {
        requestRestart: async (reason) => ({
          ok: true,
          restarted: true,
          pid: 42,
          reason,
        }),
      },
      reloadWindows: async () => {
        throw new Error("renderer reload failed");
      },
      resolvePlan: () => ({ appBundlePath: "/Moved App.app" }),
      updateArtifacts: async () => ({ ok: true, updated: true }),
      logger: { error: () => {} },
      broadcastStatus: () => {},
      fullRelaunch: {
        requestRelaunch: () => {
          relaunches.push("relaunch");
        },
      },
    }),
  });

  assert.deepEqual(await handler({ method: "client/relaunch/requested" }), {
    ok: false,
    inPlace: false,
    partial: true,
    relaunching: false,
    reloaded: false,
    updated: true,
    backendRestart: {
      ok: true,
      pending: false,
      restarted: true,
      pid: 42,
      reason: "client/relaunch/requested",
    },
    reload: {
      ok: false,
      inPlace: true,
      relaunching: false,
      reloaded: false,
      reason: "renderer reload failed",
    },
    reason: "renderer reload failed",
  });
  assert.deepEqual(relaunches, []);
});

test("client relaunch notification falls back to renderer reload when update unsupported", async () => {
  const reloads = [];
  const handler = createClientRelaunchNotificationHandler({
    rendererReload: {
      requestReload: async (reason) => {
        reloads.push(reason);
        return { ok: true, inPlace: true, reloaded: true, reason };
      },
    },
    installedArtifactUpdate: createInstalledArtifactUpdateLifecycleAdapter({
      resolvePlan: () => null,
      updateArtifacts: () => {
        throw new Error("unsupported update should not run");
      },
    }),
  });

  assert.deepEqual(await handler({ method: "client/relaunch/requested" }), {
    ok: true,
    inPlace: true,
    reloaded: true,
    reason: "client/relaunch/requested",
  });
  assert.deepEqual(reloads, ["client/relaunch/requested"]);
});

test("installed artifact update coalesces concurrent restart requests", async () => {
  let resolveUpdate;
  let updateCount = 0;
  let resolveCount = 0;
  const adapter = createInstalledArtifactUpdateLifecycleAdapter({
    resolvePlan: () => {
      resolveCount += 1;
      if (resolveCount > 1) {
        throw new Error("duplicate request should not resolve a new plan");
      }
      return { appBundlePath: "/Moved App.app" };
    },
    updateArtifacts: () => {
      updateCount += 1;
      return new Promise((resolve) => {
        resolveUpdate = resolve;
      });
    },
    fullRelaunch: {
      requestRelaunch: (reason) => ({
        ok: true,
        relaunching: true,
        reason,
      }),
    },
    broadcastStatus: () => {},
  });

  const first = adapter.requestUpdateAndRelaunch("first");
  const second = adapter.requestUpdateAndRelaunch("second");
  resolveUpdate({ ok: true, updated: true });

  assert.equal((await first).alreadyRequested, undefined);
  assert.equal((await second).alreadyRequested, true);
  assert.equal(resolveCount, 1);
  assert.equal(updateCount, 1);
});

test("client relaunch notification handler does not use app relaunch directly", async () => {
  const handler = createClientRelaunchNotificationHandler({
    rendererReload: {
      requestReload: async () => ({
        ok: true,
        inPlace: true,
        relaunching: false,
        reloaded: true,
      }),
    },
  });

  await handler({ method: "client/relaunch/requested" });
});

test("renderer reload lifecycle adapter coalesces duplicate requests", async () => {
  let resolveReload;
  let reloadCount = 0;
  const adapter = createRendererReloadLifecycleAdapter({
    broadcastStatus: () => {},
    reloadWindows: () => {
      reloadCount += 1;
      return new Promise((resolve) => {
        resolveReload = resolve;
      });
    },
  });

  const first = adapter.requestReload("restart tool");
  const second = adapter.requestReload("duplicate");
  resolveReload({ windowsReloaded: 1 });

  assert.deepEqual(await first, {
    ok: true,
    inPlace: true,
    relaunching: false,
    reloaded: true,
    alreadyRequested: false,
    windowsReloaded: 1,
    reason: "restart tool",
  });
  assert.deepEqual(await second, {
    ok: true,
    inPlace: true,
    relaunching: false,
    reloaded: true,
    alreadyRequested: true,
    windowsReloaded: 1,
    reason: "restart tool",
  });
  assert.equal(reloadCount, 1);
});

test("renderer reload lifecycle adapter falls back to full app relaunch", async () => {
  const statuses = [];
  const calls = [];
  const adapter = createRendererReloadLifecycleAdapter({
    broadcastStatus: (status) => statuses.push(status),
    logger: { warn: () => {} },
    reloadWindows: async () => {
      throw new Error("reload failed");
    },
    fullRelaunch: {
      requestRelaunch: (reason) => {
        calls.push(reason);
        return {
          ok: true,
          relaunching: true,
          alreadyRequested: false,
          reason,
        };
      },
    },
  });

  const result = await adapter.requestReload("restart tool");

  assert.deepEqual(result, {
    ok: true,
    inPlace: false,
    relaunching: true,
    reloaded: false,
    fallback: {
      ok: true,
      relaunching: true,
      alreadyRequested: false,
      reason: "restart tool",
    },
    reason: "restart tool",
  });
  assert.deepEqual(calls, ["restart tool"]);
  assert.deepEqual(statuses.at(-1), {
    lifecycle: {
      type: "rendererReload",
      phase: "fullRelaunchFallback",
      reason: "restart tool",
    },
    relaunch: {
      ok: true,
      relaunching: true,
      alreadyRequested: false,
      reason: "restart tool",
    },
  });
});
