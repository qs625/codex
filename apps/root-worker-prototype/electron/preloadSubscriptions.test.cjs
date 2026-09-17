const assert = require("node:assert/strict");
const test = require("node:test");
const { EventEmitter } = require("node:events");

const { subscribeIpcState } = require("./preloadSubscriptions.cjs");

test("subscribeIpcState ignores invalid listeners", () => {
  const ipcRenderer = new EventEmitter();

  const unsubscribe = subscribeIpcState(
    ipcRenderer,
    "codex:browser:state",
    undefined,
  );

  assert.equal(typeof unsubscribe, "function");
  assert.doesNotThrow(() => {
    ipcRenderer.emit("codex:browser:state", {}, { loading: false });
  });
  assert.doesNotThrow(() => unsubscribe());
  assert.equal(ipcRenderer.listenerCount("codex:browser:state"), 0);
});

test("subscribeIpcState delivers valid state and unsubscribes cleanly", () => {
  const ipcRenderer = new EventEmitter();
  const received = [];
  const unsubscribe = subscribeIpcState(
    ipcRenderer,
    "codex:browser:state",
    (state) => received.push(state),
  );

  ipcRenderer.emit("codex:browser:state", {}, { url: "https://example.com/" });
  unsubscribe();
  ipcRenderer.emit("codex:browser:state", {}, { url: "https://ignored.test/" });

  assert.deepEqual(received, [{ url: "https://example.com/" }]);
  assert.equal(ipcRenderer.listenerCount("codex:browser:state"), 0);
});
