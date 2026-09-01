const test = require("node:test");
const assert = require("node:assert/strict");

const {
  createAppRelaunchAdapter,
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
