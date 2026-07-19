const test = require("node:test");
const assert = require("node:assert/strict");

const {
  normalizeThreadLifecycleStatus,
} = require("./threadLifecycleStatus.cjs");

test("normalizeThreadLifecycleStatus preserves new waiting and final payloads", () => {
  assert.deepEqual(
    normalizeThreadLifecycleStatus({ type: "waiting", reason: "child" }),
    { type: "waiting", reason: "child" },
  );
  assert.deepEqual(
    normalizeThreadLifecycleStatus({
      type: "final",
      result: { type: "completed", lastAgentMessage: "done" },
    }),
    { type: "final", result: { type: "completed", lastAgentMessage: "done" } },
  );
  assert.deepEqual(
    normalizeThreadLifecycleStatus({
      type: "systemError",
      message: "manager unavailable",
    }),
    { type: "systemError", message: "manager unavailable" },
  );
});

test("normalizeThreadLifecycleStatus maps legacy thread status shapes", () => {
  assert.deepEqual(
    normalizeThreadLifecycleStatus({ type: "idle", reason: "waitChild" }),
    { type: "waiting", reason: "child" },
  );
  assert.deepEqual(normalizeThreadLifecycleStatus({ type: "complete" }), {
    type: "final",
    result: { type: "completed" },
  });
});
