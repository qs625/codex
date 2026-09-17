const test = require("node:test");
const assert = require("node:assert/strict");

const {
  autoResumePromptForOccurrence,
  autoResumeFingerprint,
  autoResumeOccurrenceId,
  collectRuntimeRecoveryThreads,
  createThreadAutoResumeCoordinator,
  isAutoResumeTargetLifecycleStatus,
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

function runtimeRestartRecovery(
  expectedThreadIds = ["system-self"],
  requestId = "restart-1",
) {
  return {
    expectedRequestIds: [requestId],
    expectedThreadIds,
    recoveryOccurrenceId: `runtime-restart:${requestId}`,
  };
}

test("auto-resume selects recoverable threads and excludes completed/self/chat", () => {
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
    true,
  );
  assert.equal(
    isAutoResumeEligibleThread(
      projectRootThread({
        id: "shutdown",
        lifecycleStatus: { type: "final", result: { type: "shutdown" } },
      }),
    ),
    false,
  );
  assert.equal(
    isAutoResumeEligibleThread(
      projectRootThread({
        id: "not-loaded",
        lifecycleStatus: { type: "notLoaded" },
      }),
    ),
    false,
  );
  assert.equal(
    isAutoResumeEligibleThread(
      projectRootThread({
        id: "initializing",
        lifecycleStatus: { type: "initializing" },
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
    true,
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
    true,
  );
  assert.equal(
    isAutoResumeEligibleThread(projectRootThread({ id: "ephemeral", ephemeral: true })),
    false,
  );
  assert.equal(
    isAutoResumeEligibleThread(
      projectRootThread({ id: "source-subagent", source: { subAgent: true } }),
    ),
    true,
  );
  assert.equal(
    isAutoResumeEligibleThread(
      projectRootThread({ id: "snake-case-child", parent_thread_id: "parent-2" }),
    ),
    true,
  );
  assert.equal(
    isAutoResumeEligibleThread(
      projectRootThread({ id: "system-self", name: "/self" }),
    ),
    false,
  );
  assert.equal(
    isAutoResumeEligibleThread(
      projectRootThread({ id: "agent-self", agentPath: "/self" }),
    ),
    false,
  );
  assert.equal(
    isAutoResumeEligibleThread(
      projectRootThread({
        id: "self-owner",
        agentPath: "/self/owner_dev_3",
      }),
    ),
    true,
  );
});

test("auto-resume target lifecycle accepts only active and waiting", () => {
  assert.equal(
    isAutoResumeTargetLifecycleStatus({ type: "active", activeFlags: [] }),
    true,
  );
  assert.equal(
    isAutoResumeTargetLifecycleStatus({ type: "waiting", reason: "command" }),
    true,
  );
  assert.equal(
    isAutoResumeTargetLifecycleStatus({
      type: "final",
      result: { type: "interrupted" },
    }),
    false,
  );
  assert.equal(
    isAutoResumeTargetLifecycleStatus({
      type: "final",
      result: { type: "completed" },
    }),
    false,
  );
  assert.equal(isAutoResumeTargetLifecycleStatus({ type: "notLoaded" }), false);
  assert.equal(isAutoResumeTargetLifecycleStatus({ type: "initializing" }), false);
});

test("auto-resume candidates are newest first and unique by thread id", () => {
  assert.deepEqual(
    pickAutoResumeCandidates([
      projectRootThread({ id: "old", updatedAt: 1 }),
      projectRootThread({
        id: "skip",
        lifecycleStatus: { type: "final", result: { type: "completed" } },
      }),
      projectRootThread({ id: "new", updatedAt: 3 }),
      projectRootThread({ id: "old", updatedAt: 5 }),
    ]).map((thread) => thread.id),
    ["old", "new"],
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
  assert.deepEqual(second.skippedThreadIds, []);
  assert.deepEqual(calls, [
    ["read", "thread-a"],
    ["subscribe", "thread-a"],
    [
      "send",
      "thread-a",
      "gpt",
      "openai",
      "medium",
      autoResumePromptForOccurrence("runtime-restart:restart-1"),
    ],
  ]);
  assert.equal(
    marked.has(
      autoResumeFingerprint(
        projectRootThread({ id: "thread-a" }),
        "runtime-restart:restart-1",
      ),
    ),
    true,
  );
});

test("restart-scoped auto-resume skips when history already has the occurrence prompt", async () => {
  const calls = [];
  const coordinator = createThreadAutoResumeCoordinator({
    stateStore: {
      has: async () => {
        throw new Error("state read unavailable");
      },
      mark: async () => {
        throw new Error("state write unavailable");
      },
    },
    readThread: async (threadId) => ({
      thread: projectRootThread({
        id: threadId,
        turns: [
          {
            items: [
              {
                type: "userMessage",
                content: [
                  {
                    type: "text",
                    text: autoResumePromptForOccurrence(
                      "runtime-restart:restart-1",
                    ),
                  },
                ],
              },
            ],
          },
        ],
      }),
    }),
    subscribeThread: async () => calls.push("subscribe"),
    sendResumeInput: async () => calls.push("send"),
    logger: { warn: () => {} },
  });

  const result = await coordinator.runAfterRuntimeRestartRecovery({
    threads: [projectRootThread({ id: "thread-a" })],
    expectedRestart: runtimeRestartRecovery(),
  });

  assert.deepEqual(result.resumedThreadIds, []);
  assert.deepEqual(result.skippedThreadIds, ["thread-a"]);
  assert.deepEqual(calls, []);
});

test("restart-scoped auto-resume markers allow a later restart with unchanged updatedAt", async () => {
  const calls = [];
  const marked = new Set();
  const coordinator = createThreadAutoResumeCoordinator({
    stateStore: {
      has: async (key) => marked.has(key),
      mark: async (key) => marked.add(key),
    },
    readThread: async (threadId) => ({
      thread: projectRootThread({
        id: threadId,
        updatedAt: 10,
        turns: [
          {
            id: "older-turn",
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
      }),
    }),
    subscribeThread: async (threadId) => calls.push(["subscribe", threadId]),
    sendResumeInput: async (thread) => calls.push(["send", thread.id]),
    logger: { warn: () => {} },
  });
  const thread = projectRootThread({ id: "thread-a", updatedAt: 10 });

  const first = await coordinator.runAfterRuntimeRestartRecovery({
    threads: [thread],
    expectedRestart: runtimeRestartRecovery(["system-self"], "restart-a"),
  });
  const repeatedFirst = await coordinator.runAfterRuntimeRestartRecovery({
    threads: [thread],
    expectedRestart: runtimeRestartRecovery(["system-self"], "restart-a"),
  });
  const second = await coordinator.runAfterRuntimeRestartRecovery({
    threads: [thread],
    expectedRestart: runtimeRestartRecovery(["system-self"], "restart-b"),
  });

  assert.deepEqual(first.resumedThreadIds, ["thread-a"]);
  assert.deepEqual(repeatedFirst.resumedThreadIds, []);
  assert.deepEqual(repeatedFirst.skippedThreadIds, []);
  assert.deepEqual(second.resumedThreadIds, ["thread-a"]);
  assert.deepEqual(calls, [
    ["subscribe", "thread-a"],
    ["send", "thread-a"],
    ["subscribe", "thread-a"],
    ["send", "thread-a"],
  ]);
  assert.equal(marked.has("thread-a:10"), false);
  assert.equal(
    marked.has("restart-v2:runtime-restart:restart-a:thread-a"),
    true,
  );
  assert.equal(
    marked.has("restart-v2:runtime-restart:restart-b:thread-a"),
    true,
  );
});

test("fallback recovery skips a thread that already has any restart fanout prompt", async () => {
  const calls = [];
  const coordinator = createThreadAutoResumeCoordinator({
    stateStore: {
      has: async () => false,
      mark: async () => {},
    },
    readThread: async (threadId) => ({
      thread: projectRootThread({
        id: threadId,
        turns: [
          {
            items: [
              {
                type: "userMessage",
                content: [
                  {
                    type: "text",
                    text: autoResumePromptForOccurrence(
                      "runtime-restart:restart-1",
                    ),
                  },
                ],
              },
            ],
          },
        ],
      }),
    }),
    subscribeThread: async () => calls.push("subscribe"),
    sendResumeInput: async () => calls.push("send"),
    logger: { warn: () => {} },
  });

  const result = await coordinator.runAfterRuntimeRestartRecovery({
    hasDurableRestartRecovery: true,
    recoveryOccurrenceId: "payload:restart-1",
    threads: [projectRootThread()],
    expectedRestart: { expectedThreadIds: [] },
  });

  assert.deepEqual(result.resumedThreadIds, []);
  assert.deepEqual(result.skippedThreadIds, ["thread-1"]);
  assert.deepEqual(calls, []);
});

test("expected restart with a different occurrence still fans out once", async () => {
  const calls = [];
  const coordinator = createThreadAutoResumeCoordinator({
    stateStore: {
      has: async () => false,
      mark: async () => {},
    },
    readThread: async (threadId) => ({
      thread: projectRootThread({
        id: threadId,
        turns: [
          {
            items: [
              {
                type: "userMessage",
                content: [
                  {
                    type: "text",
                    text: autoResumePromptForOccurrence(
                      "runtime-restart:restart-a",
                    ),
                  },
                ],
              },
            ],
          },
        ],
      }),
    }),
    subscribeThread: async (threadId) => calls.push(["subscribe", threadId]),
    sendResumeInput: async (thread, text) =>
      calls.push(["send", thread.id, text]),
    logger: { warn: () => {} },
  });

  const result = await coordinator.runAfterRuntimeRestartRecovery({
    threads: [projectRootThread()],
    expectedRestart: runtimeRestartRecovery(["system-self"], "restart-b"),
  });

  assert.deepEqual(result.resumedThreadIds, ["thread-1"]);
  assert.deepEqual(calls, [
    ["subscribe", "thread-1"],
    [
      "send",
      "thread-1",
      autoResumePromptForOccurrence("runtime-restart:restart-b"),
    ],
  ]);
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

test("same restart occurrence from expected and fallback sources runs one fanout pass", async () => {
  const calls = [];
  const coordinator = createThreadAutoResumeCoordinator({
    readThread: async (threadId) => {
      calls.push(["read", threadId]);
      return { thread: projectRootThread({ id: threadId }) };
    },
    subscribeThread: async (threadId) => calls.push(["subscribe", threadId]),
    sendResumeInput: async (thread) => calls.push(["send", thread.id]),
    stateStore: {
      has: async () => {
        throw new Error("state store unavailable");
      },
      mark: async () => {
        throw new Error("state store unavailable");
      },
    },
    logger: { warn: () => {} },
  });

  const expected = await coordinator.runAfterRuntimeRestartRecovery({
    threads: [projectRootThread({ id: "thread-a" })],
    expectedRestart: runtimeRestartRecovery(["system-self"], "restart-1"),
  });
  const fallback = await coordinator.runAfterRuntimeRestartRecovery({
    hasDurableRestartRecovery: true,
    recoveryOccurrenceId: "runtime-restart:restart-1",
    threads: [projectRootThread({ id: "thread-a" })],
    expectedRestart: { expectedThreadIds: [] },
  });

  assert.deepEqual(expected.resumedThreadIds, ["thread-a"]);
  assert.deepEqual(fallback.resumedThreadIds, []);
  assert.deepEqual(calls, [
    ["read", "thread-a"],
    ["subscribe", "thread-a"],
    ["send", "thread-a"],
  ]);
});

test("duplicate thread ids in the list are dispatched once in a pass", async () => {
  const calls = [];
  const coordinator = createThreadAutoResumeCoordinator({
    readThread: async (threadId) => {
      calls.push(["read", threadId]);
      return { thread: projectRootThread({ id: threadId }) };
    },
    subscribeThread: async (threadId) => calls.push(["subscribe", threadId]),
    sendResumeInput: async (thread) => calls.push(["send", thread.id]),
    stateStore: { has: async () => false, mark: async () => {} },
    logger: { warn: () => {} },
  });

  const result = await coordinator.runAfterRuntimeRestartRecovery({
    threads: [
      projectRootThread({ id: "thread-a", updatedAt: 1 }),
      projectRootThread({ id: "thread-a", updatedAt: 2 }),
    ],
    expectedRestart: runtimeRestartRecovery(),
  });

  assert.deepEqual(result.resumedThreadIds, ["thread-a"]);
  assert.deepEqual(calls, [
    ["read", "thread-a"],
    ["subscribe", "thread-a"],
    ["send", "thread-a"],
  ]);
});

test("duplicate thread ids use the newest snapshot before lifecycle filtering", async () => {
  const calls = [];
  const coordinator = createThreadAutoResumeCoordinator({
    readThread: async (threadId) => {
      calls.push(["read", threadId]);
      return { thread: projectRootThread({ id: threadId }) };
    },
    subscribeThread: async (threadId) => calls.push(["subscribe", threadId]),
    sendResumeInput: async (thread) => calls.push(["send", thread.id]),
    stateStore: { has: async () => false, mark: async () => {} },
    logger: { warn: () => {} },
  });

  const result = await coordinator.runAfterRuntimeRestartRecovery({
    threads: [
      projectRootThread({ id: "thread-a", updatedAt: 1 }),
      projectRootThread({
        id: "thread-a",
        updatedAt: 2,
        lifecycleStatus: { type: "final", result: { type: "completed" } },
      }),
    ],
    expectedRestart: runtimeRestartRecovery(),
  });

  assert.deepEqual(result.resumedThreadIds, []);
  assert.deepEqual(calls, []);
});

test("non-target list statuses are not read or dispatched", async () => {
  const calls = [];
  const coordinator = createThreadAutoResumeCoordinator({
    readThread: async (threadId) => {
      calls.push(["read", threadId]);
      return { thread: projectRootThread({ id: threadId }) };
    },
    subscribeThread: async (threadId) => calls.push(["subscribe", threadId]),
    sendResumeInput: async (thread) => calls.push(["send", thread.id]),
    stateStore: { has: async () => false, mark: async () => {} },
    logger: { warn: () => {} },
  });

  const result = await coordinator.runAfterRuntimeRestartRecovery({
    threads: [
      projectRootThread({
        id: "interrupted",
        lifecycleStatus: { type: "final", result: { type: "interrupted" } },
      }),
      projectRootThread({
        id: "completed",
        lifecycleStatus: { type: "final", result: { type: "completed" } },
      }),
      projectRootThread({ id: "not-loaded", lifecycleStatus: { type: "notLoaded" } }),
      projectRootThread({
        id: "initializing",
        lifecycleStatus: { type: "initializing" },
      }),
    ],
    expectedRestart: runtimeRestartRecovery(),
  });

  assert.deepEqual(result.resumedThreadIds, []);
  assert.deepEqual(calls, []);
});

test("read after a list target can only narrow the fanout target", async () => {
  const calls = [];
  const coordinator = createThreadAutoResumeCoordinator({
    readThread: async (threadId) => {
      calls.push(["read", threadId]);
      return {
        thread: projectRootThread({
          id: threadId,
          lifecycleStatus: { type: "final", result: { type: "completed" } },
        }),
      };
    },
    subscribeThread: async () => {
      throw new Error("must not subscribe");
    },
    sendResumeInput: async () => {
      throw new Error("must not send");
    },
    stateStore: { has: async () => false, mark: async () => {} },
    logger: { warn: () => {} },
  });

  const result = await coordinator.runAfterRuntimeRestartRecovery({
    threads: [projectRootThread({ id: "thread-a" })],
    expectedRestart: runtimeRestartRecovery(),
  });

  assert.deepEqual(result.resumedThreadIds, []);
  assert.deepEqual(result.skippedThreadIds, ["thread-a"]);
  assert.deepEqual(calls, [["read", "thread-a"]]);
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

test("durable restart fans out exactly once to eligible affected threads except /self", async () => {
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
      name: "/self",
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
    ["child", "running", "waiting"],
  );
  assert.deepEqual(sent, ["child", "running", "waiting"]);
  assert.deepEqual(
    (
      await coordinator.runAfterRuntimeRestartRecovery({
        threads,
        expectedRestart: runtimeRestartRecovery(),
      })
    ).resumedThreadIds,
    [],
  );
  assert.deepEqual(sent, ["child", "running", "waiting"]);
});

test("runtime recovery includes loaded active subagents missing from thread list", async () => {
  const listedThreads = [
    projectRootThread({ id: "root", updatedAt: 3 }),
    projectRootThread({
      id: "completed-root",
      lifecycleStatus: { type: "final", result: { type: "completed" } },
    }),
  ];
  const loadedSnapshots = {
    owner: projectRootThread({
      id: "owner",
      agentPath: "/project/owner",
      parentThreadId: "root",
      threadSource: "subagent",
      updatedAt: 4,
    }),
    "completed-owner": projectRootThread({
      id: "completed-owner",
      agentPath: "/project/completed_owner",
      parentThreadId: "root",
      threadSource: "subagent",
      lifecycleStatus: { type: "final", result: { type: "completed" } },
      updatedAt: 5,
    }),
  };
  const candidateReads = [];
  const fanoutReads = [];
  const sent = [];

  const recoveryThreads = await collectRuntimeRecoveryThreads({
    listedThreads,
    listLoadedThreadIds: async () => ["root", "owner", "completed-owner"],
    readThread: async (threadId) => {
      candidateReads.push(threadId);
      return { thread: loadedSnapshots[threadId] };
    },
    logger: { warn: () => {} },
  });
  const threadById = new Map(
    recoveryThreads.map((thread) => [thread.id, thread]),
  );
  const coordinator = createThreadAutoResumeCoordinator({
    stateStore: { has: async () => false, mark: async () => {} },
    readThread: async (threadId) => {
      fanoutReads.push(threadId);
      return { thread: threadById.get(threadId) };
    },
    subscribeThread: async () => {},
    sendResumeInput: async (thread, text) =>
      sent.push({ threadId: thread.id, text }),
    logger: { warn: () => {} },
  });

  const result = await coordinator.runAfterRuntimeRestartRecovery({
    threads: recoveryThreads,
    expectedRestart: runtimeRestartRecovery(["system-self"], "restart-1"),
  });

  assert.deepEqual(candidateReads, ["owner", "completed-owner"]);
  assert.deepEqual(result.resumedThreadIds, ["owner", "root"]);
  assert.deepEqual(result.skippedThreadIds, []);
  assert.deepEqual(fanoutReads, ["owner", "root"]);
  assert.deepEqual(sent, [
    {
      threadId: "owner",
      text: autoResumePromptForOccurrence("runtime-restart:restart-1"),
    },
    {
      threadId: "root",
      text: autoResumePromptForOccurrence("runtime-restart:restart-1"),
    },
  ]);
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
    expectedRestart: { expectedThreadIds: ["system-self"] },
  });

  assert.deepEqual(result.resumedThreadIds, []);
  assert.deepEqual(result.skippedThreadIds, ["thread-1"]);
  assert.equal(marked.has(autoResumeFingerprint(projectRootThread())), true);
});

test("autoResumeOccurrenceId prefers explicit and restart recovery identities", () => {
  assert.equal(
    autoResumeOccurrenceId({
      recoveryOccurrenceId: " explicit ",
      expectedRestart: runtimeRestartRecovery(),
    }),
    "explicit",
  );
  assert.equal(
    autoResumeOccurrenceId({
      expectedRestart: runtimeRestartRecovery(["system-self"], "restart-z"),
    }),
    "runtime-restart:restart-z",
  );
  assert.equal(
    autoResumeOccurrenceId({
      hasDurableRestartRecovery: true,
      expectedRestart: { payloadRecoveryId: " payload " },
    }),
    "payload",
  );
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
