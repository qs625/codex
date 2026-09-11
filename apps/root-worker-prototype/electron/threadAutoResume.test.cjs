const test = require("node:test");
const assert = require("node:assert/strict");

const {
  autoResumeFingerprint,
  createThreadAutoResumeCoordinator,
  isAutoResumeEligibleThread,
  pickAutoResumeCandidates,
  threadHasAutoResumePrompt,
} = require("./threadAutoResume.cjs");
const {
  RESTART_RECOVERY_PROMPTS,
} = require("./restartRecoveryPrompts.cjs");

function projectRootThread(overrides = {}) {
  return {
    id: "thread-1",
    cwd: "/workspace/project",
    updatedAt: 10,
    source: "appServer",
    threadSource: "user",
    lifecycleStatus: { type: "active", activeFlags: ["running"] },
    model: "gpt",
    modelProvider: "openai",
    reasoningEffort: "medium",
    turns: [],
    ...overrides,
  };
}

function runtimeRestartRecovery(expectedThreadIds = ["system-self"]) {
  return { expectedThreadIds };
}

test("auto-resume selects active project roots and excludes terminal/waiting/children", () => {
  assert.equal(isAutoResumeEligibleThread(projectRootThread()), true);
  assert.equal(
    isAutoResumeEligibleThread(
      projectRootThread({
        id: "running",
        lifecycleStatus: { type: "active", activeFlags: ["running"] },
      }),
    ),
    true,
  );
  assert.equal(
    isAutoResumeEligibleThread(
      projectRootThread({
        id: "interrupted",
        lifecycleStatus: { type: "final", result: { type: "interrupted" } },
      }),
    ),
    false,
  );
  assert.equal(
    isAutoResumeEligibleThread(
      projectRootThread({
        id: "waiting",
        lifecycleStatus: { type: "waiting", reason: "command" },
      }),
    ),
    false,
  );
  assert.equal(
    isAutoResumeEligibleThread(
      projectRootThread({
        id: "completed",
        lifecycleStatus: { type: "final", result: { type: "completed" } },
      }),
    ),
    false,
  );
  assert.equal(
    isAutoResumeEligibleThread(
      projectRootThread({
        id: "subagent",
        threadSource: "subagent",
        agentPath: "/root/worker",
      }),
    ),
    false,
  );
  assert.equal(
    isAutoResumeEligibleThread(
      projectRootThread({
        id: "external-readonly",
        modelProvider: "opencode",
      }),
    ),
    true,
  );
  assert.equal(
    isAutoResumeEligibleThread(
      projectRootThread({
        id: "chat",
        cwd: "/workspace/.my-codex-root-worker-chat-cwd",
      }),
    ),
    false,
  );
  assert.equal(
    isAutoResumeEligibleThread(
      projectRootThread({
        id: "child-with-parent",
        parentThreadId: "parent-1",
      }),
    ),
    false,
  );
  assert.equal(
    isAutoResumeEligibleThread(projectRootThread({ id: "ephemeral", ephemeral: true })),
    false,
  );
  assert.equal(
    isAutoResumeEligibleThread(
      projectRootThread({ id: "source-subagent", source: { subAgent: true } }),
    ),
    false,
  );
  assert.equal(
    isAutoResumeEligibleThread(
      projectRootThread({ id: "snake-case-child", parent_thread_id: "parent-2" }),
    ),
    false,
  );
});

test("auto-resume candidates are newest first", () => {
  assert.deepEqual(
    pickAutoResumeCandidates([
      projectRootThread({ id: "old", updatedAt: 1 }),
      projectRootThread({
        id: "skip",
        lifecycleStatus: { type: "final", result: { type: "completed" } },
      }),
      projectRootThread({ id: "new", updatedAt: 3 }),
    ]).map((thread) => thread.id),
    ["new", "old"],
  );
});

test("restart recovery fanout resumes once and submits recovery input", async () => {
  const calls = [];
  const marked = new Set();
  const coordinator = createThreadAutoResumeCoordinator({
    stateStore: {
      has: async (key) => marked.has(key),
      mark: async (key) => marked.add(key),
    },
    readThread: async (threadId) => {
      calls.push(["read", threadId]);
      return { thread: projectRootThread({ id: threadId }) };
    },
    subscribeThread: async (threadId) => {
      calls.push(["subscribe", threadId]);
      return { thread: projectRootThread({ id: threadId }) };
    },
    sendResumeInput: async (thread, text) => {
      calls.push([
        "send",
        thread.id,
        thread.model,
        thread.modelProvider,
        thread.reasoningEffort,
        text,
      ]);
      return { turn: { id: "turn-1" } };
    },
    logger: { warn: () => {} },
  });

  const first = await coordinator.runAfterRuntimeRestartRecovery({
    threads: [projectRootThread({ id: "thread-a" })],
    expectedRestart: runtimeRestartRecovery(),
  });
  const second = await coordinator.runAfterRuntimeRestartRecovery({
    threads: [projectRootThread({ id: "thread-a" })],
    expectedRestart: runtimeRestartRecovery(),
  });

  assert.deepEqual(first.resumedThreadIds, ["thread-a"]);
  assert.equal(first.focusThreadId, "thread-a");
  assert.deepEqual(second.resumedThreadIds, []);
  assert.deepEqual(second.skippedThreadIds, ["thread-a"]);
  assert.deepEqual(calls, [
    ["read", "thread-a"],
    ["subscribe", "thread-a"],
    [
      "send",
      "thread-a",
      "gpt",
      "openai",
      "medium",
      RESTART_RECOVERY_PROMPTS.projectRootFanout,
    ],
  ]);
  assert.equal(
    marked.has(autoResumeFingerprint(projectRootThread({ id: "thread-a" }))),
    true,
  );
});

test("cold startup without a durable restart fact does not fan out", async () => {
  const calls = [];
  const coordinator = createThreadAutoResumeCoordinator({
    readThread: async () => calls.push("read"),
    subscribeThread: async () => calls.push("subscribe"),
    sendResumeInput: async () => calls.push("send"),
    stateStore: { has: async () => false, mark: async () => {} },
    logger: { warn: () => {} },
  });

  const result = await coordinator.runAfterRuntimeRestartRecovery({
    threads: [projectRootThread()],
    expectedRestart: { expectedThreadIds: [] },
  });

  assert.deepEqual(result.resumedThreadIds, []);
  assert.deepEqual(calls, []);
});

test("failed or interrupted restart facts still fan out generic recovery", async () => {
  const calls = [];
  const coordinator = createThreadAutoResumeCoordinator({
    readThread: async () => calls.push("read"),
    subscribeThread: async () => calls.push("subscribe"),
    sendResumeInput: async () => calls.push("send"),
    stateStore: { has: async () => false, mark: async () => {} },
    logger: { warn: () => {} },
  });

  const result = await coordinator.runAfterRuntimeRestartRecovery({
    threads: [projectRootThread()],
    expectedRestart: {
      expectedThreadIds: ["system-self"],
    },
  });

  assert.deepEqual(result.resumedThreadIds, ["thread-1"]);
  assert.deepEqual(calls, ["read", "subscribe", "send"]);
});

test("payload fallback restart fact fans out without expected intent ids", async () => {
  const calls = [];
  const coordinator = createThreadAutoResumeCoordinator({
    readThread: async () => calls.push("read"),
    subscribeThread: async () => calls.push("subscribe"),
    sendResumeInput: async () => calls.push("send"),
    stateStore: { has: async () => false, mark: async () => {} },
    logger: { warn: () => {} },
  });

  const result = await coordinator.runAfterRuntimeRestartRecovery({
    hasDurableRestartRecovery: true,
    threads: [projectRootThread()],
    expectedRestart: { expectedThreadIds: [] },
  });

  assert.deepEqual(result.resumedThreadIds, ["thread-1"]);
  assert.deepEqual(calls, ["read", "subscribe", "send"]);
});

test("durable restart fans out exactly once to every eligible project root including /self", async () => {
  const sent = [];
  const marked = new Set();
  const coordinator = createThreadAutoResumeCoordinator({
    stateStore: {
      has: async (key) => marked.has(key),
      mark: async (key) => marked.add(key),
    },
    readThread: async (threadId) => ({
      thread: projectRootThread({
        id: threadId,
        lifecycleStatus:
          threadId === "waiting"
            ? { type: "waiting", reason: "command" }
            : { type: "active", activeFlags: ["running"] },
      }),
    }),
    subscribeThread: async () => {},
    sendResumeInput: async (thread) => sent.push(thread.id),
    logger: { warn: () => {} },
  });
  const threads = [
    projectRootThread({ id: "running", updatedAt: 3 }),
    projectRootThread({
      id: "waiting",
      updatedAt: 2,
      lifecycleStatus: { type: "waiting", reason: "command" },
    }),
    projectRootThread({
      id: "completed",
      lifecycleStatus: { type: "final", result: { type: "completed" } },
    }),
    projectRootThread({
      id: "child",
      parentThreadId: "running",
    }),
    projectRootThread({
      id: "system-self",
      updatedAt: 5,
    }),
  ];

  assert.deepEqual(
    (
      await coordinator.runAfterRuntimeRestartRecovery({
        threads,
        expectedRestart: runtimeRestartRecovery(),
      })
    ).resumedThreadIds,
    ["system-self", "running"],
  );
  assert.deepEqual(sent, ["system-self", "running"]);
  assert.deepEqual(
    (
      await coordinator.runAfterRuntimeRestartRecovery({
        threads,
        expectedRestart: runtimeRestartRecovery(),
      })
    ).resumedThreadIds,
    [],
  );
  assert.deepEqual(sent, ["system-self", "running"]);
});

test("auto-resume skips active threads that already contain recovery input", async () => {
  const restored = projectRootThread({
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "userMessage",
            content: [
              {
                type: "text",
                text: RESTART_RECOVERY_PROMPTS.projectRootFanout,
              },
            ],
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
  const result = await coordinator.runAfterRuntimeRestartRecovery({
    threads: [projectRootThread()],
    expectedRestart: runtimeRestartRecovery(),
  });

  assert.deepEqual(result.resumedThreadIds, []);
  assert.deepEqual(result.skippedThreadIds, ["thread-1"]);
  assert.equal(marked.has(autoResumeFingerprint(projectRootThread())), true);
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
    readThread: async () => ({ thread: projectRootThread() }),
    subscribeThread: async () => {
      throw new Error("resume failed");
    },
    sendResumeInput: async () => {
      throw new Error("must not send after subscribe failure");
    },
    logger: { warn: (...args) => warnings.push(args) },
  });

  const result = await coordinator.runAfterRuntimeRestartRecovery({
    threads: [projectRootThread()],
    expectedRestart: runtimeRestartRecovery(),
  });

  assert.deepEqual(result.resumedThreadIds, []);
  assert.deepEqual(result.failedThreadIds, ["thread-1"]);
  assert.equal(result.errors[0].message, "resume failed");
  assert.equal(result.focusThreadId, null);
  assert.equal(warnings.length, 1);
});
