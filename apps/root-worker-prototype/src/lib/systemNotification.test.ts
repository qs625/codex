import test from "node:test";
import assert from "node:assert/strict";

import {
  buildProjectThreadCompletedNotificationPayload,
  maybeNotifyProjectThreadCompleted,
  notifyProjectThreadCompleted,
} from "./systemNotification";
import type { Thread } from "../types";

const originalWindowDescriptor = Object.getOwnPropertyDescriptor(
  globalThis,
  "window",
);

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
  if (originalWindowDescriptor) {
    Object.defineProperty(globalThis, "window", originalWindowDescriptor);
  } else {
    Reflect.deleteProperty(globalThis, "window");
  }
});

test("buildProjectThreadCompletedNotificationPayload uses concise thread labels", () => {
  assert.deepEqual(
    buildProjectThreadCompletedNotificationPayload(
      makeThread({ name: " Build backend " }),
    ),
    {
      title: "Project thread completed",
      body: "Build backend",
    },
  );
  assert.deepEqual(
    buildProjectThreadCompletedNotificationPayload(makeThread()),
    {
      title: "Project thread completed",
      body: "/work/project",
    },
  );
});

test("maybeNotifyProjectThreadCompleted sends project completion through desktop IPC", () => {
  const payloads: unknown[] = [];
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: {
      codexDesktop: {
        showSystemNotification: (payload: unknown) => {
          payloads.push(payload);
          return Promise.resolve({ ok: true });
        },
      },
    },
  });

  assert.equal(
    maybeNotifyProjectThreadCompleted(makeThread({ name: "Build backend" }), {
      type: "final",
      result: { type: "completed" },
    }),
    true,
  );
  assert.deepEqual(payloads, [
    { title: "Project thread completed", body: "Build backend" },
  ]);
});

test("notifyProjectThreadCompleted tolerates missing or rejecting desktop IPC", () => {
  Reflect.deleteProperty(globalThis, "window");
  assert.doesNotThrow(() => {
    notifyProjectThreadCompleted(makeThread());
  });

  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: {
      codexDesktop: {
        showSystemNotification: () => Promise.reject(new Error("blocked")),
      },
    },
  });
  assert.doesNotThrow(() => {
    notifyProjectThreadCompleted(makeThread());
  });
});
