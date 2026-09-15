const assert = require("node:assert/strict");
const test = require("node:test");

const {
  MAX_TERMINAL_REPLAY_BYTES,
  activeCommandForTerminalFocus,
  addUserTerminal,
  appendCommandOutputCache,
  appendTerminalOutput,
  applyCommandOutputCacheToDescriptor,
  closeTerminalTab,
  commandFocusDescriptor,
  commandFocusDescriptorForTerminalFocus,
  commandFocusDescriptorFromRequest,
  commandFocusDescriptorFromLiveSession,
  createTerminalPanelState,
  deleteCommandOutputCache,
  focusCommandTerminal,
  liveCommandSessionForTerminalFocus,
  markTerminalExited,
  mergeTerminalSessions,
  reattachTerminalSessions,
  replayBufferFromDescriptor,
  setTerminalTabSize,
  terminalSafeTruncatedReplay,
  terminalPanelSnapshot,
  terminalCommandItemKey,
  terminalSessionKey,
  terminalTabNeedsLiveSessionRefresh,
  terminalTabSupports,
} = require("./terminalPanel.cjs");

function descriptor(overrides = {}) {
  return {
    sessionId: "model:thread:call:42",
    generation: "call",
    origin: "model",
    threadId: "thread",
    commandItemId: "call",
    processId: "42",
    title: "shell",
    cwd: "/tmp",
    replayBase64: null,
    replayTruncated: false,
    replayThroughSequence: 0,
    canResize: true,
    canWrite: true,
    canTerminate: true,
    ...overrides,
  };
}

test("merge restores live sessions and marks missing sessions lost", () => {
  const state = createTerminalPanelState();
  mergeTerminalSessions(state, [descriptor()], "thread");
  assert.equal(state.tabs[0].status, "running");
  mergeTerminalSessions(state, [], "thread");
  assert.equal(state.tabs[0].status, "lost");
});

test("merge preserves live terminal size for width-sensitive replay", () => {
  const state = createTerminalPanelState();
  mergeTerminalSessions(
    state,
    [
      descriptor({
        replayBase64: Buffer.from("progress\rnext").toString("base64"),
        size: { rows: 31, cols: 97 },
      }),
    ],
    "thread",
  );

  assert.deepEqual(state.tabs[0].size, { rows: 31, cols: 97 });
  assert.deepEqual(terminalPanelSnapshot(state).tabs[0].size, {
    rows: 31,
    cols: 97,
  });

  mergeTerminalSessions(
    state,
    [
      descriptor({
        sessionId: "model:thread:call:42",
        size: { rows: 33, cols: 101 },
      }),
    ],
    "thread",
  );
  assert.deepEqual(state.tabs[0].size, { rows: 33, cols: 101 });
});

test("terminal resize updates tab size metadata used by future replay", () => {
  const state = createTerminalPanelState();
  mergeTerminalSessions(
    state,
    [descriptor({ size: { rows: 24, cols: 80 } })],
    "thread",
  );
  const tab = state.tabs[0];

  assert.equal(setTerminalTabSize(tab, { rows: 35, cols: 118 }), true);
  assert.deepEqual(tab.size, { rows: 35, cols: 118 });
  assert.deepEqual(terminalPanelSnapshot(state).tabs[0].size, {
    rows: 35,
    cols: 118,
  });
  assert.equal(setTerminalTabSize(tab, { rows: 0, cols: 0 }), false);
  assert.deepEqual(tab.size, { rows: 35, cols: 118 });
});

test("truncated terminal replay starts after a partial carriage-return progress line", () => {
  const replay = Buffer.from(
    "buildkit partial progress that lost its leading cursor state\r#20 DONE 0.2s\n#21 [stage-1 5/6] RUN printf '%s\\n'\n",
  );

  assert.equal(
    terminalSafeTruncatedReplay(replay).toString(),
    "#20 DONE 0.2s\n#21 [stage-1 5/6] RUN printf '%s\\n'\n",
  );
});

test("truncated terminal replay drops a wrapped leading line before later newlines", () => {
  const wrappedTail = Buffer.from(
    "ps/group-manager-bot/fly.toml\n#20 DONE 0.2s\n#21 [stage-1 5/6] RUN printf '%s\\n'\n",
  );
  const tab = focusCommandTerminal(
    createTerminalPanelState(),
    descriptor({
      replayBase64: wrappedTail.toString("base64"),
      replayTruncated: true,
      replayThroughSequence: 12,
    }),
  );

  assert.equal(
    tab.replay.toString(),
    "#20 DONE 0.2s\n#21 [stage-1 5/6] RUN printf '%s\\n'\n",
  );
  assert.equal(tab.replayTruncated, true);
  assert.equal(tab.lastSequence, 12);
});

test("complete terminal replay preserves carriage returns and wrapped lines", () => {
  const completeReplay = Buffer.from(
    "#20 [stage-1 4/6] RUN mkdir -p /app-runtime/apps/group-manager-bot\r#20 DONE 0.2s\n",
  );

  assert.equal(
    replayBufferFromDescriptor({
      replayBase64: completeReplay.toString("base64"),
      replayTruncated: false,
    }).toString(),
    completeReplay.toString(),
  );
});

test("truncated command output cache hydrate starts replay at a terminal boundary", () => {
  const state = createTerminalPanelState();
  appendCommandOutputCache(
    state,
    "thread",
    "call",
    Buffer.from("partial wrapped docker output\n#22 DONE 0.4s\n").toString("base64"),
    7,
  );
  const descriptorWithCache = applyCommandOutputCacheToDescriptor(
    state,
    commandFocusDescriptorFromRequest({
      threadId: "thread",
      commandItemId: "call",
      processId: "42",
      command: "docker build",
      cwd: "/repo",
      status: "running",
    }),
  );

  assert.equal(descriptorWithCache.replayTruncated, true);
  const tab = focusCommandTerminal(state, descriptorWithCache);

  assert.equal(tab.replay.toString(), "#22 DONE 0.4s\n");
  assert.equal(tab.lastSequence, 7);
});

test("output sequence is idempotent and reports gaps", () => {
  const state = createTerminalPanelState();
  mergeTerminalSessions(state, [descriptor()], "thread");
  const tab = state.tabs[0];
  assert.equal(appendTerminalOutput(tab, Buffer.from("a").toString("base64"), 1), true);
  assert.equal(appendTerminalOutput(tab, Buffer.from("a").toString("base64"), 1), false);
  assert.equal(appendTerminalOutput(tab, Buffer.from("c").toString("base64"), 3), true);
  assert.equal(tab.hasSequenceGap, true);
  assert.equal(Buffer.from(terminalPanelSnapshot(state).tabs[0].replayBase64, "base64").toString(), "ac");
});

test("bounded replay retains the newest bytes and close only detaches", () => {
  const state = createTerminalPanelState();
  const tab = addUserTerminal(state, descriptor({
    sessionId: "user:one",
    generation: "one",
    origin: "user",
    threadId: null,
    commandItemId: null,
    processId: "one",
  }));
  appendTerminalOutput(
    tab,
    Buffer.alloc(MAX_TERMINAL_REPLAY_BYTES + 5, "x").toString("base64"),
    null,
  );
  assert.equal(tab.replay.length, MAX_TERMINAL_REPLAY_BYTES);
  assert.equal(tab.replayTruncated, true);
  assert.equal(closeTerminalTab(state, tab.id), true);
  assert.equal(state.tabs.length, 0);
  mergeTerminalSessions(state, [descriptor({
    sessionId: "user:one",
    generation: "one",
    origin: "user",
    threadId: null,
    commandItemId: null,
    processId: "one",
  })]);
  assert.equal(state.tabs.length, 0);
});

test("snapshot watermark replaces older deltas and rejects duplicates", () => {
  const state = createTerminalPanelState();
  mergeTerminalSessions(state, [descriptor()], "thread");
  const tab = state.tabs[0];
  appendTerminalOutput(tab, Buffer.from("old").toString("base64"), 1);
  appendTerminalOutput(tab, Buffer.from("gap").toString("base64"), 3);
  assert.equal(tab.hasSequenceGap, true);

  mergeTerminalSessions(
    state,
    [
      descriptor({
        replayBase64: Buffer.from("snapshot").toString("base64"),
        replayThroughSequence: 3,
      }),
    ],
    "thread",
  );

  assert.equal(tab.lastSequence, 3);
  assert.equal(tab.hasSequenceGap, false);
  assert.equal(tab.replay.toString(), "snapshot");
  assert.equal(
    appendTerminalOutput(tab, Buffer.from("duplicate").toString("base64"), 3),
    false,
  );
  assert.equal(
    appendTerminalOutput(tab, Buffer.from("+delta").toString("base64"), 4),
    true,
  );
  assert.equal(tab.replay.toString(), "snapshot+delta");
});

test("server generation replaces the optimistic user terminal identity", () => {
  const state = createTerminalPanelState();
  const tab = addUserTerminal(state, descriptor({
    sessionId: "user:one",
    generation: "one",
    origin: "user",
    threadId: null,
    commandItemId: null,
    processId: "one",
  }));

  mergeTerminalSessions(state, [descriptor({
    sessionId: "user:one",
    generation: "runtime-generation",
    origin: "user",
    threadId: null,
    commandItemId: null,
    processId: "one",
  })]);

  assert.equal(state.tabs.length, 1);
  assert.equal(tab.generation, "runtime-generation");
  assert.equal(tab.status, "running");
});

test("explicit reattach clears detached tombstones", () => {
  const state = createTerminalPanelState();
  mergeTerminalSessions(state, [descriptor()], "thread");
  closeTerminalTab(state, state.tabs[0].id);
  assert.equal(terminalPanelSnapshot(state).detachedCount, 2);

  assert.equal(reattachTerminalSessions(state), 2);
  mergeTerminalSessions(state, [descriptor()], "thread");

  assert.equal(state.tabs.length, 1);
  assert.equal(terminalPanelSnapshot(state).detachedCount, 0);
});

test("focusing a non-PTY command creates a read-only output tab", () => {
  const state = createTerminalPanelState();
  const tab = focusCommandTerminal(
    state,
    descriptor({
      sessionId: "command:thread:call",
      processId: "call",
      replayBase64: Buffer.from("output").toString("base64"),
      canResize: false,
      canWrite: false,
      canTerminate: false,
    }),
  );

  assert.equal(tab.readOnlyOutput, true);
  assert.equal(tab.canResize, false);
  assert.equal(tab.canWrite, false);
  assert.equal(tab.canTerminate, false);
  assert.equal(terminalTabSupports(tab, "write"), false);
  assert.equal(terminalTabSupports(tab, "resize"), false);
  assert.equal(terminalTabSupports(tab, "terminate"), false);
  assert.equal(tab.replay.toString(), "output");
  assert.equal(state.activeTabId, tab.id);

  mergeTerminalSessions(state, [], "thread");
  assert.equal(tab.status, "running");
});

test("focusing an existing model terminal reuses and selects its tab", () => {
  const state = createTerminalPanelState();
  mergeTerminalSessions(state, [descriptor()], "thread");
  const existing = state.tabs[0];

  const tab = focusCommandTerminal(
    state,
    descriptor({
      sessionId: "command:thread:call",
      canResize: false,
      canWrite: false,
      canTerminate: false,
    }),
  );

  assert.equal(tab, existing);
  assert.equal(state.tabs.length, 1);
  assert.equal(state.activeTabId, existing.id);
  assert.equal(existing.readOnlyOutput, undefined);
  assert.equal(terminalTabSupports(existing, "write"), true);
  assert.equal(terminalTabSupports(existing, "resize"), true);
  assert.equal(terminalTabSupports(existing, "terminate"), true);
});

test("focus descriptor keeps running active command available despite process mismatch", () => {
  const command = {
    threadId: "thread",
    commandItemId: "call",
    processId: "stale-renderer-process",
  };
  const activeCommand = {
    type: "commandExecution",
    id: "call",
    command: "npm test",
    cwd: "/repo",
    processId: "runtime-process",
    status: "running",
    aggregatedOutput: "ready\n",
  };

  assert.equal(
    activeCommandForTerminalFocus(
      { activeCommandItems: [activeCommand] },
      command,
    ),
    activeCommand,
  );
  assert.deepEqual(commandFocusDescriptor(command, activeCommand), {
    sessionId: "command:thread:call",
    generation: "call",
    origin: "model",
    threadId: "thread",
    commandItemId: "call",
    processId: "runtime-process",
    title: "npm test",
    cwd: "/repo",
    replayBase64: Buffer.from("ready\n").toString("base64"),
    replayTruncated: false,
    replayThroughSequence: 0,
    size: null,
    canResize: false,
    canWrite: false,
    canTerminate: false,
  });
});

test("command output cache hydrates focused running command replay", () => {
  const state = createTerminalPanelState();
  appendCommandOutputCache(
    state,
    "thread",
    "call",
    Buffer.from("first\n").toString("base64"),
    1,
  );
  appendCommandOutputCache(
    state,
    "thread",
    "call",
    Buffer.from("second\n").toString("base64"),
    2,
  );

  const descriptorWithCache = applyCommandOutputCacheToDescriptor(
    state,
    commandFocusDescriptorFromRequest({
      threadId: "thread",
      commandItemId: "call",
      processId: "42",
      command: "npm test",
      cwd: "/repo",
      status: "running",
    }),
  );
  const tab = focusCommandTerminal(state, descriptorWithCache);

  assert.equal(tab.replay.toString(), "first\nsecond\n");
  assert.equal(tab.lastSequence, 2);
  assert.equal(tab.canResize, false);
});

test("focused read-only command hydrates replay when active output arrives later", () => {
  const state = createTerminalPanelState();
  const command = {
    threadId: "thread",
    commandItemId: "call",
    processId: "42",
    command: "npm test",
    cwd: "/repo",
    status: "running",
  };
  const fallback = focusCommandTerminal(
    state,
    commandFocusDescriptorFromRequest(command),
  );

  assert.equal(fallback.replay.toString(), "");
  assert.equal(fallback.lastSequence, 0);

  const hydrated = focusCommandTerminal(
    state,
    commandFocusDescriptor(command, {
      type: "commandExecution",
      id: "call",
      command: "npm test",
      cwd: "/repo",
      processId: "42",
      status: "running",
      aggregatedOutput: "cached output\n",
    }),
  );

  assert.equal(hydrated, fallback);
  assert.equal(state.tabs.length, 1);
  assert.equal(state.activeTabId, fallback.id);
  assert.equal(hydrated.replay.toString(), "cached output\n");
  assert.equal(hydrated.lastSequence, 0);
});

test("command output cache can be shared before terminal panel opens", () => {
  const sharedCache = new Map();
  const notificationOnlyState = { commandOutputCache: sharedCache };
  appendCommandOutputCache(
    notificationOnlyState,
    "thread",
    "call",
    Buffer.from("cached before open\n").toString("base64"),
    1,
  );

  const state = createTerminalPanelState(sharedCache);
  const descriptorWithCache = applyCommandOutputCacheToDescriptor(
    state,
    commandFocusDescriptorFromRequest({
      threadId: "thread",
      commandItemId: "call",
      processId: "42",
      command: "npm test",
      cwd: "/repo",
      status: "running",
    }),
  );
  const tab = focusCommandTerminal(state, descriptorWithCache);

  assert.equal(tab.replay.toString(), "cached before open\n");
});

test("command output cache marks tail-only hydrate truncated", () => {
  const state = createTerminalPanelState();
  appendCommandOutputCache(
    state,
    "thread",
    "call",
    Buffer.from("tail only\n").toString("base64"),
    3,
  );

  const descriptorWithCache = applyCommandOutputCacheToDescriptor(
    state,
    commandFocusDescriptorFromRequest({
      threadId: "thread",
      commandItemId: "call",
      processId: "42",
      command: "npm test",
      cwd: "/repo",
      status: "running",
    }),
  );

  assert.equal(
    Buffer.from(descriptorWithCache.replayBase64, "base64").toString(),
    "tail only\n",
  );
  assert.equal(descriptorWithCache.replayTruncated, true);
});

test("focused command replay keeps descriptor when cache is not newer", () => {
  const state = createTerminalPanelState();
  appendCommandOutputCache(
    state,
    "thread",
    "call",
    Buffer.from("cached\n").toString("base64"),
    1,
  );

  const descriptorWithCache = applyCommandOutputCacheToDescriptor(state, {
    ...descriptor({
      sessionId: "command:thread:call",
      processId: "42",
      replayBase64: Buffer.from("live\n").toString("base64"),
      replayThroughSequence: 1,
    }),
    canWrite: false,
    canTerminate: false,
  });

  assert.equal(
    Buffer.from(descriptorWithCache.replayBase64, "base64").toString(),
    "live\n",
  );
  assert.equal(descriptorWithCache.replayThroughSequence, 1);
});

test("focused command replay appends contiguous cache after descriptor", () => {
  const state = createTerminalPanelState();
  appendCommandOutputCache(
    state,
    "thread",
    "call",
    Buffer.from("tail\n").toString("base64"),
    3,
  );

  const descriptorWithCache = applyCommandOutputCacheToDescriptor(state, {
    ...descriptor({
      sessionId: "command:thread:call",
      processId: "42",
      replayBase64: Buffer.from("head\n").toString("base64"),
      replayThroughSequence: 2,
    }),
    canWrite: false,
    canTerminate: false,
  });

  assert.equal(
    Buffer.from(descriptorWithCache.replayBase64, "base64").toString(),
    "head\ntail\n",
  );
  assert.equal(descriptorWithCache.replayThroughSequence, 3);
});

test("focused command replay rejects non-contiguous newer cache over descriptor", () => {
  const state = createTerminalPanelState();
  appendCommandOutputCache(
    state,
    "thread",
    "call",
    Buffer.from("late\n").toString("base64"),
    4,
  );

  const descriptorWithCache = applyCommandOutputCacheToDescriptor(state, {
    ...descriptor({
      sessionId: "command:thread:call",
      processId: "42",
      replayBase64: Buffer.from("head\n").toString("base64"),
      replayThroughSequence: 2,
    }),
    canWrite: false,
    canTerminate: false,
  });

  assert.equal(
    Buffer.from(descriptorWithCache.replayBase64, "base64").toString(),
    "head\n",
  );
  assert.equal(descriptorWithCache.replayThroughSequence, 2);
});

test("command output cache can be deleted after command completion", () => {
  const state = createTerminalPanelState();
  appendCommandOutputCache(
    state,
    "thread",
    "call",
    Buffer.from("output\n").toString("base64"),
    1,
  );

  assert.equal(deleteCommandOutputCache(state, "thread", "call"), true);
  const descriptorWithCache = applyCommandOutputCacheToDescriptor(
    state,
    commandFocusDescriptorFromRequest({
      threadId: "thread",
      commandItemId: "call",
      processId: "42",
      command: "npm test",
      cwd: "/repo",
      status: "running",
    }),
  );

  assert.equal(descriptorWithCache.replayBase64, null);
});

test("focus descriptor rejects truly stale command items", () => {
  const command = { threadId: "thread", commandItemId: "call" };

  assert.equal(activeCommandForTerminalFocus(null, command), null);
  assert.equal(
    activeCommandForTerminalFocus({ activeCommandItems: [] }, command),
    null,
  );
  assert.equal(
    activeCommandForTerminalFocus(
      {
        activeCommandItems: [
          {
            type: "commandExecution",
            id: "call",
            command: "npm test",
            cwd: "/repo",
            status: "completed",
            aggregatedOutput: "done\n",
          },
        ],
      },
      command,
    ),
    null,
  );
});

test("live command session keeps focus available when active snapshot is stale", () => {
  const state = createTerminalPanelState();
  mergeTerminalSessions(
    state,
    [
      descriptor({
        sessionId: "model:thread:call:42",
        commandItemId: "call",
        processId: "42",
        title: "npm test",
        cwd: "/repo",
      }),
    ],
    "thread",
  );
  const command = {
    threadId: "thread",
    commandItemId: "call",
    processId: "stale-renderer-process",
  };

  const session = liveCommandSessionForTerminalFocus(state, command);
  assert.equal(session, state.tabs[0]);
  assert.deepEqual(
    commandFocusDescriptorForTerminalFocus(command, null, session, {
      liveSessionRefreshed: true,
    }),
    commandFocusDescriptorFromLiveSession(command, session),
  );
  const tab = focusCommandTerminal(
    state,
    commandFocusDescriptorFromLiveSession(command, session),
  );

  assert.equal(tab, state.tabs[0]);
  assert.equal(state.tabs.length, 1);
  assert.equal(state.activeTabId, tab.id);
  assert.equal(tab.readOnlyOutput, undefined);
});

test("live command session wins over active command fallback descriptor", () => {
  const state = createTerminalPanelState();
  mergeTerminalSessions(
    state,
    [
      descriptor({
        sessionId: "model:thread:call:42",
        commandItemId: "call",
        processId: "42",
        title: "npm test",
        cwd: "/repo",
      }),
    ],
    "thread",
  );
  const command = {
    threadId: "thread",
    commandItemId: "call",
    processId: null,
  };
  const activeCommand = {
    type: "commandExecution",
    id: "call",
    command: "npm test",
    cwd: "/repo",
    processId: null,
    status: "running",
    aggregatedOutput: null,
  };
  const session = liveCommandSessionForTerminalFocus(state, command);

  assert.deepEqual(
    commandFocusDescriptorForTerminalFocus(command, activeCommand, session, {
      liveSessionRefreshed: true,
    }),
    commandFocusDescriptorFromLiveSession(command, session),
  );
});

test("live command session cannot prove focus when session refresh failed", () => {
  const state = createTerminalPanelState();
  mergeTerminalSessions(state, [descriptor()], "thread");
  const command = {
    threadId: "thread",
    commandItemId: "call",
    processId: "42",
  };
  const session = liveCommandSessionForTerminalFocus(state, command);

  assert.equal(session, state.tabs[0]);
  assert.equal(
    commandFocusDescriptorForTerminalFocus(command, null, session, {
      liveSessionRefreshed: false,
    }),
    null,
  );
});

test("active command item proves focus even when session refresh failed", () => {
  const command = {
    threadId: "thread",
    commandItemId: "call",
    processId: "stale-renderer-process",
  };
  const activeCommand = {
    type: "commandExecution",
    id: "call",
    command: "npm test",
    cwd: "/repo",
    processId: "runtime-process",
    status: "running",
    aggregatedOutput: null,
  };

  assert.deepEqual(
    commandFocusDescriptorForTerminalFocus(command, activeCommand, null, {
      liveSessionRefreshed: false,
    }),
    commandFocusDescriptor(command, activeCommand),
  );
});

test("renderer live command summary proves focus without process id or live session", () => {
  const command = {
    threadId: "thread",
    commandItemId: "call",
    processId: null,
    command: "npm test",
    cwd: "/repo",
    status: "inProgress",
  };

  assert.deepEqual(
    commandFocusDescriptorForTerminalFocus(command, null, null, {
      liveSessionRefreshed: true,
    }),
    commandFocusDescriptorFromRequest(command),
  );
});

test("renderer completed command summary does not prove focus", () => {
  assert.equal(
    commandFocusDescriptorForTerminalFocus(
      {
        threadId: "thread",
        commandItemId: "call",
        processId: null,
        command: "npm test",
        cwd: "/repo",
        status: "completed",
      },
      null,
      null,
      { liveSessionRefreshed: true },
    ),
    null,
  );
});

test("live session merge upgrades a read-only request fallback", () => {
  const state = createTerminalPanelState();
  const command = {
    threadId: "thread",
    commandItemId: "call",
    processId: "42",
    command: "npm test",
    cwd: "/repo",
    status: "running",
  };
  const fallback = focusCommandTerminal(
    state,
    commandFocusDescriptorFromRequest(command),
  );

  assert.equal(fallback.readOnlyOutput, true);

  mergeTerminalSessions(
    state,
    [
      descriptor({
        sessionId: "model:thread:call:42",
        commandItemId: "call",
        processId: "42",
        title: "npm test",
        cwd: "/repo",
      }),
    ],
    "thread",
  );

  assert.equal(state.tabs.length, 1);
  assert.equal(state.tabs[0], fallback);
  assert.equal(fallback.readOnlyOutput, undefined);
  assert.equal(fallback.canWrite, true);
  assert.equal(
    liveCommandSessionForTerminalFocus(state, {
      threadId: "thread",
      commandItemId: "call",
      processId: "42",
    }),
    fallback,
  );
});

test("live session merge upgrades read-only fallback with placeholder process id", () => {
  const state = createTerminalPanelState();
  const command = {
    threadId: "thread",
    commandItemId: "call",
    processId: null,
    command: "npm test",
    cwd: "/repo",
    status: "running",
  };
  const fallback = focusCommandTerminal(
    state,
    commandFocusDescriptorFromRequest(command),
  );

  assert.equal(fallback.processId, "call");
  assert.equal(fallback.readOnlyOutput, true);
  assert.equal(fallback.canResize, false);

  mergeTerminalSessions(
    state,
    [
      descriptor({
        sessionId: "model:thread:call:42",
        generation: "call",
        commandItemId: "call",
        processId: "42",
        title: "npm test",
        cwd: "/repo",
      }),
    ],
    "thread",
  );

  assert.equal(state.tabs.length, 1);
  assert.equal(state.tabs[0], fallback);
  assert.equal(state.activeTabId, fallback.id);
  assert.equal(fallback.sessionId, "model:thread:call:42");
  assert.equal(fallback.processId, "42");
  assert.equal(fallback.readOnlyOutput, undefined);
  assert.equal(fallback.canResize, true);
  assert.equal(terminalTabSupports(fallback, "resize"), true);
});

test("live session merge finds read-only fallback after unrelated tabs", () => {
  const state = createTerminalPanelState();
  addUserTerminal(state, descriptor({
    sessionId: "user:one",
    generation: "one",
    origin: "user",
    threadId: null,
    commandItemId: null,
    processId: "one",
  }));
  const fallback = focusCommandTerminal(
    state,
    commandFocusDescriptorFromRequest({
      threadId: "thread",
      commandItemId: "call",
      processId: null,
      command: "npm test",
      cwd: "/repo",
      status: "running",
    }),
  );

  mergeTerminalSessions(
    state,
    [
      descriptor({
        sessionId: "model:thread:call:runtime-process",
        generation: "call",
        commandItemId: "call",
        processId: "runtime-process",
        title: "npm test",
        cwd: "/repo",
      }),
    ],
    "thread",
  );

  assert.equal(state.tabs.length, 2);
  assert.equal(state.tabs[1], fallback);
  assert.equal(fallback.sessionId, "model:thread:call:runtime-process");
  assert.equal(fallback.processId, "runtime-process");
  assert.equal(fallback.readOnlyOutput, undefined);
});

test("live session merge preserves tab-order priority over exact session matches", () => {
  const fallbackState = createTerminalPanelState();
  const fallback = focusCommandTerminal(
    fallbackState,
    commandFocusDescriptorFromRequest({
      threadId: "thread",
      commandItemId: "call",
      processId: null,
      command: "npm test",
      cwd: "/repo",
      status: "running",
    }),
  );
  const state = createTerminalPanelState();
  mergeTerminalSessions(
    state,
    [
      descriptor({
        sessionId: "model:thread:call:runtime-process",
        generation: "call",
        commandItemId: "call",
        processId: "runtime-process",
      }),
    ],
    "thread",
  );
  const exactSessionTab = state.tabs[0];
  state.tabs.unshift(fallback);

  mergeTerminalSessions(
    state,
    [
      descriptor({
        sessionId: "model:thread:call:runtime-process",
        generation: "call",
        commandItemId: "call",
        processId: "runtime-process",
      }),
    ],
    "thread",
  );

  assert.equal(state.tabs.length, 2);
  assert.equal(state.tabs[0], fallback);
  assert.equal(state.tabs[1], exactSessionTab);
  assert.equal(fallback.sessionId, "model:thread:call:runtime-process");
  assert.equal(fallback.readOnlyOutput, undefined);

  mergeTerminalSessions(
    state,
    [
      descriptor({
        sessionId: "model:thread:call:runtime-process",
        generation: "call",
        commandItemId: "call",
        processId: "runtime-process",
        replayBase64: Buffer.from("fresh\n").toString("base64"),
      }),
    ],
    "thread",
  );

  assert.equal(state.tabs.length, 2);
  assert.equal(fallback.replay.toString(), "fresh\n");
  assert.equal(exactSessionTab.replay.toString(), "");
});

test("live session merge does not reuse upgraded fallback for a second process", () => {
  const state = createTerminalPanelState();
  focusCommandTerminal(
    state,
    commandFocusDescriptorFromRequest({
      threadId: "thread",
      commandItemId: "call",
      processId: null,
      command: "npm test",
      cwd: "/repo",
      status: "running",
    }),
  );

  mergeTerminalSessions(
    state,
    [
      descriptor({
        sessionId: "model:thread:call:runtime-one",
        generation: "call",
        commandItemId: "call",
        processId: "runtime-one",
      }),
      descriptor({
        sessionId: "model:thread:call:runtime-two",
        generation: "call",
        commandItemId: "call",
        processId: "runtime-two",
      }),
    ],
    "thread",
  );

  assert.equal(state.tabs.length, 2);
  assert.equal(state.tabs[0].processId, "runtime-one");
  assert.equal(state.tabs[1].processId, "runtime-two");
});

test("read-only model command fallback requests live session refresh while output streams", () => {
  const state = createTerminalPanelState();
  const command = {
    threadId: "thread",
    commandItemId: "call",
    processId: null,
    command: "fly deploy",
    cwd: "/repo",
    status: "running",
  };
  const fallback = focusCommandTerminal(
    state,
    commandFocusDescriptorFromRequest(command),
  );

  assert.equal(fallback.processId, "call");
  assert.equal(fallback.readOnlyOutput, true);
  assert.equal(terminalTabNeedsLiveSessionRefresh(fallback), true);

  fallback.processId = "42";
  appendTerminalOutput(
    fallback,
    Buffer.from("==> Verifying app config\r").toString("base64"),
    1,
  );

  assert.equal(fallback.readOnlyOutput, true);
  assert.equal(terminalTabNeedsLiveSessionRefresh(fallback), true);

  mergeTerminalSessions(
    state,
    [
      descriptor({
        sessionId: "model:thread:call:42",
        generation: "call",
        commandItemId: "call",
        processId: "42",
        title: "fly deploy",
        cwd: "/repo",
      }),
    ],
    "thread",
  );

  assert.equal(state.tabs[0], fallback);
  assert.equal(fallback.readOnlyOutput, undefined);
  assert.equal(fallback.canResize, true);
  assert.equal(terminalTabNeedsLiveSessionRefresh(fallback), false);
});

test("closed read-only fallback detaches later live descriptor with different process id", () => {
  const state = createTerminalPanelState();
  const command = {
    threadId: "thread",
    commandItemId: "call",
    processId: null,
    command: "npm test",
    cwd: "/repo",
    status: "running",
  };
  const fallback = focusCommandTerminal(
    state,
    commandFocusDescriptorFromRequest(command),
  );

  assert.equal(closeTerminalTab(state, fallback.id), true);
  assert.equal(
    state.detachedSessionKeys.has(terminalSessionKey(fallback)),
    true,
  );
  assert.equal(
    state.detachedSessionKeys.has(terminalCommandItemKey(fallback)),
    true,
  );

  mergeTerminalSessions(
    state,
    [
      descriptor({
        sessionId: "model:thread:call:42",
        generation: "call",
        commandItemId: "call",
        processId: "42",
        title: "npm test",
        cwd: "/repo",
      }),
    ],
    "thread",
  );
  assert.equal(state.tabs.length, 0);

  assert.equal(reattachTerminalSessions(state), 2);
  mergeTerminalSessions(
    state,
    [
      descriptor({
        sessionId: "model:thread:call:42",
        generation: "call",
        commandItemId: "call",
        processId: "42",
        title: "npm test",
        cwd: "/repo",
      }),
    ],
    "thread",
  );
  assert.equal(state.tabs.length, 1);
  assert.equal(state.tabs[0].readOnlyOutput, undefined);
  assert.equal(state.tabs[0].canResize, true);
});

test("live command session can match process id when descriptor lacks command item id", () => {
  const state = createTerminalPanelState();
  mergeTerminalSessions(
    state,
    [
      descriptor({
        sessionId: "model:thread:missing-id:42",
        commandItemId: null,
        processId: "42",
      }),
    ],
    "thread",
  );
  const command = {
    threadId: "thread",
    commandItemId: "call",
    processId: "42",
  };

  const session = liveCommandSessionForTerminalFocus(state, command);
  assert.equal(session, state.tabs[0]);
  const tab = focusCommandTerminal(
    state,
    commandFocusDescriptorFromLiveSession(command, session),
  );

  assert.equal(tab, state.tabs[0]);
  assert.equal(state.tabs.length, 1);
  assert.equal(state.activeTabId, tab.id);
});

test("live command session does not match without command item or process proof", () => {
  const state = createTerminalPanelState();
  mergeTerminalSessions(
    state,
    [
      descriptor({
        sessionId: "model:thread:missing-id:42",
        commandItemId: null,
        processId: "42",
      }),
    ],
    "thread",
  );

  assert.equal(
    liveCommandSessionForTerminalFocus(state, {
      threadId: "thread",
      commandItemId: "call",
    }),
    null,
  );
});

test("completed or lost sessions do not bypass stale focus guard", () => {
  const exitedState = createTerminalPanelState();
  mergeTerminalSessions(exitedState, [descriptor()], "thread");
  markTerminalExited(exitedState.tabs[0], 0);

  assert.equal(
    liveCommandSessionForTerminalFocus(exitedState, {
      threadId: "thread",
      commandItemId: "call",
      processId: "42",
    }),
    null,
  );

  const lostState = createTerminalPanelState();
  mergeTerminalSessions(lostState, [descriptor()], "thread");
  mergeTerminalSessions(lostState, [], "thread");

  assert.equal(
    liveCommandSessionForTerminalFocus(lostState, {
      threadId: "thread",
      commandItemId: "call",
      processId: "42",
    }),
    null,
  );
});

test("focusing a running command creates fallback when descriptor id mismatches", () => {
  const state = createTerminalPanelState();
  mergeTerminalSessions(
    state,
    [
      descriptor({
        sessionId: "model:thread:other:42",
        commandItemId: "other-call",
        processId: "42",
      }),
    ],
    "thread",
  );

  const tab = focusCommandTerminal(
    state,
    descriptor({
      sessionId: "command:thread:call",
      commandItemId: "call",
      processId: "42",
      replayBase64: Buffer.from("fallback").toString("base64"),
      canResize: false,
      canWrite: false,
      canTerminate: false,
    }),
  );

  assert.equal(state.tabs.length, 2);
  assert.equal(tab.readOnlyOutput, true);
  assert.equal(tab.id, "command:thread:call");
  assert.equal(state.activeTabId, tab.id);
  assert.equal(tab.replay.toString(), "fallback");
});

test("focusing a running command reuses PTY descriptor missing command item id", () => {
  const state = createTerminalPanelState();
  mergeTerminalSessions(
    state,
    [
      descriptor({
        sessionId: "model:thread:missing-id:42",
        commandItemId: null,
        processId: "42",
      }),
    ],
    "thread",
  );
  const existing = state.tabs[0];

  const tab = focusCommandTerminal(
    state,
    descriptor({
      sessionId: "command:thread:call",
      commandItemId: "call",
      processId: "42",
      canResize: false,
      canWrite: false,
      canTerminate: false,
    }),
  );

  assert.equal(tab, existing);
  assert.equal(state.tabs.length, 1);
  assert.equal(state.activeTabId, existing.id);
  assert.equal(existing.readOnlyOutput, undefined);
});

test("focusing a running command skips lost missing-id PTY tab", () => {
  const state = createTerminalPanelState();
  mergeTerminalSessions(
    state,
    [
      descriptor({
        sessionId: "model:thread:missing-id:42",
        commandItemId: null,
        processId: "42",
      }),
    ],
    "thread",
  );
  mergeTerminalSessions(state, [], "thread");

  const tab = focusCommandTerminal(
    state,
    descriptor({
      sessionId: "command:thread:call",
      commandItemId: "call",
      processId: "42",
      replayBase64: Buffer.from("fallback").toString("base64"),
      canResize: false,
      canWrite: false,
      canTerminate: false,
    }),
  );

  assert.equal(state.tabs.length, 2);
  assert.equal(tab.readOnlyOutput, true);
  assert.equal(tab.id, "command:thread:call");
  assert.equal(state.activeTabId, tab.id);
  assert.equal(tab.replay.toString(), "fallback");
});

test("focusing a running command skips exited missing-id PTY tab", () => {
  const state = createTerminalPanelState();
  mergeTerminalSessions(
    state,
    [
      descriptor({
        sessionId: "model:thread:missing-id:42",
        commandItemId: null,
        processId: "42",
      }),
    ],
    "thread",
  );
  markTerminalExited(state.tabs[0], 0);

  const tab = focusCommandTerminal(
    state,
    descriptor({
      sessionId: "command:thread:call",
      commandItemId: "call",
      processId: "42",
      replayBase64: Buffer.from("fallback").toString("base64"),
      canResize: false,
      canWrite: false,
      canTerminate: false,
    }),
  );

  assert.equal(state.tabs.length, 2);
  assert.equal(tab.readOnlyOutput, true);
  assert.equal(tab.id, "command:thread:call");
  assert.equal(state.activeTabId, tab.id);
  assert.equal(tab.replay.toString(), "fallback");
});
