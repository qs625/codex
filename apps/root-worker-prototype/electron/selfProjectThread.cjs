const { buildSelfCommandThreadStartParams } = require("./threadConfig.cjs");

const SELF_PROJECT_THREAD_NAME = "/self";

function isSelfProjectThread(thread, project) {
  const systemThreadId = normalizeThreadId(project?.systemThreadId);
  return (
    isManagedSystemSelfProject(project) &&
    systemThreadId !== null &&
    thread?.id === systemThreadId &&
    thread?.name === SELF_PROJECT_THREAD_NAME &&
    normalizeProjectPath(thread?.cwd) === normalizeProjectPath(project.workspace) &&
    !thread?.parentThreadId &&
    !thread?.parent_thread_id &&
    !thread?.forkedFromId &&
    !thread?.forked_from_id &&
    thread?.threadSource !== "subagent" &&
    !isSubAgentSource(thread?.source)
  );
}

async function ensureSelfProjectThread(
  appServerClient,
  normalizeThread,
  persistSystemThreadId,
  project,
  threads,
) {
  const existing = threads.find((thread) => isSelfProjectThread(thread, project));
  if (existing) {
    return { created: false, runtime: null, thread: existing, threads };
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
