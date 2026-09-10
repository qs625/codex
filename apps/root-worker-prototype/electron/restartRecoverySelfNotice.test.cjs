const assert = require("node:assert/strict");
const test = require("node:test");

const {
  notifyRecoverableRestartErrorOnSelf,
} = require("./restartRecoverySelfNotice.cjs");

test("recoverable restart errors create a real user turn on exact /self", async () => {
  const calls = [];
  const result = await notifyRecoverableRestartErrorOnSelf({
    sourceThreadId: "origin-thread",
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
    sendUserInput: async (thread, text) => {
      calls.push(["send", thread.id, text]);
      return { turn: { id: "turn-1" } };
    },
  });

  assert.deepEqual(calls, [
    ["read", "self-thread"],
    ["subscribe", "self-thread"],
    ["send", "self-thread", "中文恢复通知；原始 thread id：origin-thread。"],
  ]);
  assert.equal(result.selfThreadId, "self-thread");
  assert.equal(result.sourceThreadId, "origin-thread");
});

test("recoverable restart errors reject a non-exact /self target", async () => {
  await assert.rejects(
    () =>
      notifyRecoverableRestartErrorOnSelf({
        sourceThreadId: "origin-thread",
        prompt: "中文恢复通知",
        listThreads: async () => ({ materializedSelfThreadId: "not-enough" }),
        readThread: async () => {
          throw new Error("must not read");
        },
        subscribeThread: async () => {
          throw new Error("must not subscribe");
        },
        sendUserInput: async () => {
          throw new Error("must not send");
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
        prompt: "中文恢复通知",
        listThreads: async () => ({ selfProjectThreadId: "self-thread" }),
        readThread: async () => ({ thread: { id: "another-thread" } }),
        subscribeThread: async () => {
          throw new Error("must not subscribe");
        },
        sendUserInput: async () => {
          throw new Error("must not send");
        },
      }),
    /exact \/self thread could not be read/,
  );
});
