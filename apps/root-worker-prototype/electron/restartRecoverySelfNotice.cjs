"use strict";

async function notifyRecoverableRestartErrorOnSelf({
  sourceThreadId,
  noticeId,
  prompt,
  readThread,
  subscribeThread,
  injectConversationMessage,
} = {}) {
  const originalThreadId = requiredString(sourceThreadId, "source thread id");
  const itemId = requiredString(noticeId, "recovery notice id");
  const text = requiredString(prompt, "recovery prompt");
  if (
    typeof readThread !== "function" ||
    typeof subscribeThread !== "function" ||
    typeof injectConversationMessage !== "function"
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
  const result = await injectConversationMessage(thread, {
    id: itemId,
    text,
  });
  const verifyReadResult = await readThread(selfThreadId, true);
  const verifiedThread = verifyReadResult?.thread ?? null;
  if (verifiedThread?.id !== selfThreadId) {
    throw new Error("self recovery notice verification read mismatched thread");
  }
  if (!isExactSelfThread(verifiedThread)) {
    throw new Error("self recovery notice verification read is not exact /self");
  }
  if (!threadHasRecoveryNotice(verifiedThread, itemId, text)) {
    throw new Error("self recovery notice was not durable after injection");
  }
  return {
    selfThreadId,
    sourceThreadId: originalThreadId,
    result,
  };
}

function isExactSelfThread(thread) {
  return (
    thread?.name === "/self" ||
    thread?.agentPath === "/self" ||
    thread?.agent_path === "/self"
  );
}

function threadHasRecoveryNotice(thread, itemId, text) {
  for (const turn of thread?.turns ?? []) {
    for (const item of turn.items ?? []) {
      if (item?.id === itemId) {
        return true;
      }
      if (itemTextMatches(item, text)) {
        return true;
      }
    }
  }
  return false;
}

function itemTextMatches(item, text) {
  if (item?.type !== "agentMessage" && item?.type !== "userMessage") {
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
