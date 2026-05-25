function createClientStatus() {
  return {
    phase: "starting",
    detail: "Starting language server",
    initialized: false,
    activeProgressTokens: new Set(),
  };
}

function markClientReady(status) {
  status.initialized = true;
  if (status.activeProgressTokens.size > 0) {
    status.phase = "indexing";
    if (!status.detail) {
      status.detail = "Indexing workspace";
    }
    return;
  }

  status.phase = "ready";
  status.detail = "Ready";
}

function markClientError(status, detail) {
  status.phase = "error";
  status.detail = detail;
}

function applyProgressNotification(status, message) {
  if (message.method !== "$/progress") {
    return;
  }

  const token = message.params?.token;
  const value = message.params?.value;
  if (!token || !value || typeof value.kind !== "string") {
    return;
  }

  if (value.kind === "begin") {
    status.activeProgressTokens.add(token);
    status.phase = status.initialized ? "indexing" : "starting";
    status.detail = value.message ?? value.title ?? "Indexing workspace";
    return;
  }

  if (value.kind === "end") {
    status.activeProgressTokens.delete(token);
    if (status.initialized) {
      status.phase = status.activeProgressTokens.size > 0 ? "indexing" : "ready";
      status.detail =
        status.activeProgressTokens.size > 0 ? status.detail ?? "Indexing workspace" : "Ready";
    }
    return;
  }

  if (value.kind === "report" && typeof value.message === "string" && value.message.trim()) {
    status.detail = value.message;
  }
}

function snapshotClientStatus(status) {
  return {
    phase: status.phase,
    detail: status.detail,
  };
}

module.exports = {
  applyProgressNotification,
  createClientStatus,
  markClientError,
  markClientReady,
  snapshotClientStatus,
};
