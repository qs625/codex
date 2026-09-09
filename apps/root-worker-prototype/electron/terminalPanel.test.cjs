const assert = require("node:assert/strict");
const test = require("node:test");

const {
  MAX_TERMINAL_REPLAY_BYTES,
  addUserTerminal,
  appendTerminalOutput,
  closeTerminalTab,
  createTerminalPanelState,
  mergeTerminalSessions,
  terminalPanelSnapshot,
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
