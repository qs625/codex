const test = require("node:test");
const assert = require("node:assert/strict");

const {
  CHAT_COMPAT_CWD_BASENAME,
  buildChatCompatCwd,
  buildCreateThreadStartParams,
  buildThreadListParams,
  buildSubscribeThreadResumeParams,
  withRealtimeConversationFeature,
} = require("./threadConfig.cjs");

test("withRealtimeConversationFeature enables realtime conversation by default", () => {
  assert.deepEqual(withRealtimeConversationFeature(), {
    config: {
      features: {
        goals: true,
        realtime_conversation: true,
      },
      realtime: {
        version: "v2",
        type: "transcription",
        transport: "webrtc",
      },
    },
  });
});

test("withRealtimeConversationFeature preserves existing config entries", () => {
  assert.deepEqual(
    withRealtimeConversationFeature({
      cwd: "/workspace/root",
      config: {
        model: "gpt-5",
        features: {
          plugins: true,
        },
        realtime: {
          voice: "cedar",
        },
      },
    }),
    {
      cwd: "/workspace/root",
      config: {
        model: "gpt-5",
        features: {
          goals: true,
          plugins: true,
          realtime_conversation: true,
        },
        realtime: {
          voice: "cedar",
          version: "v2",
          type: "transcription",
          transport: "webrtc",
        },
      },
    },
  );
});

test("withRealtimeConversationFeature replaces non-object features values", () => {
  assert.deepEqual(
    withRealtimeConversationFeature({
      config: {
        features: "invalid",
      },
    }),
    {
      config: {
        features: {
          goals: true,
          realtime_conversation: true,
        },
        realtime: {
          version: "v2",
          type: "transcription",
          transport: "webrtc",
        },
      },
    },
  );
});

test("withRealtimeConversationFeature replaces non-object realtime values", () => {
  assert.deepEqual(
    withRealtimeConversationFeature({
      config: {
        realtime: "invalid",
      },
    }),
    {
      config: {
        features: {
          goals: true,
          realtime_conversation: true,
        },
        realtime: {
          version: "v2",
          type: "transcription",
          transport: "webrtc",
        },
      },
    },
  );
});

test("buildChatCompatCwd uses a stable recognizable basename", () => {
  assert.equal(
    buildChatCompatCwd("/tmp/root-worker"),
    `/tmp/root-worker/${CHAT_COMPAT_CWD_BASENAME}`,
  );
});

test("buildCreateThreadStartParams creates read-only chat thread params", () => {
  const params = buildCreateThreadStartParams(
    {
      threadMode: "chat",
      cwd: "/work/project",
      name: "Ignored by thread/start",
      taskName: "Ask about tools",
      threadProvider: "native",
      model: "gpt-5",
      serviceTier: null,
    },
    { chatCompatCwd: "/tmp/root-worker/.my-codex-root-worker-chat-cwd" },
  );

  assert.equal(params.cwd, "/tmp/root-worker/.my-codex-root-worker-chat-cwd");
  assert.equal(params.permissions, ":read-only");
  assert.equal(params.sandbox, undefined);
  assert.equal(params.approvalPolicy, "never");
  assert.equal(params.taskName, "Ask about tools");
  assert.equal(params.threadProvider, "native");
  assert.equal(params.model, "gpt-5");
  assert.equal(params.serviceTier, null);
  assert.equal(params.config.features.realtime_conversation, true);
});

test("buildCreateThreadStartParams preserves project thread sandbox behavior", () => {
  const params = buildCreateThreadStartParams({
    threadMode: "project",
    cwd: " /work/project ",
    taskName: "owner_dev",
  });

  assert.equal(params.cwd, "/work/project");
  assert.equal(params.sandbox, "danger-full-access");
  assert.equal(params.permissions, undefined);
  assert.equal(params.approvalPolicy, "never");
  assert.equal(params.taskName, "owner_dev");
});

test("buildThreadListParams requests all providers for root-worker navigation", () => {
  assert.deepEqual(buildThreadListParams(), {
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
  });
});

test("buildSubscribeThreadResumeParams resumes with realtime conversation config", () => {
  assert.deepEqual(buildSubscribeThreadResumeParams("thread-1"), {
    threadId: "thread-1",
    excludeTurns: true,
    config: {
      features: {
        goals: true,
        realtime_conversation: true,
      },
      realtime: {
        version: "v2",
        type: "transcription",
        transport: "webrtc",
      },
    },
  });
});
