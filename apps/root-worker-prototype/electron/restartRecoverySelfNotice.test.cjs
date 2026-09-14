const assert = require("node:assert/strict");
const test = require("node:test");

const {
  notifyRecoverableRestartErrorOnSelf,
} = require("./restartRecoverySelfNotice.cjs");

test("recoverable restart errors inject a durable message on exact /self", async () => {
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
    injectConversationMessage: async (thread, message) => {
      calls.push(["inject", thread.id, message.id, message.text]);
      thread.turns.push({
        items: [
          {
            type: "agentMessage",
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
      "inject",
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
        injectConversationMessage: async () => {
          throw new Error("must not inject");
        },
      }),
    /source thread is not exact \/self/,
  );

  assert.deepEqual(calls, [["read", "project-thread"]]);
});

test("recoverable restart errors reject when injection is not durably readable", async () => {
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
        injectConversationMessage: async (thread, message) => {
          calls.push(["inject", thread.id, message.id, message.text]);
          return {};
        },
      }),
    /self recovery notice was not durable after injection/,
  );

  assert.deepEqual(calls, [
    ["read", "origin-thread"],
    ["subscribe", "origin-thread"],
    ["inject", "origin-thread", "runtime-restart-recovery:restart-1", "中文恢复通知"],
    ["read", "origin-thread"],
  ]);
});

test("recoverable restart errors detect an existing agent content notice", async () => {
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
                type: "agentMessage",
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
    injectConversationMessage: async () => {
      throw new Error("must not inject");
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
                  type: "agentMessage",
                  id: "runtime-restart-recovery:restart-1",
                  text: "中文恢复通知",
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
    injectConversationMessage: async () => {
      throw new Error("must not inject");
    },
  });

  assert.deepEqual(calls, [["read", "origin-thread"]]);
  assert.equal(result.skipped, true);
  assert.equal(result.selfThreadId, "origin-thread");
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
    injectConversationMessage: async () => {
      throw new Error("must not inject");
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
        injectConversationMessage: async () => {
          throw new Error("must not inject");
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
        injectConversationMessage: async () => {
          throw new Error("must not inject");
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
        injectConversationMessage: async () => {
          throw new Error("must not inject");
        },
      }),
    /source thread read returned a different thread/,
  );
  assert.deepEqual(calls, [["read", "origin-thread"]]);
});
