const MAX_TERMINAL_REPLAY_BYTES = 1024 * 1024;

function createTerminalPanelState() {
  return {
    activeTabId: null,
    tabs: [],
    detachedSessionKeys: new Set(),
  };
}

function terminalPanelSnapshot(state) {
  return {
    activeTabId: state.activeTabId,
    tabs: state.tabs.map(terminalTabSnapshot),
    detachedCount: state.detachedSessionKeys.size,
  };
}

function terminalTabSnapshot(tab) {
  const { replay, ...metadata } = tab;
  return {
    ...metadata,
    replayBase64: replay.toString("base64"),
  };
}

function terminalTabMetadata(tab) {
  const { replay, ...metadata } = tab;
  return metadata;
}

function terminalTabSupports(tab, control) {
  switch (control) {
    case "write":
      return tab.canWrite === true;
    case "resize":
      return tab.canResize === true;
    case "terminate":
      return tab.canTerminate === true;
    default:
      return false;
  }
}

function mergeTerminalSessions(state, sessions, threadId = null) {
  const activeKeys = new Set();
  for (const descriptor of sessions) {
    const key = terminalSessionKey(descriptor);
    activeKeys.add(key);
    if (state.detachedSessionKeys.has(key)) {
      continue;
    }
    const existing = state.tabs.find(
      (tab) =>
        terminalSessionKey(tab) === key ||
        (tab.origin === "user" &&
          tab.status === "starting" &&
          tab.processId === descriptor.processId),
    );
    if (existing) {
      const descriptorSequence = normalizeSequence(
        descriptor.replayThroughSequence,
      );
      const shouldApplyReplay =
        descriptor.replayBase64 &&
        descriptorSequence >= (existing.lastSequence ?? 0);
      Object.assign(existing, normalizeDescriptor(descriptor), { status: "running" });
      delete existing.readOnlyOutput;
      if (shouldApplyReplay) {
        existing.replay = Buffer.from(descriptor.replayBase64, "base64");
        existing.lastSequence = descriptorSequence;
        existing.hasSequenceGap = false;
      }
      continue;
    }
    state.tabs.push({
      ...normalizeDescriptor(descriptor),
      id: descriptor.sessionId,
      status: "running",
      replay: descriptor.replayBase64
        ? Buffer.from(descriptor.replayBase64, "base64")
        : Buffer.alloc(0),
      lastSequence: normalizeSequence(descriptor.replayThroughSequence),
      hasSequenceGap: false,
      backgroundActivity: false,
    });
  }
  for (const tab of state.tabs) {
    const wasInListedScope =
      tab.origin === "user" ||
      (!tab.readOnlyOutput && threadId && tab.threadId === threadId);
    if (
      wasInListedScope &&
      tab.status === "running" &&
      !activeKeys.has(terminalSessionKey(tab))
    ) {
      tab.status = "lost";
    }
  }
  if (!state.activeTabId || !state.tabs.some((tab) => tab.id === state.activeTabId)) {
    state.activeTabId = state.tabs[0]?.id ?? null;
  }
  return state;
}

function reattachTerminalSessions(state) {
  const detachedCount = state.detachedSessionKeys.size;
  state.detachedSessionKeys.clear();
  return detachedCount;
}

function addUserTerminal(state, descriptor) {
  const tab = {
    ...normalizeDescriptor(descriptor),
    id: descriptor.sessionId,
    status: "starting",
    replay: Buffer.alloc(0),
    lastSequence: null,
    hasSequenceGap: false,
    backgroundActivity: false,
  };
  state.tabs.push(tab);
  state.activeTabId = tab.id;
  return tab;
}

function focusCommandTerminal(state, descriptor) {
  const existing = state.tabs.find(
    (tab) =>
      tab.origin === "model" &&
      tab.threadId === descriptor.threadId &&
      ((tab.commandItemId === descriptor.commandItemId &&
        (tab.readOnlyOutput === true || isLivePtyTerminalTab(tab))) ||
        (tab.commandItemId == null &&
          isLivePtyTerminalTab(tab) &&
          tab.processId === descriptor.processId)),
  );
  if (existing) {
    state.activeTabId = existing.id;
    return existing;
  }
  const tab = {
    ...normalizeDescriptor(descriptor),
    id: descriptor.sessionId,
    status: "running",
    replay: descriptor.replayBase64
      ? Buffer.from(descriptor.replayBase64, "base64")
      : Buffer.alloc(0),
    lastSequence: normalizeSequence(descriptor.replayThroughSequence),
    hasSequenceGap: false,
    backgroundActivity: false,
    readOnlyOutput: true,
  };
  state.tabs.push(tab);
  state.activeTabId = tab.id;
  return tab;
}

function isLivePtyTerminalTab(tab) {
  return (
    tab.readOnlyOutput !== true &&
    (tab.status === "running" || tab.status === "starting")
  );
}

function activeCommandForTerminalFocus(thread, command) {
  if (!thread || !Array.isArray(thread.activeCommandItems)) {
    return null;
  }
  return (
    thread.activeCommandItems.find(
      (item) =>
        item &&
        item.type === "commandExecution" &&
        item.id === command.commandItemId &&
        isRunningCommandStatus(item.status),
    ) ?? null
  );
}

function liveCommandSessionForTerminalFocus(state, command) {
  if (!state || !Array.isArray(state.tabs)) {
    return null;
  }
  const requestedProcessId =
    typeof command.processId === "string" && command.processId.length > 0
      ? command.processId
      : null;
  return (
    state.tabs.find(
      (tab) =>
        tab &&
        tab.origin === "model" &&
        tab.threadId === command.threadId &&
        isLivePtyTerminalTab(tab) &&
        (tab.commandItemId === command.commandItemId ||
          (tab.commandItemId == null &&
            requestedProcessId !== null &&
            tab.processId === requestedProcessId)),
    ) ?? null
  );
}

function commandFocusDescriptor(command, activeCommand) {
  return {
    sessionId: `command:${command.threadId}:${command.commandItemId}`,
    generation: command.commandItemId,
    origin: "model",
    threadId: command.threadId,
    commandItemId: command.commandItemId,
    processId: activeCommand.processId || command.processId || activeCommand.id,
    title: activeCommand.command,
    cwd: activeCommand.cwd,
    replayBase64: activeCommand.aggregatedOutput
      ? Buffer.from(activeCommand.aggregatedOutput).toString("base64")
      : null,
    replayTruncated: false,
    replayThroughSequence: 0,
    canResize: false,
    canWrite: false,
    canTerminate: false,
  };
}

function commandFocusDescriptorForTerminalFocus(
  command,
  activeCommand,
  liveSession,
  options = {},
) {
  if (activeCommand) {
    return commandFocusDescriptor(command, activeCommand);
  }
  if (options.liveSessionRefreshed !== true || !liveSession) {
    return commandFocusDescriptorFromRequest(command);
  }
  return commandFocusDescriptorFromLiveSession(command, liveSession);
}

function commandFocusDescriptorFromRequest(command) {
  if (
    typeof command.command !== "string" ||
    command.command.length === 0 ||
    typeof command.cwd !== "string" ||
    typeof command.status !== "string" ||
    !isRunningCommandStatus(command.status)
  ) {
    return null;
  }
  return {
    sessionId: `command:${command.threadId}:${command.commandItemId}`,
    generation: command.commandItemId,
    origin: "model",
    threadId: command.threadId,
    commandItemId: command.commandItemId,
    processId: command.processId || command.commandItemId,
    title: command.command,
    cwd: command.cwd,
    replayBase64: null,
    replayTruncated: false,
    replayThroughSequence: 0,
    canResize: false,
    canWrite: false,
    canTerminate: false,
  };
}

function commandFocusDescriptorFromLiveSession(command, session) {
  return {
    sessionId: session.sessionId || session.id,
    generation: session.generation || command.commandItemId,
    origin: "model",
    threadId: command.threadId,
    commandItemId: command.commandItemId,
    processId: session.processId || command.processId || command.commandItemId,
    title: session.title || command.commandItemId,
    cwd: session.cwd || "",
    replayBase64: Buffer.isBuffer(session.replay)
      ? session.replay.toString("base64")
      : null,
    replayTruncated: Boolean(session.replayTruncated),
    replayThroughSequence: session.lastSequence ?? session.replayThroughSequence ?? 0,
    canResize: session.canResize !== false,
    canWrite: session.canWrite !== false,
    canTerminate: session.canTerminate !== false,
  };
}

function selectTerminalTab(state, tabId) {
  const tab = state.tabs.find((candidate) => candidate.id === tabId);
  if (!tab) {
    return false;
  }
  state.activeTabId = tab.id;
  tab.backgroundActivity = false;
  return true;
}

function closeTerminalTab(state, tabId) {
  const index = state.tabs.findIndex((tab) => tab.id === tabId);
  if (index === -1) {
    return false;
  }
  const wasActive = state.activeTabId === tabId;
  state.detachedSessionKeys.add(terminalSessionKey(state.tabs[index]));
  state.tabs.splice(index, 1);
  if (wasActive) {
    state.activeTabId =
      state.tabs[Math.min(index, state.tabs.length - 1)]?.id ?? null;
  }
  return true;
}

function appendTerminalOutput(tab, deltaBase64, sequence) {
  if (Number.isInteger(sequence)) {
    if (Number.isInteger(tab.lastSequence) && sequence <= tab.lastSequence) {
      return false;
    }
    if (Number.isInteger(tab.lastSequence) && sequence > tab.lastSequence + 1) {
      tab.hasSequenceGap = true;
      tab.replayTruncated = true;
    }
    tab.lastSequence = sequence;
  }
  const delta = Buffer.from(deltaBase64, "base64");
  if (delta.length === 0) {
    return false;
  }
  tab.replay = Buffer.concat([tab.replay, delta]);
  if (tab.replay.length > MAX_TERMINAL_REPLAY_BYTES) {
    tab.replay = tab.replay.subarray(tab.replay.length - MAX_TERMINAL_REPLAY_BYTES);
    tab.replayTruncated = true;
  }
  if (tab.status === "starting") {
    tab.status = "running";
  }
  tab.backgroundActivity = true;
  return true;
}

function markTerminalExited(tab, exitCode) {
  tab.status = "exited";
  tab.exitCode = Number.isInteger(exitCode) ? exitCode : null;
}

function markRunningTerminalsLost(state) {
  for (const tab of state.tabs) {
    if (tab.status === "running" || tab.status === "starting") {
      tab.status = "lost";
    }
  }
}

function terminalSessionKey(value) {
  return [
    value.origin,
    value.threadId ?? "",
    value.processId,
    value.generation,
  ].join(":");
}

function isTerminalSessionDetached(state, value) {
  return state.detachedSessionKeys.has(terminalSessionKey(value));
}

function isRunningCommandStatus(status) {
  const normalized = String(status || "")
    .trim()
    .toLowerCase()
    .replace(/[_-]/g, "");
  return normalized === "running" || normalized === "inprogress";
}

function normalizeDescriptor(descriptor) {
  return {
    sessionId: descriptor.sessionId,
    generation: descriptor.generation,
    origin: descriptor.origin,
    threadId: descriptor.threadId ?? null,
    commandItemId: descriptor.commandItemId ?? null,
    processId: descriptor.processId,
    title: descriptor.title || "Terminal",
    cwd: descriptor.cwd || "",
    replayTruncated: Boolean(descriptor.replayTruncated),
    replayThroughSequence: normalizeSequence(
      descriptor.replayThroughSequence,
    ),
    canResize: descriptor.canResize !== false,
    canWrite: descriptor.canWrite !== false,
    canTerminate: descriptor.canTerminate !== false,
    exitCode: null,
  };
}

function normalizeSequence(value) {
  return Number.isSafeInteger(value) && value >= 0 ? value : 0;
}

module.exports = {
  MAX_TERMINAL_REPLAY_BYTES,
  activeCommandForTerminalFocus,
  addUserTerminal,
  commandFocusDescriptor,
  commandFocusDescriptorForTerminalFocus,
  commandFocusDescriptorFromRequest,
  commandFocusDescriptorFromLiveSession,
  focusCommandTerminal,
  appendTerminalOutput,
  closeTerminalTab,
  createTerminalPanelState,
  markRunningTerminalsLost,
  markTerminalExited,
  mergeTerminalSessions,
  liveCommandSessionForTerminalFocus,
  reattachTerminalSessions,
  isTerminalSessionDetached,
  isRunningCommandStatus,
  selectTerminalTab,
  terminalPanelSnapshot,
  terminalTabMetadata,
  terminalTabSupports,
  terminalSessionKey,
};
