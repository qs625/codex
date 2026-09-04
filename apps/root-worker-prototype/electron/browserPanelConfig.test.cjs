const test = require("node:test");
const assert = require("node:assert/strict");

const {
  browserPanelWebPreferences,
  browserSessionPartition,
} = require("./browserPanelConfig.cjs");

test("browser panel keeps isolated sandboxed webContents preferences", () => {
  assert.deepEqual(browserPanelWebPreferences(), {
    allowRunningInsecureContent: false,
    contextIsolation: true,
    nodeIntegration: false,
    partition: browserSessionPartition,
    sandbox: true,
    webSecurity: true,
  });
  assert.equal(browserSessionPartition, "persist:root-worker-browser");
});
