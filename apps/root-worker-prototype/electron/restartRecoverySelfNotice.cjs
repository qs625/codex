"use strict";

async function notifyRecoverableRestartErrorOnSelf({
  sourceThreadId,
  prompt,
  listThreads,
  readThread,
  subscribeThread,
  sendUserInput,
} = {}) {
  const originalThreadId = requiredString(sourceThreadId, "source thread id");
  const text = requiredString(prompt, "recovery prompt");
  if (
    typeof listThreads !== "function" ||
    typeof readThread !== "function" ||
    typeof subscribeThread !== "function" ||
    typeof sendUserInput !== "function"
  ) {
    throw new Error("recoverable restart self notification adapter is unavailable");
  }

  const listResult = await listThreads();
  const selfThreadId = requiredString(
    listResult?.selfProjectThreadId,
    "exact /self thread id",
  );
  const readResult = await readThread(selfThreadId, true);
  const thread = readResult?.thread ?? null;
  if (thread?.id !== selfThreadId) {
    throw new Error("exact /self thread could not be read");
  }
  await subscribeThread(selfThreadId);
  const result = await sendUserInput(thread, text);
  return {
    selfThreadId,
    sourceThreadId: originalThreadId,
    result,
  };
}

function requiredString(value, label) {
  if (typeof value !== "string" || !value.trim()) {
    throw new Error(`recoverable restart notification requires ${label}`);
  }
  return value.trim();
}

module.exports = {
  notifyRecoverableRestartErrorOnSelf,
};
