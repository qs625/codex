const assert = require("node:assert/strict");
const test = require("node:test");
const { EventEmitter } = require("node:events");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");

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

test("preload entry keeps subscription helper self-contained for sandbox", () => {
  const preloadSource = readFileSync(join(__dirname, "preload.cjs"), "utf8");

  assert.doesNotMatch(preloadSource, /require\("\.\/preloadSubscriptions\.cjs"\)/);
  assert.match(preloadSource, /function subscribeIpcState\(channel, listener\)/);
  assert.match(preloadSource, /typeof listener !== "function"/);
  assert.match(
    preloadSource,
    /subscribeBrowserState\(listener\) \{[\s\S]*subscribeIpcState\("codex:browser:state", listener\)/,
  );
  assert.match(
    preloadSource,
    /subscribeTerminalState\(listener\) \{[\s\S]*subscribeIpcState\("codex:terminal:state", listener\)/,
  );
});
