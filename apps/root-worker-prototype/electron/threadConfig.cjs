function withRealtimeConversationFeature(params = {}) {
  return {
    ...params,
    config: {
      ...(params.config ?? {}),
      features: {
        ...readObject(params.config?.features),
        realtime_conversation: true,
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
