const test = require("node:test");
const assert = require("node:assert/strict");

const {
  AUTO_RESUME_PROMPT,
  autoResumeFingerprint,
  createThreadAutoResumeCoordinator,
  isAutoResumeEligibleThread,
  pickAutoResumeCandidates,
  threadHasAutoResumePrompt,
} = require("./threadAutoResume.cjs");

function interruptedThread(overrides = {}) {
  return {
    id: "thread-1",
    updatedAt: 10,
    source: "appServer",
    threadSource: "user",
    lifecycleStatus: { type: "final", result: { type: "interrupted" } },
    model: "gpt",
    modelProvider: "openai",
    reasoningEffort: "medium",
    turns: [],
    ...overrides,
  };
}

test("auto-resume selects interrupted root app-server threads only", () => {
  assert.equal(isAutoResumeEligibleThread(interruptedThread()), true);
  assert.equal(
    isAutoResumeEligibleThread(
      interruptedThread({
        id: "running",
        lifecycleStatus: { type: "active", activeFlags: ["running"] },
      }),
    ),
    false,
  );
  assert.equal(
    isAutoResumeEligibleThread(
      interruptedThread({
        id: "completed",
        lifecycleStatus: { type: "final", result: { type: "completed" } },
      }),
    ),
    false,
  );
  assert.equal(
    isAutoResumeEligibleThread(
      interruptedThread({
        id: "subagent",
        threadSource: "subagent",
        agentPath: "/root/worker",
      }),
    ),
    false,
  );
  assert.equal(
    isAutoResumeEligibleThread(
      interruptedThread({
        id: "external-readonly",
        modelProvider: "opencode",
      }),
    ),
    false,
  );
  assert.equal(
    isAutoResumeEligibleThread(
      interruptedThread({
        id: "external-restorable",
        modelProvider: "opencode",
        restoreThread: true,
      }),
    ),
    true,
  );
});

test("auto-resume candidates are newest first", () => {
  assert.deepEqual(
    pickAutoResumeCandidates([
      interruptedThread({ id: "old", updatedAt: 1 }),
      interruptedThread({ id: "skip", lifecycleStatus: { type: "notLoaded" } }),
      interruptedThread({ id: "new", updatedAt: 3 }),
    ]).map((thread) => thread.id),
    ["new", "old"],
  );
});

test("auto-resume coordinator resumes once and submits recovery input", async () => {
  const calls = [];
  const marked = new Set();
  const coordinator = createThreadAutoResumeCoordinator({
    stateStore: {
      has: async (key) => marked.has(key),
      mark: async (key) => marked.add(key),
    },
    readThread: async (threadId) => {
      calls.push(["read", threadId]);
      return { thread: interruptedThread({ id: threadId }) };
    },
    subscribeThread: async (threadId) => {
      calls.push(["subscribe", threadId]);
      return { thread: interruptedThread({ id: threadId }) };
    },
    sendResumeInput: async (thread) => {
      calls.push([
        "send",
        thread.id,
        thread.model,
        thread.modelProvider,
        thread.reasoningEffort,
        AUTO_RESUME_PROMPT,
      ]);
      return { turn: { id: "turn-1" } };
    },
    logger: { warn: () => {} },
  });

  const first = await coordinator.run([interruptedThread({ id: "thread-a" })]);
  const second = await coordinator.run([interruptedThread({ id: "thread-a" })]);

  assert.deepEqual(first.resumedThreadIds, ["thread-a"]);
  assert.equal(first.focusThreadId, "thread-a");
  assert.deepEqual(second.resumedThreadIds, []);
  assert.deepEqual(second.skippedThreadIds, ["thread-a"]);
  assert.deepEqual(calls, [
    ["read", "thread-a"],
    ["subscribe", "thread-a"],
    ["send", "thread-a", "gpt", "openai", "medium", AUTO_RESUME_PROMPT],
  ]);
  assert.equal(
    marked.has(autoResumeFingerprint(interruptedThread({ id: "thread-a" }))),
    true,
  );
});

test("auto-resume skips interrupted threads that already contain recovery input", async () => {
  const restored = interruptedThread({
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "userMessage",
            content: [{ type: "text", text: AUTO_RESUME_PROMPT }],
          },
        ],
      },
    ],
  });
  const marked = new Set();
  const coordinator = createThreadAutoResumeCoordinator({
    stateStore: {
      has: async () => false,
      mark: async (key) => marked.add(key),
    },
    readThread: async () => ({ thread: restored }),
    subscribeThread: async () => {
      throw new Error("must not subscribe");
    },
    sendResumeInput: async () => {
      throw new Error("must not send");
    },
    logger: { warn: () => {} },
  });

  assert.equal(threadHasAutoResumePrompt(restored), true);
  const result = await coordinator.run([interruptedThread()]);

  assert.deepEqual(result.resumedThreadIds, []);
  assert.deepEqual(result.skippedThreadIds, ["thread-1"]);
  assert.equal(marked.has(autoResumeFingerprint(interruptedThread())), true);
});

test("auto-resume coordinator handles failures without throwing", async () => {
  const warnings = [];
  const coordinator = createThreadAutoResumeCoordinator({
    stateStore: {
      has: async () => false,
      mark: async () => {
        throw new Error("must not mark failed attempts");
      },
    },
    readThread: async () => ({ thread: interruptedThread() }),
    subscribeThread: async () => {
      throw new Error("resume failed");
    },
    sendResumeInput: async () => {
      throw new Error("must not send after subscribe failure");
    },
    logger: { warn: (...args) => warnings.push(args) },
  });

  const result = await coordinator.run([interruptedThread()]);

  assert.deepEqual(result.resumedThreadIds, []);
  assert.deepEqual(result.failedThreadIds, ["thread-1"]);
  assert.equal(result.errors[0].message, "resume failed");
  assert.equal(result.focusThreadId, null);
  assert.equal(warnings.length, 1);
});
