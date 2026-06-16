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

function readObject(value) {
  return value && typeof value === "object" && !Array.isArray(value) ? value : {};
}

module.exports = {
  withRealtimeConversationFeature,
};
