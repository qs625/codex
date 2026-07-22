const path = require("node:path");

const CHAT_COMPAT_CWD_BASENAME = ".my-codex-root-worker-chat-cwd";

function withRealtimeConversationFeature(params = {}) {
  return {
    ...params,
    config: {
      ...(params.config ?? {}),
      features: {
        ...readObject(params.config?.features),
        goals: true,
        realtime_conversation: true,
      },
      realtime: {
        ...readObject(params.config?.realtime),
        version: "v2",
        type: "transcription",
        transport: "webrtc",
      },
    },
  };
}

function buildChatCompatCwd(basePath) {
  if (typeof basePath !== "string" || !basePath.trim()) {
    throw new Error("chat compat cwd needs a stable base path");
  }
  return path.join(basePath, CHAT_COMPAT_CWD_BASENAME);
}

function buildCreateThreadStartParams(payload = {}, options = {}) {
  const payloadObject = readObject(payload);
  const rawCwd =
    typeof payloadObject.cwd === "string" && payloadObject.cwd.trim()
      ? payloadObject.cwd.trim()
      : undefined;
  const isChatThread = payloadObject.threadMode === "chat";
  const chatCompatCwd =
    typeof options.chatCompatCwd === "string" && options.chatCompatCwd.trim()
      ? options.chatCompatCwd.trim()
      : undefined;
  const cwd = isChatThread ? chatCompatCwd : rawCwd;

  if (isChatThread && !cwd) {
    throw new Error("chat thread needs a compat cwd");
  }

  return withRealtimeConversationFeature({
    ...(cwd ? { cwd } : {}),
    taskName: payloadObject.taskName || undefined,
    agentType: payloadObject.agentType || undefined,
    model: payloadObject.model || undefined,
    modelProvider: payloadObject.modelProvider || undefined,
    reasoningEffort: payloadObject.reasoningEffort || undefined,
    serviceTier:
      Object.prototype.hasOwnProperty.call(payloadObject, "serviceTier")
        ? payloadObject.serviceTier
        : undefined,
    approvalPolicy: "never",
    ...(isChatThread
      ? { permissions: ":read-only" }
      : { sandbox: "danger-full-access" }),
    threadSource: "user",
  });
}

function buildThreadListParams() {
  return {
    limit: 200,
    modelProviders: [],
    sourceKinds: [
      "appServer",
      "cli",
      "vscode",
      "exec",
      "subAgent",
      "subAgentReview",
      "subAgentCompact",
      "subAgentThreadSpawn",
      "subAgentOther",
      "unknown",
    ],
  };
}

function buildSubscribeThreadResumeParams(threadId) {
  return withRealtimeConversationFeature({ threadId, excludeTurns: true });
}

function readObject(value) {
  return value && typeof value === "object" && !Array.isArray(value) ? value : {};
}

module.exports = {
  CHAT_COMPAT_CWD_BASENAME,
  buildChatCompatCwd,
  buildCreateThreadStartParams,
  buildThreadListParams,
  buildSubscribeThreadResumeParams,
  withRealtimeConversationFeature,
};
