const test = require("node:test");
const assert = require("node:assert/strict");

const { withRealtimeConversationFeature } = require("./threadConfig.cjs");

test("withRealtimeConversationFeature enables realtime conversation by default", () => {
  assert.deepEqual(withRealtimeConversationFeature(), {
    config: {
      features: {
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
