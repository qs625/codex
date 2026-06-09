function buildTurnStartParams(payload, input) {
  return {
    threadId: payload.threadId,
    input,
    model: payload.model ?? undefined,
    modelProvider: payload.modelProvider ?? undefined,
    effort: payload.effort ?? undefined,
  };
}

function mergeRuntimeOverride(previousRuntime, payload) {
  return {
    model: payload.model ?? previousRuntime?.model ?? null,
    modelProvider: payload.modelProvider ?? previousRuntime?.modelProvider ?? null,
    reasoningEffort: payload.effort ?? previousRuntime?.reasoningEffort ?? null,
  };
}

function resolveRuntimeForResume(existingRuntime, resume) {
  if (existingRuntime?.localOverride) {
    return existingRuntime;
  }
  return {
    model: resume.model ?? null,
    modelProvider: resume.modelProvider ?? null,
    reasoningEffort: resume.reasoningEffort ?? null,
  };
}

module.exports = {
  buildTurnStartParams,
  mergeRuntimeOverride,
  resolveRuntimeForResume,
};
