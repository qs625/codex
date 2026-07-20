import test from "node:test";
import assert from "node:assert/strict";

import {
  maybeNotifyProjectThreadCompleted,
  notifyProjectThreadCompleted,
} from "./systemNotification";
import type { Thread } from "../types";

const originalNotification = globalThis.Notification;

function makeThread(overrides: Partial<Thread> = {}): Thread {
  return {
    id: "thread-1",
    sessionId: "session-1",
    forkedFromId: null,
    preview: "",
    ephemeral: false,
    modelProvider: "openai",
    model: "gpt-5",
    reasoningEffort: null,
    createdAt: 1,
    updatedAt: 1,
    lifecycleStatus: { type: "active", activeFlags: [] },
    path: null,
    cwd: "/work/project",
    cliVersion: "test",
    source: "cli",
    threadSource: null,
    agentNickname: null,
    agentRole: null,
    gitInfo: null,
    name: null,
    skills: [],
    turns: [],
    ...overrides,
  };
}

test.afterEach(() => {
  Object.defineProperty(globalThis, "Notification", {
    configurable: true,
    value: originalNotification,
    writable: true,
  });
});

test("maybeNotifyProjectThreadCompleted sends granted project completion notification", () => {
  const notifications: Array<{ title: string; body?: string }> = [];
  class FakeNotification {
    static permission = "granted";
    static requestPermission = test.mock.fn();

    constructor(title: string, options?: NotificationOptions) {
      notifications.push({ title, body: options?.body });
    }
  }
  Object.defineProperty(globalThis, "Notification", {
    configurable: true,
    value: FakeNotification,
    writable: true,
  });

  assert.equal(
    maybeNotifyProjectThreadCompleted(makeThread({ name: "Build backend" }), {
      type: "final",
      result: { type: "completed" },
    }),
    true,
  );
  assert.deepEqual(notifications, [
    { title: "Project thread completed", body: "Build backend" },
  ]);
});

test("maybeNotifyProjectThreadCompleted ignores denied permissions and non-edges", () => {
  class FakeNotification {
    static permission = "denied";
    static requestPermission = test.mock.fn();

    constructor() {
      throw new Error("should not construct");
    }
  }
  Object.defineProperty(globalThis, "Notification", {
    configurable: true,
    value: FakeNotification,
    writable: true,
  });

  assert.equal(
    maybeNotifyProjectThreadCompleted(
      makeThread({
        lifecycleStatus: { type: "final", result: { type: "completed" } },
      }),
      { type: "final", result: { type: "completed" } },
    ),
    false,
  );
  assert.equal(FakeNotification.requestPermission.mock.callCount(), 0);
  assert.doesNotThrow(() => {
    notifyProjectThreadCompleted(makeThread());
  });
});

test("notifyProjectThreadCompleted tolerates missing or throwing Notification API", () => {
  Object.defineProperty(globalThis, "Notification", {
    configurable: true,
    value: undefined,
    writable: true,
  });
  assert.doesNotThrow(() => {
    notifyProjectThreadCompleted(makeThread());
  });

  class ThrowingNotification {
    static permission = "granted";

    constructor() {
      throw new Error("blocked");
    }
  }
  Object.defineProperty(globalThis, "Notification", {
    configurable: true,
    value: ThrowingNotification,
    writable: true,
  });
  assert.doesNotThrow(() => {
    notifyProjectThreadCompleted(makeThread());
  });
});
