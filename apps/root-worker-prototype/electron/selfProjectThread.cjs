const { buildSelfCommandThreadStartParams } = require("./threadConfig.cjs");

const SELF_PROJECT_THREAD_NAME = "/self";

function isSelfProjectThread(thread, project) {
  const workspace = normalizePath(project?.workspace);
  if (!workspace || normalizePath(thread?.cwd) !== workspace) {
    return false;
  }
  return (
    normalizeProjectPath(thread?.agentPath) === SELF_PROJECT_THREAD_NAME ||
    normalizeProjectPath(thread?.path) === SELF_PROJECT_THREAD_NAME ||
    thread?.name === SELF_PROJECT_THREAD_NAME
  );
}

async function ensureSelfProjectThread(
  appServerClient,
  normalizeThread,
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

module.exports = {
  SELF_PROJECT_THREAD_NAME,
  ensureSelfProjectThread,
  isSelfProjectThread,
  sendSelfCommandToThread,
};
