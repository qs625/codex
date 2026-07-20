const test = require("node:test");
const assert = require("node:assert/strict");

const {
  normalizeSystemNotificationPayload,
  showSystemNotification,
} = require("./systemNotification.cjs");

test("normalizeSystemNotificationPayload trims and bounds text", () => {
  assert.deepEqual(
    normalizeSystemNotificationPayload({
      title: "  Project complete  ",
      body: "  /work/project  ",
    }),
    {
      title: "Project complete",
      body: "/work/project",
    },
  );
  assert.deepEqual(
    normalizeSystemNotificationPayload({
      title: "x".repeat(90),
      body: "y".repeat(170),
    }),
    {
      title: `${"x".repeat(77)}...`,
      body: `${"y".repeat(157)}...`,
    },
  );
});

test("showSystemNotification displays a valid notification", () => {
  const shown = [];
  class FakeNotification {
    constructor(payload) {
      this.payload = payload;
    }

    show() {
      shown.push(this.payload);
    }
  }

  assert.deepEqual(
    showSystemNotification(
      { title: "Project complete", body: "/work/project" },
      { Notification: FakeNotification },
    ),
    { ok: true },
  );
  assert.deepEqual(shown, [
    { title: "Project complete", body: "/work/project" },
  ]);
});

test("showSystemNotification rejects invalid payloads without throwing", () => {
  class ThrowingNotification {
    constructor() {
      throw new Error("should not construct");
    }
  }

  for (const payload of [
    null,
    undefined,
    "Project complete",
    [],
    {},
    { title: "" },
    { title: "   " },
    { title: 42 },
    { title: "Ready", body: "" },
    { title: "Ready", body: "   " },
    { title: "Ready", body: 42 },
    { title: "Ready", body: {} },
  ]) {
    assert.deepEqual(
      showSystemNotification(payload, { Notification: ThrowingNotification }),
      { ok: false, reason: "invalidPayload" },
    );
  }
});

test("showSystemNotification reports unavailable and show failures", () => {
  assert.deepEqual(showSystemNotification({ title: "Ready" }), {
    ok: false,
    reason: "unavailable",
  });

  class NoShowNotification {}
  assert.deepEqual(
    showSystemNotification(
      { title: "Ready" },
      { Notification: NoShowNotification },
    ),
    { ok: false, reason: "unavailable" },
  );

  class ThrowingNotification {
    constructor() {
      throw new Error("blocked");
    }
  }
  assert.deepEqual(
    showSystemNotification(
      { title: "Ready" },
      { Notification: ThrowingNotification },
    ),
    { ok: false, reason: "showFailed" },
  );
});
