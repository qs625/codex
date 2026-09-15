const assert = require("node:assert/strict");
const test = require("node:test");

const {
  notifyRecoverableRestartErrorOnSelf,
} = require("./restartRecoverySelfNotice.cjs");

test("recoverable restart errors submit a durable recovery user message on exact /self", async () => {
  const calls = [];
  const selfThread = {
    id: "origin-thread",
    name: "/self",
    model: "gpt",
    modelProvider: "openai",
    reasoningEffort: "medium",
    turns: [],
  };
  const result = await notifyRecoverableRestartErrorOnSelf({
    sourceThreadId: "origin-thread",
    noticeId: "runtime-restart-recovery:restart-1",
    prompt: "中文恢复通知；原始 thread id：origin-thread。",
    listThreads: async () => {
      throw new Error("must not choose a fallback /self thread");
    },
    readThread: async (threadId) => {
      calls.push(["read", threadId]);
      return { thread: selfThread };
    },
    subscribeThread: async (threadId) => calls.push(["subscribe", threadId]),
    submitRecoveryMessage: async (thread, message) => {
      calls.push(["submit", thread.id, message.id, message.text]);
      thread.turns.push({
        items: [
          {
            type: "userMessage",
            id: message.id,
            content: [{ type: "text", text: message.text }],
          },
        ],
      });
      return {};
    },
  });

  assert.deepEqual(calls, [
    ["read", "origin-thread"],
    ["subscribe", "origin-thread"],
    [
      "submit",
      "origin-thread",
      "runtime-restart-recovery:restart-1",
      "中文恢复通知；原始 thread id：origin-thread。",
    ],
    ["read", "origin-thread"],
  ]);
  assert.equal(result.selfThreadId, "origin-thread");
  assert.equal(result.sourceThreadId, "origin-thread");
});

test("recoverable restart errors reject when source is not exact /self", async () => {
  const calls = [];
  await assert.rejects(
    () =>
      notifyRecoverableRestartErrorOnSelf({
        sourceThreadId: "project-thread",
        noticeId: "runtime-restart-recovery:restart-1",
        prompt: "中文恢复通知",
        listThreads: async () => {
          calls.push(["list"]);
          return { selfProjectThreadId: "self-thread" };
        },
        readThread: async (threadId) => {
          calls.push(["read", threadId]);
          return { thread: { id: threadId, name: "Project" } };
        },
        subscribeThread: async () => {
          throw new Error("must not subscribe");
        },
        submitRecoveryMessage: async () => {
          throw new Error("must not submit");
        },
      }),
    /source thread is not exact \/self/,
  );

  assert.deepEqual(calls, [["read", "project-thread"]]);
});

test("recoverable restart errors reject when submission is not durably readable", async () => {
  const calls = [];
  await assert.rejects(
    () =>
      notifyRecoverableRestartErrorOnSelf({
        sourceThreadId: "origin-thread",
        noticeId: "runtime-restart-recovery:restart-1",
        prompt: "中文恢复通知",
        listThreads: async () => {
          throw new Error("must not choose a fallback /self thread");
        },
        readThread: async (threadId) => {
          calls.push(["read", threadId]);
          return {
            thread: {
              id: threadId,
              name: "/self",
              turns: [],
            },
          };
        },
        subscribeThread: async (threadId) => calls.push(["subscribe", threadId]),
        submitRecoveryMessage: async (thread, message) => {
          calls.push(["submit", thread.id, message.id, message.text]);
          return {};
        },
        verifyAttempts: 1,
      }),
    /self recovery notice was not durable after submission/,
  );

  assert.deepEqual(calls, [
    ["read", "origin-thread"],
    ["subscribe", "origin-thread"],
    ["submit", "origin-thread", "runtime-restart-recovery:restart-1", "中文恢复通知"],
    ["read", "origin-thread"],
  ]);
});

test("recoverable restart errors detect an existing user content notice", async () => {
  const result = await notifyRecoverableRestartErrorOnSelf({
    sourceThreadId: "origin-thread",
    noticeId: "runtime-restart-recovery:restart-1",
    prompt: "中文恢复通知",
    listThreads: async () => {
      throw new Error("must not choose a fallback /self thread");
    },
    readThread: async () => ({
      thread: {
        id: "origin-thread",
        name: "/self",
        turns: [
          {
            items: [
              {
                type: "userMessage",
                id: "other-id",
                content: [{ type: "text", text: "中文恢复通知" }],
              },
            ],
          },
        ],
      },
    }),
    subscribeThread: async () => {
      throw new Error("must not subscribe");
    },
    submitRecoveryMessage: async () => {
      throw new Error("must not submit");
    },
  });

  assert.equal(result.skipped, true);
});

test("recoverable restart errors skip an existing durable notice", async () => {
  const calls = [];
  const result = await notifyRecoverableRestartErrorOnSelf({
    sourceThreadId: "origin-thread",
    noticeId: "runtime-restart-recovery:restart-1",
    prompt: "中文恢复通知",
    listThreads: async () => ({ selfProjectThreadId: "self-thread" }),
    readThread: async (threadId) => {
      calls.push(["read", threadId]);
      return {
        thread: {
          id: threadId,
          name: "/self",
          turns: [
            {
              items: [
                {
                  type: "userMessage",
                  id: "runtime-restart-recovery:restart-1",
                  content: [{ type: "text", text: "中文恢复通知" }],
                },
              ],
            },
          ],
        },
      };
    },
    subscribeThread: async () => {
      throw new Error("must not subscribe");
    },
    submitRecoveryMessage: async () => {
      throw new Error("must not submit");
    },
  });

  assert.deepEqual(calls, [["read", "origin-thread"]]);
  assert.equal(result.skipped, true);
  assert.equal(result.selfThreadId, "origin-thread");
});

test("recoverable restart errors skip an existing durable notice by typed id", async () => {
  const calls = [];
  const result = await notifyRecoverableRestartErrorOnSelf({
    sourceThreadId: "origin-thread",
    noticeId: "runtime-restart-recovery:restart-1",
    prompt: "新版恢复通知",
    readThread: async (threadId) => {
      calls.push(["read", threadId]);
      return {
        thread: {
          id: threadId,
          name: "/self",
          turns: [
            {
              items: [
                {
                  type: "userMessage",
                  id: "runtime-restart-recovery:restart-1",
                  content: [{ type: "text", text: "旧版恢复通知" }],
                },
              ],
            },
          ],
        },
      };
    },
    subscribeThread: async () => {
      throw new Error("must not subscribe");
    },
    submitRecoveryMessage: async () => {
      throw new Error("must not submit");
    },
  });

  assert.deepEqual(calls, [["read", "origin-thread"]]);
  assert.equal(result.skipped, true);
});

test("recoverable restart errors wait for submitted user message to become durable", async () => {
  const calls = [];
  let readCount = 0;
  let submitted = false;
  const result = await notifyRecoverableRestartErrorOnSelf({
    sourceThreadId: "origin-thread",
    noticeId: "runtime-restart-recovery:restart-1",
    prompt: "中文恢复通知",
    readThread: async (threadId) => {
      readCount += 1;
      calls.push(["read", threadId]);
      return {
        thread: {
          id: threadId,
          name: "/self",
          turns:
            submitted && readCount >= 4
              ? [
                  {
                    items: [
                      {
                        type: "userMessage",
                        id: "submitted-user-message",
                        content: [{ type: "text", text: "中文恢复通知" }],
                      },
                    ],
                  },
                ]
              : [],
        },
      };
    },
    subscribeThread: async (threadId) => calls.push(["subscribe", threadId]),
    submitRecoveryMessage: async (thread, message) => {
      calls.push(["submit", thread.id, message.id, message.text]);
      submitted = true;
      return {};
    },
    verifyAttempts: 3,
    verifyDelayMs: 0,
  });

  assert.equal(result.selfThreadId, "origin-thread");
  assert.deepEqual(calls, [
    ["read", "origin-thread"],
    ["subscribe", "origin-thread"],
    ["submit", "origin-thread", "runtime-restart-recovery:restart-1", "中文恢复通知"],
    ["read", "origin-thread"],
    ["read", "origin-thread"],
    ["read", "origin-thread"],
  ]);
});

test("recoverable restart errors do not accept client recovery as durable user notice", async () => {
  const calls = [];
  let submitted = false;
  const result = await notifyRecoverableRestartErrorOnSelf({
    sourceThreadId: "origin-thread",
    noticeId: "runtime-restart-recovery:restart-1",
    prompt: "中文恢复通知",
    readThread: async (threadId) => {
      calls.push(["read", threadId]);
      return {
        thread: {
          id: threadId,
          name: "/self",
          turns: [
            {
              items: [
                {
                  type: "clientRecovery",
                  id: "runtime-restart-recovery:restart-1",
                  reason: "中文恢复通知",
                },
                ...(submitted
                  ? [
                      {
                        type: "userMessage",
                        id: "submitted-user-message",
                        content: [{ type: "text", text: "中文恢复通知" }],
                      },
                    ]
                  : []),
              ],
            },
          ],
        },
      };
    },
    subscribeThread: async (threadId) => calls.push(["subscribe", threadId]),
    submitRecoveryMessage: async (thread, message) => {
      calls.push(["submit", thread.id, message.id, message.text]);
      submitted = true;
      return {};
    },
  });

  assert.equal(result.selfThreadId, "origin-thread");
  assert.deepEqual(calls, [
    ["read", "origin-thread"],
    ["subscribe", "origin-thread"],
    ["submit", "origin-thread", "runtime-restart-recovery:restart-1", "中文恢复通知"],
    ["read", "origin-thread"],
  ]);
});

test("recoverable restart errors skip a legacy user-turn notice", async () => {
  const result = await notifyRecoverableRestartErrorOnSelf({
    sourceThreadId: "origin-thread",
    noticeId: "runtime-restart-recovery:restart-1",
    prompt: "中文恢复通知",
    listThreads: async () => ({ selfProjectThreadId: "self-thread" }),
    readThread: async () => ({
      thread: {
        id: "origin-thread",
        name: "/self",
        turns: [
          {
            items: [
              {
                type: "userMessage",
                id: "legacy-user-turn",
                content: [{ type: "text", text: "中文恢复通知" }],
              },
            ],
          },
        ],
      },
    }),
    subscribeThread: async () => {
      throw new Error("must not subscribe");
    },
    submitRecoveryMessage: async () => {
      throw new Error("must not submit");
    },
  });

  assert.equal(result.skipped, true);
});

test("recoverable restart errors reject a non-exact /self target", async () => {
  await assert.rejects(
    () =>
      notifyRecoverableRestartErrorOnSelf({
        sourceThreadId: "origin-thread",
        noticeId: "runtime-restart-recovery:restart-1",
        prompt: "中文恢复通知",
        listThreads: async () => ({ materializedSelfThreadId: "not-enough" }),
        readThread: async (threadId) => ({ thread: { id: threadId } }),
        subscribeThread: async () => {
          throw new Error("must not subscribe");
        },
        submitRecoveryMessage: async () => {
          throw new Error("must not submit");
        },
      }),
    /source thread is not exact \/self/,
  );
});

test("recoverable restart errors reject a mismatched /self read result", async () => {
  await assert.rejects(
    () =>
      notifyRecoverableRestartErrorOnSelf({
        sourceThreadId: "origin-thread",
        noticeId: "runtime-restart-recovery:restart-1",
        prompt: "中文恢复通知",
        listThreads: async () => ({ selfProjectThreadId: "self-thread" }),
        readThread: async (threadId) => ({
          thread:
            threadId === "origin-thread"
              ? { id: "another-thread" }
              : { id: "another-thread" },
        }),
        subscribeThread: async () => {
          throw new Error("must not subscribe");
        },
        submitRecoveryMessage: async () => {
          throw new Error("must not submit");
        },
      }),
    /source thread read returned a different thread/,
  );
});

test("recoverable restart errors reject source mismatch even when fallback /self exists", async () => {
  const calls = [];
  await assert.rejects(
    () =>
      notifyRecoverableRestartErrorOnSelf({
        sourceThreadId: "origin-thread",
        noticeId: "runtime-restart-recovery:restart-1",
        prompt: "中文恢复通知",
        listThreads: async () => {
          calls.push(["list"]);
          return { selfProjectThreadId: "self-thread" };
        },
        readThread: async (threadId) => {
          calls.push(["read", threadId]);
          return {
            thread:
              threadId === "origin-thread"
                ? { id: "another-thread", name: "/self" }
                : { id: "self-thread", name: "/self" },
          };
        },
        subscribeThread: async () => {
          throw new Error("must not subscribe");
        },
        submitRecoveryMessage: async () => {
          throw new Error("must not submit");
        },
      }),
    /source thread read returned a different thread/,
  );
  assert.deepEqual(calls, [["read", "origin-thread"]]);
});
