const AUTO_RESUME_PROMPT =
  "The Morpheus client restarted and restored this interrupted session. Review the recovered context and decide whether to continue, rerun checks, explain the interruption, or stop.";
const EXTERNAL_THREAD_PROVIDER_IDS = new Set([
  "claude_cli",
  "codex_cli",
  "opencode",
]);

function createThreadAutoResumeCoordinator({
  readThread,
  subscribeThread,
  sendResumeInput,
  stateStore,
  logger = console,
} = {}) {
  const inFlightKeys = new Set();
  const completedKeys = new Set();

  return {
    async run(threads = []) {
      const resumedThreadIds = [];
      const skippedThreadIds = [];
      const failedThreadIds = [];
      const errors = [];

      for (const thread of pickAutoResumeCandidates(threads)) {
        const key = autoResumeFingerprint(thread);
        if (
          !key ||
          inFlightKeys.has(key) ||
          completedKeys.has(key) ||
          (await hasCompletedAutoResume(stateStore, key, logger))
        ) {
          skippedThreadIds.push(thread.id);
          continue;
        }

        inFlightKeys.add(key);
        try {
          const readResult = await readThread(thread.id, true);
          const restoredThread = readResult?.thread ?? thread;
          if (!isAutoResumeEligibleThread(restoredThread)) {
            skippedThreadIds.push(thread.id);
            continue;
          }
          if (threadHasAutoResumePrompt(restoredThread)) {
            completedKeys.add(key);
            await markCompletedAutoResume(stateStore, key, logger);
            skippedThreadIds.push(thread.id);
            continue;
          }

          await subscribeThread(thread.id);
          await sendResumeInput(restoredThread);
          completedKeys.add(key);
          await markCompletedAutoResume(stateStore, key, logger);
          resumedThreadIds.push(thread.id);
        } catch (error) {
          failedThreadIds.push(thread.id);
          const message =
            error instanceof Error ? error.message : String(error);
          errors.push({ threadId: thread.id, message });
          logger.warn?.(
            "[prototype] failed to auto-resume interrupted thread",
            JSON.stringify({ threadId: thread.id, message }),
          );
        } finally {
          inFlightKeys.delete(key);
        }
      }

      return {
        resumedThreadIds,
        skippedThreadIds,
        failedThreadIds,
        errors,
        focusThreadId: resumedThreadIds[0] ?? null,
      };
    },
  };
}

async function hasCompletedAutoResume(stateStore, key, logger) {
  try {
    return Boolean(await stateStore?.has?.(key));
  } catch (error) {
    logger.warn?.(
      "[prototype] failed to read auto-resume state",
      JSON.stringify({ key, message: errorMessage(error) }),
    );
    return false;
  }
}

async function markCompletedAutoResume(stateStore, key, logger) {
  try {
    await stateStore?.mark?.(key);
  } catch (error) {
    logger.warn?.(
      "[prototype] failed to persist auto-resume state",
      JSON.stringify({ key, message: errorMessage(error) }),
    );
  }
}

function pickAutoResumeCandidates(threads = []) {
  return threads
    .filter(isAutoResumeEligibleThread)
    .sort((left, right) => (right.updatedAt ?? 0) - (left.updatedAt ?? 0));
}

function isAutoResumeEligibleThread(thread) {
  if (!thread?.id || !isInterruptedLifecycleStatus(thread.lifecycleStatus)) {
    return false;
  }
  if (thread.ephemeral || thread.threadSource === "subagent") {
    return false;
  }
  if (thread.agentPath || thread.agentRole || thread.agentNickname) {
    return false;
  }
  if (isSubAgentSource(thread.source)) {
    return false;
  }

  const provider = readThreadProvider(thread);
  if (!provider) {
    return thread.source === "appServer" || thread.threadSource === "user";
  }
  if (provider.id === "native") {
    return true;
  }
  return provider.restoreThread === true;
}

function autoResumeFingerprint(thread) {
  if (!thread?.id) {
    return null;
  }
  return `${thread.id}:${thread.updatedAt ?? "unknown"}`;
}

function isInterruptedLifecycleStatus(status) {
  return (
    status?.type === "final" &&
    (status.result?.type === "interrupted" || status.result === "interrupted")
  );
}

function readThreadProvider(thread) {
  let providerId =
    thread.threadProvider ??
    thread.thread_provider ??
    thread.provider?.id ??
    thread.providerId ??
    null;
  if (!providerId) {
    const modelProvider = thread.modelProvider ?? thread.model_provider ?? null;
    if (EXTERNAL_THREAD_PROVIDER_IDS.has(modelProvider)) {
      providerId = modelProvider;
    }
  }
  if (!providerId) {
    return null;
  }
  const capabilities =
    thread.provider?.capabilities ?? thread.capabilities ?? {};
  return {
    id: providerId,
    restoreThread: capabilities.restoreThread ?? thread.restoreThread ?? false,
  };
}

function isSubAgentSource(source) {
  return Boolean(
    source &&
      typeof source === "object" &&
      Object.prototype.hasOwnProperty.call(source, "subAgent"),
  );
}

function threadHasAutoResumePrompt(thread) {
  for (const turn of thread.turns ?? []) {
    for (const item of turn.items ?? []) {
      if (item.type !== "userMessage") {
        continue;
      }
      for (const content of item.content ?? []) {
        if (content?.type === "text" && content.text === AUTO_RESUME_PROMPT) {
          return true;
        }
      }
    }
  }
  return false;
}

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

function createJsonAutoResumeStateStore(filePath, fs) {
  let loaded = null;

  async function load() {
    if (loaded) {
      return loaded;
    }
    try {
      const raw = await fs.readFile(filePath, "utf8");
      const parsed = JSON.parse(raw);
      loaded = new Set(
        Array.isArray(parsed?.completed) ? parsed.completed : [],
      );
    } catch (error) {
      if (error?.code !== "ENOENT") {
        throw error;
      }
      loaded = new Set();
    }
    return loaded;
  }

  return {
    async has(key) {
      return (await load()).has(key);
    },
    async mark(key) {
      const completed = await load();
      completed.add(key);
      await fs.mkdir(require("node:path").dirname(filePath), {
        recursive: true,
      });
      await fs.writeFile(
        filePath,
        `${JSON.stringify({ completed: [...completed].sort() }, null, 2)}\n`,
        "utf8",
      );
    },
  };
}

module.exports = {
  AUTO_RESUME_PROMPT,
  autoResumeFingerprint,
  createJsonAutoResumeStateStore,
  createThreadAutoResumeCoordinator,
  isAutoResumeEligibleThread,
  isInterruptedLifecycleStatus,
  pickAutoResumeCandidates,
  threadHasAutoResumePrompt,
};
