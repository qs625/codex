const test = require("node:test");
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");

const {
  nextBrowserTabIdAfterClose,
  shouldDetachAttachedBrowserPanelView,
} = require("./browserPanelTabs.cjs");

const tabs = [{ id: "tab-a" }, { id: "tab-b" }, { id: "tab-c" }];

test("nextBrowserTabIdAfterClose keeps active tab when closing background tab", () => {
  assert.equal(nextBrowserTabIdAfterClose(tabs, "tab-a", "tab-b"), "tab-a");
});

test("nextBrowserTabIdAfterClose selects the next neighbor for active middle tab", () => {
  assert.equal(nextBrowserTabIdAfterClose(tabs, "tab-b", "tab-b"), "tab-c");
});

test("nextBrowserTabIdAfterClose selects previous neighbor for active last tab", () => {
  assert.equal(nextBrowserTabIdAfterClose(tabs, "tab-c", "tab-c"), "tab-b");
});

test("nextBrowserTabIdAfterClose returns null when the last tab closes", () => {
  assert.equal(nextBrowserTabIdAfterClose([{ id: "tab-a" }], "tab-a", "tab-a"), null);
});

test("shouldDetachAttachedBrowserPanelView skips destroyed windows and tabs", () => {
  assert.equal(
    shouldDetachAttachedBrowserPanelView({
      attachedTabId: "tab-a",
      tabDestroyed: false,
      windowDestroyed: false,
    }),
    true,
  );
  assert.equal(
    shouldDetachAttachedBrowserPanelView({
      attachedTabId: "tab-a",
      tabDestroyed: false,
      windowDestroyed: true,
    }),
    false,
  );
  assert.equal(
    shouldDetachAttachedBrowserPanelView({
      attachedTabId: "tab-a",
      tabDestroyed: true,
      windowDestroyed: false,
    }),
    false,
  );
  assert.equal(
    shouldDetachAttachedBrowserPanelView({
      attachedTabId: null,
      tabDestroyed: false,
      windowDestroyed: false,
    }),
    false,
  );
});

test("browser panel native view lifecycle raises active view and detaches hidden views", () => {
  const mainSource = readFileSync(join(__dirname, "main.cjs"), "utf8");

  assert.match(
    mainSource,
    /ipcMain\.handle\("codex:browser:show"[\s\S]*setBrowserPanelBounds\(panel, bounds\);[\s\S]*attachBrowserPanel\(panel\);/,
  );
  assert.match(
    mainSource,
    /function attachBrowserPanel\(panel\) \{[\s\S]*attachActiveBrowserPanelView\(panel, \{ raise: true \}\);[\s\S]*\}/,
  );
  assert.match(
    mainSource,
    /function detachBrowserPanel\(panel\) \{[\s\S]*detachAllBrowserPanelViews\(panel\);[\s\S]*panel\.visible = false;/,
  );
  assert.match(
    mainSource,
    /function setBrowserPanelBounds\(panel, bounds\) \{[\s\S]*attachActiveBrowserPanelView\(panel, \{ raise: true \}\);[\s\S]*\}/,
  );
  assert.match(
    mainSource,
    /function attachActiveBrowserPanelView\(panel, \{ raise = false \} = \{\}\)/,
  );
  assert.match(mainSource, /function detachAllBrowserPanelViews\(panel\)/);
});
