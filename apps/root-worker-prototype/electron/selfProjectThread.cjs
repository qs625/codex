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
};
