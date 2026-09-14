"use strict";

async function notifyRecoverableRestartErrorOnSelf({
  sourceThreadId,
  noticeId,
  prompt,
  listThreads,
  readThread,
  subscribeThread,
  injectConversationMessage,
} = {}) {
  const originalThreadId = requiredString(sourceThreadId, "source thread id");
  const itemId = requiredString(noticeId, "recovery notice id");
  const text = requiredString(prompt, "recovery prompt");
  if (
    typeof listThreads !== "function" ||
    typeof readThread !== "function" ||
    typeof subscribeThread !== "function" ||
    typeof injectConversationMessage !== "function"
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
  return {
    selfThreadId,
    sourceThreadId: originalThreadId,
    result,
  };
}

function threadHasRecoveryNotice(thread, itemId, text) {
  for (const turn of thread?.turns ?? []) {
    for (const item of turn.items ?? []) {
      if (item?.id === itemId) {
        return true;
      }
      if (item?.type === "agentMessage" && item.text === text) {
        return true;
      }
      if (item?.type !== "userMessage") {
        continue;
      }
      for (const content of item.content ?? []) {
        if (content?.type === "text" && content.text === text) {
          return true;
        }
      }
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
