const assert = require("node:assert/strict");
const test = require("node:test");

const {
  notifyRecoverableRestartErrorOnSelf,
} = require("./restartRecoverySelfNotice.cjs");

test("recoverable restart errors inject a durable message on exact /self", async () => {
  const calls = [];
  const result = await notifyRecoverableRestartErrorOnSelf({
    sourceThreadId: "origin-thread",
    noticeId: "runtime-restart-recovery:restart-1",
    prompt: "中文恢复通知；原始 thread id：origin-thread。",
    listThreads: async () => ({ selfProjectThreadId: "self-thread" }),
    readThread: async (threadId) => {
      calls.push(["read", threadId]);
      return {
        thread: {
          id: threadId,
          model: "gpt",
          modelProvider: "openai",
          reasoningEffort: "medium",
        },
      };
    },
    subscribeThread: async (threadId) => calls.push(["subscribe", threadId]),
    injectConversationMessage: async (thread, message) => {
      calls.push(["inject", thread.id, message.id, message.text]);
      return {};
    },
  });

  assert.deepEqual(calls, [
    ["read", "self-thread"],
    ["subscribe", "self-thread"],
    [
      "inject",
      "self-thread",
      "runtime-restart-recovery:restart-1",
      "中文恢复通知；原始 thread id：origin-thread。",
    ],
  ]);
  assert.equal(result.selfThreadId, "self-thread");
  assert.equal(result.sourceThreadId, "origin-thread");
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

  assert.deepEqual(calls, [["read", "self-thread"]]);
  assert.equal(result.skipped, true);
  assert.equal(result.selfThreadId, "self-thread");
});

test("recoverable restart errors skip a legacy user-turn notice", async () => {
  const result = await notifyRecoverableRestartErrorOnSelf({
    sourceThreadId: "origin-thread",
    noticeId: "runtime-restart-recovery:restart-1",
    prompt: "中文恢复通知",
    listThreads: async () => ({ selfProjectThreadId: "self-thread" }),
    readThread: async () => ({
      thread: {
        id: "self-thread",
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
        readThread: async () => {
          throw new Error("must not read");
        },
        subscribeThread: async () => {
          throw new Error("must not subscribe");
        },
        injectConversationMessage: async () => {
          throw new Error("must not inject");
        },
      }),
    /exact \/self thread id/,
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
        readThread: async () => ({ thread: { id: "another-thread" } }),
        subscribeThread: async () => {
          throw new Error("must not subscribe");
        },
        injectConversationMessage: async () => {
          throw new Error("must not inject");
        },
      }),
    /exact \/self thread could not be read/,
  );
});
