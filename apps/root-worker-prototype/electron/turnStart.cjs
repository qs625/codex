function buildTurnStartParams(payload, input) {
  return {
    threadId: payload.threadId,
    input,
    model: payload.model ?? undefined,
    effort: payload.effort ?? undefined,
  };
}

function mergeRuntimeOverride(previousRuntime, payload) {
  return {
    model: payload.model ?? previousRuntime?.model ?? null,
    reasoningEffort: payload.effort ?? previousRuntime?.reasoningEffort ?? null,
  };
}

function resolveRuntimeForResume(existingRuntime, resume) {
  if (existingRuntime?.localOverride) {
    return existingRuntime;
  }
  return {
    model: resume.model ?? null,
    reasoningEffort: resume.reasoningEffort ?? null,
  };
}

module.exports = {
  buildTurnStartParams,
  mergeRuntimeOverride,
  resolveRuntimeForResume,
};
