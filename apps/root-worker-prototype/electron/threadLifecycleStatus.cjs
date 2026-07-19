function normalizeThreadLifecycleStatus(status) {
  if (status && typeof status === "object" && typeof status.type === "string") {
    if (status.type === "idle") {
      if (status.reason === "waitChild") {
        return { type: "waiting", reason: "child" };
      }
      if (status.reason === "waitCommand") {
        return { type: "waiting", reason: "command" };
      }
      if (status.reason === "waitEventSubscription") {
        return { type: "waiting", reason: "eventSubscription" };
      }
      return { type: "final", result: { type: "completed" } };
    }
    if (status.type === "complete") {
      return { type: "final", result: { type: "completed" } };
    }
    if (status.type === "systemError") {
      return { type: "systemError", message: status.message ?? null };
    }
    if (status.type === "active") {
      return {
        type: "active",
        activeFlags: Array.isArray(status.activeFlags)
          ? status.activeFlags.filter((flag) => typeof flag === "string")
          : [],
      };
    }
    if (status.type === "waiting") {
      return {
        type: "waiting",
        reason:
          typeof status.reason === "string" ? status.reason : "eventSubscription",
      };
    }
    if (status.type === "final") {
      const result =
        status.result && typeof status.result === "object"
          ? normalizeThreadLifecycleFinalStatus(status.result)
          : { type: "completed" };
      return { type: "final", result };
    }
    if (status.type === "initializing" || status.type === "notLoaded") {
      return { type: status.type };
    }
    return { type: "systemError", message: null };
  }

  if (typeof status === "string") {
    return status === "idle" || status === "complete"
      ? { type: "final", result: { type: "completed" } }
      : { type: status };
  }

  return { type: "notLoaded" };
}

function normalizeThreadLifecycleFinalStatus(result) {
  if (result.type === "completed") {
    return {
      type: "completed",
      ...(typeof result.lastAgentMessage === "string"
        ? { lastAgentMessage: result.lastAgentMessage }
        : {}),
    };
  }
  if (result.type === "errored") {
    return {
      type: "errored",
      ...(typeof result.message === "string" ? { message: result.message } : {}),
    };
  }
  if (result.type === "interrupted" || result.type === "shutdown") {
    return { type: result.type };
  }
  return { type: "completed" };
}

module.exports = {
  normalizeThreadLifecycleStatus,
};
