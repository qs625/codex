const MAX_TERMINAL_REPLAY_BYTES = 1024 * 1024;

function createTerminalPanelState(commandOutputCache = new Map()) {
  return {
    activeTabId: null,
    tabs: [],
    detachedSessionKeys: new Set(),
    commandOutputCache,
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

function terminalTabNeedsLiveSessionRefresh(tab) {
  return (
    tab &&
    tab.origin === "model" &&
    tab.readOnlyOutput === true &&
    (tab.status === "running" || tab.status === "starting")
  );
}

function replayBufferFromDescriptor(descriptor) {
  const replay = descriptor.replayBase64
    ? Buffer.from(descriptor.replayBase64, "base64")
    : Buffer.alloc(0);
  if (!descriptor.replayTruncated) {
    return replay;
  }
  return terminalSafeTruncatedReplay(replay);
}

function terminalSafeTruncatedReplay(replay) {
  if (!Buffer.isBuffer(replay) || replay.length === 0) {
    return Buffer.alloc(0);
  }
  const cr = replay.indexOf(0x0d);
  const lf = replay.indexOf(0x0a);
  const boundary =
    cr === -1 ? lf : lf === -1 ? cr : Math.min(cr, lf);
  if (boundary === -1) {
    return Buffer.alloc(0);
  }
  let start = boundary + 1;
  if (replay[boundary] === 0x0d && replay[start] === 0x0a) {
    start += 1;
  }
  return replay.subarray(start);
}

function mergeTerminalSessions(state, sessions, threadId = null) {
  const activeKeys = new Set();
  const tabIndex = indexTerminalTabsForSessionMerge(state.tabs);
  for (const descriptor of sessions) {
    const key = terminalSessionKey(descriptor);
    activeKeys.add(key);
    if (isTerminalSessionDetached(state, descriptor)) {
      continue;
    }
    const existing = findTerminalSessionMergeTarget(tabIndex, descriptor);
    if (existing) {
      removeTerminalTabFromSessionMergeIndex(tabIndex, existing);
      mergeFocusedCommandDescriptor(existing, descriptor);
      existing.status = "running";
      delete existing.readOnlyOutput;
      addTerminalTabToSessionMergeIndex(tabIndex, existing);
      continue;
    }
    const tab = createTerminalTab(descriptor, {
      status: "running",
      lastSequence: normalizeSequence(descriptor.replayThroughSequence),
    });
    state.tabs.push(tab);
    addTerminalTabToSessionMergeIndex(tabIndex, tab, state.tabs.length - 1);
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

function indexTerminalTabsForSessionMerge(tabs) {
  const index = {
    bySessionKey: new Map(),
    readOnlyModelByCommandKey: new Map(),
    startingUserByProcessId: new Map(),
    orderByTab: new Map(),
  };
  for (const [order, tab] of tabs.entries()) {
    addTerminalTabToSessionMergeIndex(index, tab, order);
  }
  return index;
}

function addTerminalTabToSessionMergeIndex(
  index,
  tab,
  order = index.orderByTab.get(tab),
) {
  const tabOrder = Number.isInteger(order) ? order : index.orderByTab.size;
  index.orderByTab.set(tab, tabOrder);
  setTerminalTabIndexValue(
    index.bySessionKey,
    terminalSessionKey(tab),
    tab,
    tabOrder,
  );
  if (tab.origin === "model" && tab.readOnlyOutput === true) {
    const commandKey = terminalCommandLookupKey(tab.threadId, tab.commandItemId);
    if (commandKey) {
      setTerminalTabIndexValue(
        index.readOnlyModelByCommandKey,
        commandKey,
        tab,
        tabOrder,
      );
    }
  }
  if (tab.origin === "user" && tab.status === "starting") {
    setTerminalTabIndexValue(
      index.startingUserByProcessId,
      tab.processId,
      tab,
      tabOrder,
    );
  }
}

function removeTerminalTabFromSessionMergeIndex(index, tab) {
  deleteTerminalTabIndexValue(index.bySessionKey, terminalSessionKey(tab), tab);
  const commandKey = terminalCommandLookupKey(tab.threadId, tab.commandItemId);
  if (commandKey) {
    deleteTerminalTabIndexValue(index.readOnlyModelByCommandKey, commandKey, tab);
  }
  deleteTerminalTabIndexValue(index.startingUserByProcessId, tab.processId, tab);
}

function setTerminalTabIndexValue(index, key, tab, order) {
  const existing = key ? index.get(key) : null;
  if (key && (!existing || order < existing.order)) {
    index.set(key, { tab, order });
  }
}

function deleteTerminalTabIndexValue(index, key, tab) {
  if (key && index.get(key)?.tab === tab) {
    index.delete(key);
  }
}

function findTerminalSessionMergeTarget(index, descriptor) {
  let match = index.bySessionKey.get(terminalSessionKey(descriptor)) ?? null;
  if (descriptor.origin === "model") {
    const commandMatch = index.readOnlyModelByCommandKey.get(
      terminalCommandLookupKey(descriptor.threadId, descriptor.commandItemId),
    );
    if (commandMatch) {
      match = earlierTerminalTabIndexEntry(match, commandMatch);
    }
  }
  if (descriptor.origin === "user") {
    match = earlierTerminalTabIndexEntry(
      match,
      index.startingUserByProcessId.get(descriptor.processId) ?? null,
    );
  }
  return match?.tab ?? null;
}

function earlierTerminalTabIndexEntry(left, right) {
  if (!left) {
    return right;
  }
  if (!right) {
    return left;
  }
  return right.order < left.order ? right : left;
}

function reattachTerminalSessions(state) {
  const detachedCount = state.detachedSessionKeys.size;
  state.detachedSessionKeys.clear();
  return detachedCount;
}

function addUserTerminal(state, descriptor) {
  const tab = createTerminalTab(descriptor, {
    status: "starting",
    lastSequence: null,
  });
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
    mergeFocusedCommandDescriptor(existing, descriptor);
    return existing;
  }
  const tab = createTerminalTab(descriptor, {
    status: "running",
    lastSequence: normalizeSequence(descriptor.replayThroughSequence),
    readOnlyOutput: true,
  });
  state.tabs.push(tab);
  state.activeTabId = tab.id;
  return tab;
}

function createTerminalTab(
  descriptor,
  { status, lastSequence, readOnlyOutput = false },
) {
  return {
    ...normalizeDescriptor(descriptor),
    id: descriptor.sessionId,
    status,
    replay: replayBufferFromDescriptor(descriptor),
    lastSequence,
    hasSequenceGap: false,
    backgroundActivity: false,
    ...(readOnlyOutput ? { readOnlyOutput: true } : {}),
  };
}

function mergeFocusedCommandDescriptor(tab, descriptor) {
  const normalized = normalizeDescriptor(descriptor);
  const currentSequence = normalizeSequence(tab.lastSequence);
  const descriptorSequence = normalizeSequence(descriptor.replayThroughSequence);
  Object.assign(tab, normalized, {
    canResize: tab.canResize === true || normalized.canResize === true,
    canWrite: tab.canWrite === true || normalized.canWrite === true,
    canTerminate: tab.canTerminate === true || normalized.canTerminate === true,
  });
  if (descriptor.replayBase64 && descriptorSequence >= currentSequence) {
    tab.replay = replayBufferFromDescriptor(descriptor);
    tab.lastSequence = descriptorSequence;
    tab.hasSequenceGap = false;
  }
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
    size: null,
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
  if (options.liveSessionRefreshed === true && liveSession) {
    return commandFocusDescriptorFromLiveSession(command, liveSession);
  }
  if (activeCommand) {
    return commandFocusDescriptor(command, activeCommand);
  }
  if (options.liveSessionRefreshed !== true) {
    return commandFocusDescriptorFromRequest(command);
  }
  return commandFocusDescriptorFromRequest(command);
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
    size: null,
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
    size: normalizeTerminalSizeDescriptor(session.size),
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
  recordDetachedTerminalSession(state, state.tabs[index]);
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

function setTerminalTabSize(tab, size) {
  const normalized = normalizeTerminalSizeDescriptor(size);
  if (!normalized) {
    return false;
  }
  tab.size = normalized;
  return true;
}

function commandOutputCacheKey(threadId, commandItemId) {
  if (
    typeof threadId !== "string" ||
    threadId.length === 0 ||
    typeof commandItemId !== "string" ||
    commandItemId.length === 0
  ) {
    return null;
  }
  return `${threadId}\0${commandItemId}`;
}

function appendCommandOutputCache(state, threadId, commandItemId, deltaBase64, sequence) {
  const key = commandOutputCacheKey(threadId, commandItemId);
  if (!key || !deltaBase64) {
    return null;
  }
  let entry = state.commandOutputCache.get(key);
  if (!entry) {
    entry = {
      replay: Buffer.alloc(0),
      firstSequence: null,
      lastSequence: null,
      replayTruncated: false,
      hasSequenceGap: false,
    };
    state.commandOutputCache.set(key, entry);
  }
  if (
    Number.isInteger(sequence) &&
    !Number.isInteger(entry.firstSequence)
  ) {
    entry.firstSequence = sequence;
  }
  if (!appendTerminalOutput(entry, deltaBase64, sequence)) {
    return entry;
  }
  return entry;
}

function applyCommandOutputCacheToDescriptor(state, descriptor) {
  const key = commandOutputCacheKey(descriptor.threadId, descriptor.commandItemId);
  if (!key) {
    return descriptor;
  }
  const entry = state.commandOutputCache.get(key);
  if (!entry || entry.replay.length === 0) {
    return descriptor;
  }
  const descriptorSequence = normalizeSequence(descriptor.replayThroughSequence);
  const cacheSequence = normalizeSequence(entry.lastSequence);
  if (descriptor.replayBase64 && descriptorSequence >= cacheSequence) {
    return descriptor;
  }
  if (descriptor.replayBase64) {
    const firstCacheSequence = Number.isInteger(entry.firstSequence)
      ? entry.firstSequence
      : null;
    if (firstCacheSequence === descriptorSequence + 1) {
      return {
        ...descriptor,
        replayBase64: Buffer.concat([
          Buffer.from(descriptor.replayBase64, "base64"),
          entry.replay,
        ]).toString("base64"),
        replayTruncated:
          Boolean(descriptor.replayTruncated) || Boolean(entry.replayTruncated),
        replayThroughSequence: cacheSequence,
      };
    }
    if (firstCacheSequence !== 1) {
      return descriptor;
    }
  }
  return {
    ...descriptor,
    replayBase64: entry.replay.toString("base64"),
    replayTruncated:
      Boolean(descriptor.replayTruncated) ||
      Boolean(entry.replayTruncated) ||
      (Number.isInteger(entry.firstSequence) && entry.firstSequence > 1),
    replayThroughSequence: cacheSequence,
  };
}

function deleteCommandOutputCache(state, threadId, commandItemId) {
  const key = commandOutputCacheKey(threadId, commandItemId);
  if (!key) {
    return false;
  }
  return state.commandOutputCache.delete(key);
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

function terminalCommandItemKey(value) {
  if (
    value.origin !== "model" ||
    !value.threadId ||
    !value.commandItemId
  ) {
    return null;
  }
  return terminalCommandLookupKey(value.threadId, value.commandItemId);
}

function terminalCommandLookupKey(threadId, commandItemId) {
  if (
    typeof threadId !== "string" ||
    threadId.length === 0 ||
    typeof commandItemId !== "string" ||
    commandItemId.length === 0
  ) {
    return null;
  }
  return ["model-command", threadId, commandItemId].join(":");
}

function recordDetachedTerminalSession(state, value) {
  state.detachedSessionKeys.add(terminalSessionKey(value));
  const commandItemKey = terminalCommandItemKey(value);
  if (commandItemKey) {
    state.detachedSessionKeys.add(commandItemKey);
  }
}

function isTerminalSessionDetached(state, value) {
  if (state.detachedSessionKeys.has(terminalSessionKey(value))) {
    return true;
  }
  const commandItemKey = terminalCommandItemKey(value);
  return commandItemKey ? state.detachedSessionKeys.has(commandItemKey) : false;
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
    size: normalizeTerminalSizeDescriptor(descriptor.size),
    canResize: descriptor.canResize !== false,
    canWrite: descriptor.canWrite !== false,
    canTerminate: descriptor.canTerminate !== false,
    exitCode: null,
  };
}

function normalizeSequence(value) {
  return Number.isSafeInteger(value) && value >= 0 ? value : 0;
}

function normalizeTerminalSizeDescriptor(size) {
  if (!size || typeof size !== "object") {
    return null;
  }
  const rows = Math.round(Number(size.rows));
  const cols = Math.round(Number(size.cols));
  if (
    !Number.isSafeInteger(rows) ||
    !Number.isSafeInteger(cols) ||
    rows <= 0 ||
    cols <= 0
  ) {
    return null;
  }
  return { rows, cols };
}

module.exports = {
  MAX_TERMINAL_REPLAY_BYTES,
  activeCommandForTerminalFocus,
  addUserTerminal,
  commandFocusDescriptor,
  commandFocusDescriptorForTerminalFocus,
  commandFocusDescriptorFromRequest,
  commandFocusDescriptorFromLiveSession,
  appendCommandOutputCache,
  applyCommandOutputCacheToDescriptor,
  deleteCommandOutputCache,
  focusCommandTerminal,
  appendTerminalOutput,
  closeTerminalTab,
  createTerminalPanelState,
  markRunningTerminalsLost,
  markTerminalExited,
  mergeTerminalSessions,
  liveCommandSessionForTerminalFocus,
  reattachTerminalSessions,
  replayBufferFromDescriptor,
  setTerminalTabSize,
  isTerminalSessionDetached,
  isRunningCommandStatus,
  selectTerminalTab,
  terminalPanelSnapshot,
  terminalSafeTruncatedReplay,
  terminalTabMetadata,
  terminalTabNeedsLiveSessionRefresh,
  terminalTabSupports,
  terminalCommandItemKey,
  terminalSessionKey,
};
