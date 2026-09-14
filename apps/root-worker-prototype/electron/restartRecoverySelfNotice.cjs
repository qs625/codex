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

  const sourceReadResult = await readThread(originalThreadId, true);
  const sourceThread = sourceReadResult?.thread ?? null;
  if (sourceThread && sourceThread.id !== originalThreadId) {
    throw new Error("source thread read returned a different thread");
  }
  const target =
    sourceThread && isExactSelfThread(sourceThread)
      ? { thread: sourceThread, threadId: originalThreadId }
      : await loadCurrentSelfThread({ listThreads, readThread });
  const { thread, threadId: selfThreadId } = target;
  if (thread?.id !== selfThreadId || !isExactSelfThread(thread)) {
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

async function loadCurrentSelfThread({ listThreads, readThread }) {
  const listResult = await listThreads();
  const selfThreadId = requiredString(
    listResult?.selfProjectThreadId,
    "exact /self thread id",
  );
  const readResult = await readThread(selfThreadId, true);
  return {
    thread: readResult?.thread ?? null,
    threadId: selfThreadId,
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
