const assert = require("node:assert/strict");
const test = require("node:test");

const {
  MAX_TERMINAL_REPLAY_BYTES,
  activeCommandForTerminalFocus,
  addUserTerminal,
  appendTerminalOutput,
  closeTerminalTab,
  commandFocusDescriptor,
  commandFocusDescriptorForTerminalFocus,
  commandFocusDescriptorFromLiveSession,
  createTerminalPanelState,
  focusCommandTerminal,
  liveCommandSessionForTerminalFocus,
  markTerminalExited,
  mergeTerminalSessions,
  reattachTerminalSessions,
  terminalPanelSnapshot,
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
  assert.equal(terminalPanelSnapshot(state).detachedCount, 1);

  assert.equal(reattachTerminalSessions(state), 1);
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
    canResize: false,
    canWrite: false,
    canTerminate: false,
  });
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
