"use strict";

async function notifyRecoverableRestartErrorOnSelf({
  sourceThreadId,
  noticeId,
  prompt,
  readThread,
  subscribeThread,
  submitRecoveryMessage,
  verifyAttempts = 40,
  verifyDelayMs = 250,
} = {}) {
  const originalThreadId = requiredString(sourceThreadId, "source thread id");
  const itemId = requiredString(noticeId, "recovery notice id");
  const text = requiredString(prompt, "recovery prompt");
  if (
    typeof readThread !== "function" ||
    typeof subscribeThread !== "function" ||
    typeof submitRecoveryMessage !== "function"
  ) {
    throw new Error("recoverable restart self notification adapter is unavailable");
  }

  const sourceReadResult = await readThread(originalThreadId, true);
  const sourceThread = sourceReadResult?.thread ?? null;
  if (sourceThread && sourceThread.id !== originalThreadId) {
    throw new Error("source thread read returned a different thread");
  }
  if (!sourceThread || !isExactSelfThread(sourceThread)) {
    throw new Error("source thread is not exact /self");
  }
  const thread = sourceThread;
  const selfThreadId = originalThreadId;
  if (threadHasRecoveryNotice(thread, itemId, text)) {
    return {
      selfThreadId,
      sourceThreadId: originalThreadId,
      skipped: true,
    };
  }
  await subscribeThread(selfThreadId);
  const result = await submitRecoveryMessage(thread, {
    id: itemId,
    text,
  });
  const verified = await waitForRecoveryNotice({
    selfThreadId,
    itemId,
    text,
    readThread,
    attempts: verifyAttempts,
    delayMs: verifyDelayMs,
  });
  if (!verified) {
    throw new Error("self recovery notice was not durable after submission");
  }
  return {
    selfThreadId,
    sourceThreadId: originalThreadId,
    result,
  };
}

async function waitForRecoveryNotice({
  selfThreadId,
  itemId,
  text,
  readThread,
  attempts,
  delayMs,
}) {
  const maxAttempts = Math.max(1, Number(attempts) || 1);
  const waitMs = Math.max(0, Number(delayMs) || 0);
  for (let attempt = 0; attempt < maxAttempts; attempt += 1) {
    const verifyReadResult = await readThread(selfThreadId, true);
    const verifiedThread = verifyReadResult?.thread ?? null;
    if (verifiedThread?.id !== selfThreadId) {
      throw new Error("self recovery notice verification read mismatched thread");
    }
    if (!isExactSelfThread(verifiedThread)) {
      throw new Error("self recovery notice verification read is not exact /self");
    }
    if (threadHasRecoveryNotice(verifiedThread, itemId, text)) {
      return true;
    }
    if (attempt + 1 < maxAttempts && waitMs > 0) {
      await delay(waitMs);
    }
  }
  return false;
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function isExactSelfThread(thread) {
  return (
    thread?.name === "/self" ||
    thread?.agentPath === "/self" ||
    thread?.agent_path === "/self"
  );
}

function threadHasRecoveryNotice(thread, _itemId, text) {
  for (const turn of thread?.turns ?? []) {
    for (const item of turn.items ?? []) {
      if (itemTextMatches(item, text)) {
        return true;
      }
    }
  }
  return false;
}

function itemTextMatches(item, text) {
  if (item?.type !== "userMessage") {
    return false;
  }
  if (item.text === text) {
    return true;
  }
  for (const content of item.content ?? []) {
    if (
      (content?.type === "text" || content?.type === "output_text") &&
      content.text === text
    ) {
      return true;
    }
  }
  return false;
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
