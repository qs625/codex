const { buildSelfCommandThreadStartParams } = require("./threadConfig.cjs");

const SELF_PROJECT_THREAD_NAME = "/self";

function isSelfProjectThread(thread, project) {
  return (
    thread?.name === SELF_PROJECT_THREAD_NAME ||
    thread?.agentPath === SELF_PROJECT_THREAD_NAME ||
    thread?.agent_path === SELF_PROJECT_THREAD_NAME
  );
}

async function ensureSelfProjectThread(
  appServerClient,
  normalizeThread,
  persistSystemThreadId,
  project,
  threads,
) {
  const selfThreads = threads.filter((thread) =>
    isSelfProjectThread(thread, project),
  );
  const existing =
    selfThreads.find((thread) => thread.id === project.systemThreadId) ??
    selfThreads[0];
  if (existing) {
    // Older clients could create more than one /self thread when their local
    // project record did not point at the backend-returned thread. Agent paths
    // are global, so never expose those stale roots for a later resume.
    const duplicateSelfThreads = selfThreads.filter(
      (thread) => thread.id !== existing.id,
    );
    await Promise.all(
      duplicateSelfThreads.map(async (thread) => {
        try {
          await appServerClient.request("thread/archive", { threadId: thread.id });
        } catch {
          // Keep the canonical root usable if cleanup needs to be retried.
        }
      }),
    );
    project.systemThreadId = existing.id;
    if (typeof persistSystemThreadId === "function") {
      await persistSystemThreadId(existing.id);
    }
    return {
      created: false,
      runtime: null,
      thread: existing,
      threads: threads.filter((thread) => !duplicateSelfThreads.includes(thread)),
    };
  }

  const start = await appServerClient.request(
    "thread/start",
    buildSelfCommandThreadStartParams(project),
  );
  await appServerClient.request("thread/name/set", {
    threadId: start.thread.id,
    name: SELF_PROJECT_THREAD_NAME,
  });
  const runtime = {
    model: start.model ?? null,
    modelProvider: start.modelProvider ?? null,
    reasoningEffort: start.reasoningEffort ?? null,
  };
  const thread = normalizeThread(
    { ...start.thread, name: SELF_PROJECT_THREAD_NAME },
    runtime,
  );
  project.systemThreadId = thread.id;
  if (typeof persistSystemThreadId === "function") {
    await persistSystemThreadId(thread.id);
  }
  return {
    created: true,
    runtime,
    thread,
    threads: [thread, ...threads],
  };
}

async function sendSelfCommandToThread({
  appServerClient,
  buildTurnInput,
  loadThreadForTurn,
  normalizeThread,
  project,
  persistSystemThreadId,
  rememberThreadRuntime,
  startThreadTurn,
  text,
  threads,
}) {
  const commandText = typeof text === "string" ? text.trim() : "";
  if (!commandText) {
    throw new Error("Self command requires task text.");
  }

  const result = await ensureSelfProjectThread(
    appServerClient,
    normalizeThread,
    persistSystemThreadId,
    project,
    threads,
  );
  if (result.created) {
    rememberThreadRuntime(result.thread.id, result.runtime);
  }
  const threadForTurn = result.created
    ? result.thread
    : await loadThreadForTurn(result.thread.id);
  if (!threadForTurn) {
    throw new Error("Self thread is unavailable.");
  }

  const turnPayload = {
    threadId: threadForTurn.id,
    model: threadForTurn.model ?? null,
    modelProvider: threadForTurn.modelProvider ?? null,
    effort: threadForTurn.reasoningEffort ?? null,
    text: commandText,
    skills: [],
    images: [],
  };
  const turn = await startThreadTurn(turnPayload, buildTurnInput(turnPayload));

  return {
    materializedSelfThreadId: result.created ? result.thread.id : null,
    thread: threadForTurn,
    threads: result.threads,
    turn,
  };
}

function normalizePath(value) {
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim();
  if (!trimmed) {
    return null;
  }
  const normalized = trimmed.replaceAll("\\", "/").replace(/\/+$/, "");
  return normalized || "/";
}

function normalizeProjectPath(value) {
  return normalizePath(value);
}

function normalizeThreadId(value) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function isManagedSystemSelfProject(project) {
  return (
    project?.id === SELF_PROJECT_THREAD_NAME &&
    project?.path === SELF_PROJECT_THREAD_NAME &&
    project?.system === true &&
    project?.managedBy === "morpheus"
  );
}

function isSubAgentSource(source) {
  return (
    source &&
    typeof source === "object" &&
    Object.prototype.hasOwnProperty.call(source, "subAgent")
  );
}

module.exports = {
  SELF_PROJECT_THREAD_NAME,
  ensureSelfProjectThread,
  isSelfProjectThread,
  sendSelfCommandToThread,
};
