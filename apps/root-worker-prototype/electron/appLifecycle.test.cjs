const test = require("node:test");
const assert = require("node:assert/strict");

const {
  createAppRelaunchAdapter,
  createClientRelaunchNotificationHandler,
  createRendererReloadLifecycleAdapter,
  isClientRelaunchNotification,
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
