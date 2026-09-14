const path = require("node:path");
const {
  RESTART_RECOVERY_PROMPTS,
} = require("./restartRecoveryPrompts.cjs");
const {
  CHAT_COMPAT_CWD_BASENAME,
} = require("./threadConfig.cjs");

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
    async runAfterRuntimeRestartRecovery({
      hasDurableRestartRecovery = false,
      recoveryOccurrenceId = null,
      threads = [],
      expectedRestart,
    } = {}) {
      const occurrence = autoResumeOccurrence({
        expectedRestart,
        hasDurableRestartRecovery,
        recoveryOccurrenceId,
      });
      const occurrenceId = occurrence.id;
      if (
        !hasDurableRuntimeRestartRecovery({
          expectedRestart,
          hasDurableRestartRecovery,
          recoveryOccurrenceId: occurrenceId,
        })
      ) {
        return emptyAutoResumeResult();
      }
      return run(threads, occurrence);
    },
  };

  async function run(threads = [], occurrence = { id: null, source: "none" }) {
    const occurrenceId = occurrence.id;
    const resumedThreadIds = [];
    const skippedThreadIds = [];
    const failedThreadIds = [];
    const errors = [];

    for (const thread of pickAutoResumeCandidates(threads)) {
      const key = autoResumeFingerprint(thread, occurrenceId);
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
        if (
          threadHasAutoResumePrompt(restoredThread, occurrenceId, {
            allowAnyRecoveryPrompt:
              !occurrenceId || occurrence.source !== "expected",
          })
        ) {
          completedKeys.add(key);
          await markCompletedAutoResume(stateStore, key, logger);
          skippedThreadIds.push(thread.id);
          continue;
        }

        await subscribeThread(thread.id);
        await sendResumeInput(
          restoredThread,
          autoResumePromptForOccurrence(occurrenceId),
        );
        completedKeys.add(key);
        await markCompletedAutoResume(stateStore, key, logger);
        resumedThreadIds.push(thread.id);
      } catch (error) {
        failedThreadIds.push(thread.id);
        const message = error instanceof Error ? error.message : String(error);
        errors.push({ threadId: thread.id, message });
        logger.warn?.(
          "[prototype] failed to fan out restart recovery to project root",
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
  }
}

function hasDurableRuntimeRestartRecovery({
  expectedRestart,
  hasDurableRestartRecovery,
  recoveryOccurrenceId = null,
}) {
  return (
    hasDurableRestartRecovery === true ||
    Boolean(recoveryOccurrenceId) ||
    (Array.isArray(expectedRestart?.expectedThreadIds) &&
      expectedRestart.expectedThreadIds.length > 0)
  );
}

function emptyAutoResumeResult() {
  return {
    resumedThreadIds: [],
    skippedThreadIds: [],
    failedThreadIds: [],
    errors: [],
    focusThreadId: null,
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
  if (!thread?.id || !isRecoverableLifecycleStatus(thread.lifecycleStatus)) {
    return false;
  }
  if (!isProjectRootThread(thread)) {
    return false;
  }
  return true;
}

function autoResumeFingerprint(thread, occurrenceId = null) {
  if (!thread?.id) {
    return null;
  }
  const occurrence = normalizeAutoResumeOccurrenceId(occurrenceId);
  if (occurrence) {
    return `restart-v2:${occurrence}:${thread.id}`;
  }
  return `${thread.id}:${thread.updatedAt ?? "unknown"}`;
}

function autoResumeOccurrenceId({
  expectedRestart,
  hasDurableRestartRecovery,
  recoveryOccurrenceId,
} = {}) {
  return autoResumeOccurrence({
    expectedRestart,
    hasDurableRestartRecovery,
    recoveryOccurrenceId,
  }).id;
}

function autoResumeOccurrence({
  expectedRestart,
  hasDurableRestartRecovery,
  recoveryOccurrenceId,
} = {}) {
  const explicit = normalizeAutoResumeOccurrenceId(recoveryOccurrenceId);
  const expected = normalizeAutoResumeOccurrenceId(
    expectedRestart?.recoveryOccurrenceId,
  );
  if (explicit) {
    return {
      id: explicit,
      source: expected && explicit === expected ? "expected" : "explicit",
    };
  }
  if (expected) {
    return { id: expected, source: "expected" };
  }
  if (hasDurableRestartRecovery === true) {
    return {
      id: normalizeAutoResumeOccurrenceId(expectedRestart?.payloadRecoveryId),
      source: "payload",
    };
  }
  return { id: null, source: "none" };
}

function normalizeAutoResumeOccurrenceId(value) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function isInterruptedLifecycleStatus(status) {
  return (
    status?.type === "final" &&
    (status.result?.type === "interrupted" || status.result === "interrupted")
  );
}

function isProjectRootThread(thread) {
  if (thread.ephemeral || thread.threadSource === "subagent") {
    return false;
  }
  if (isSubAgentSource(thread.source)) {
    return false;
  }
  if (thread.parentThreadId || thread.parent_thread_id) {
    return false;
  }
  const cwd = typeof thread.cwd === "string" ? thread.cwd.trim() : "";
  return (
    cwd.length > 0 &&
    !cwd.startsWith("codex://") &&
    path.basename(cwd) !== CHAT_COMPAT_CWD_BASENAME
  );
}

function isCompletedFinalLifecycleStatus(status) {
  return status?.type === "final" && status.result?.type === "completed";
}

function isActiveLifecycleStatus(status) {
  return status?.type === "active";
}

function isRecoverableLifecycleStatus(status) {
  if (status?.type === "active" || status?.type === "waiting") {
    return true;
  }
  return (
    status?.type === "final" &&
    status.result?.type === "interrupted"
  );
}

function isSubAgentSource(source) {
  return Boolean(
    source &&
      typeof source === "object" &&
      Object.prototype.hasOwnProperty.call(source, "subAgent"),
  );
}

function autoResumePromptForOccurrence(occurrenceId = null) {
  const occurrence = normalizeAutoResumeOccurrenceId(occurrenceId);
  if (!occurrence) {
    return RESTART_RECOVERY_PROMPTS.projectRootFanout;
  }
  return `${RESTART_RECOVERY_PROMPTS.projectRootFanout}\n\n恢复标识：${occurrence}`;
}

function threadHasAutoResumePrompt(
  thread,
  occurrenceId = null,
  { allowAnyRecoveryPrompt = false } = {},
) {
  const expectedPrompt = autoResumePromptForOccurrence(occurrenceId);
  for (const turn of thread.turns ?? []) {
    for (const item of turn.items ?? []) {
      if (item.type !== "userMessage") {
        continue;
      }
      for (const content of item.content ?? []) {
        if (
          content?.type === "text" &&
          (content.text === expectedPrompt ||
            (allowAnyRecoveryPrompt &&
              isAutoResumeRecoveryPromptText(content.text)))
        ) {
          return true;
        }
      }
    }
  }
  return false;
}

function isAutoResumeRecoveryPromptText(text) {
  return (
    text === RESTART_RECOVERY_PROMPTS.projectRootFanout ||
    (typeof text === "string" &&
      text.startsWith(`${RESTART_RECOVERY_PROMPTS.projectRootFanout}\n\n恢复标识：`))
  );
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
  autoResumePromptForOccurrence,
  autoResumeFingerprint,
  autoResumeOccurrenceId,
  createJsonAutoResumeStateStore,
  createThreadAutoResumeCoordinator,
  hasDurableRuntimeRestartRecovery,
  isCompletedFinalLifecycleStatus,
  isActiveLifecycleStatus,
  isAutoResumeEligibleThread,
  isRecoverableLifecycleStatus,
  isInterruptedLifecycleStatus,
  isProjectRootThread,
  pickAutoResumeCandidates,
  threadHasAutoResumePrompt,
};
