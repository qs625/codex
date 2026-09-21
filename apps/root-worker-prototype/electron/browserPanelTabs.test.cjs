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

test("browser panel native view lifecycle raises only on explicit show or tab actions", () => {
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
    /function setBrowserPanelBounds\(panel, bounds\) \{[\s\S]*attachActiveBrowserPanelView\(panel\);[\s\S]*\}/,
  );
  assert.doesNotMatch(
    mainSource.slice(
      mainSource.indexOf("function setBrowserPanelBounds(panel, bounds) {"),
      mainSource.indexOf("function sendBrowserPanelState(panel)", mainSource.indexOf("function setBrowserPanelBounds(panel, bounds) {")),
    ),
    /attachActiveBrowserPanelView\(panel, \{ raise: true \}\)/,
    "passive Browser bounds refresh must not raise the native view and steal focus",
  );
  assert.match(
    mainSource,
    /function attachActiveBrowserPanelView\(panel, \{ raise = false \} = \{\}\)/,
  );
  assert.match(
    mainSource,
    /function ensureBrowserPanelTabAttachedForNavigation\([\s\S]*attachActiveBrowserPanelView\(panel, \{ raise \}\)/,
  );
  assert.match(
    mainSource,
    /function ensureBrowserPanelTabAttachedForNavigation\([\s\S]*if \(!panel\.visible\) \{[\s\S]*throw new Error\("Browser page is not visible in the panel"\);/,
  );
  assert.match(
    mainSource,
    /function ensureBrowserPanelTabAttachedForNavigation\([\s\S]*browserPanelBoundsAreVisible\(panel\.bounds\)[\s\S]*throw new Error\("Browser page has no visible panel bounds"\);/,
  );
  assert.match(
    mainSource,
    /async function waitForBrowserPanelVisibleNavigationTarget\(panel, tab\) \{[\s\S]*if \(!panel\.visible\) \{[\s\S]*throw new Error\("Browser page is not visible in the panel"\);[\s\S]*ensureBrowserPanelTabAttachedForNavigation\(panel, tab, \{ raise: true \}\);[\s\S]*waitForBrowserPanelDevToolsTarget\(tab\.view\.webContents\);/,
  );
  assert.match(
    mainSource,
    /async function loadBrowserPanelTabUrl\([\s\S]*\{ requireVisiblePanel = true \} = \{\},[\s\S]*stopBrowserPanelWebContentsLoad\(tab\);[\s\S]*if \(requireVisiblePanel\) \{[\s\S]*ensureBrowserPanelTabAttachedForNavigation\(panel, tab, \{ raise: true \}\);[\s\S]*tab\.view\.webContents\.loadURL\(normalized\.url\)[\s\S]*await waitForBrowserPanelNavigationTarget\(panel, tab, \{[\s\S]*requireVisiblePanel,[\s\S]*\}\);[\s\S]*completeBrowserPanelNavigation\(panel, tab, navigationSequence\);/,
  );
  assert.match(
    mainSource,
    /tab\.view\.webContents\.on\("did-stop-loading", \(\) => \{[\s\S]*void completeBrowserPanelNavigationWhenTargetReady\([\s\S]*tab\.pendingNavigationSequence,[\s\S]*\);[\s\S]*return;/,
  );
  assert.match(
    mainSource,
    /tab\.view\.webContents\.on\("did-finish-load", \(\) => \{[\s\S]*void completeBrowserPanelNavigationWhenTargetReady\([\s\S]*tab\.pendingNavigationSequence,[\s\S]*\);[\s\S]*return;/,
  );
  assert.match(
    mainSource,
    /ipcMain\.handle\("codex:browser:reload"[\s\S]*tab\.pendingNavigationSequence = navigationSequence;[\s\S]*tab\.pendingNavigationRequiresVisiblePanel = true;[\s\S]*tab\.pendingNavigationTarget = tab\.view\.webContents\.getURL\(\) \|\| null;/,
  );
  assert.match(
    mainSource,
    /tab\.view\.webContents\.on\("did-start-loading", \(\) => \{[\s\S]*tab\.pendingNavigationSequence = navigationSequence;[\s\S]*tab\.pendingNavigationRequiresVisiblePanel = true;[\s\S]*tab\.pendingNavigationTarget = currentUrl \|\| null;/,
  );
  assert.match(
    mainSource,
    /function stopBrowserPanelNavigation\(tab\) \{[\s\S]*tab\.pendingNavigationSequence = null;[\s\S]*tab\.pendingNavigationRequiresVisiblePanel = true;[\s\S]*tab\.pendingNavigationTarget = null;/,
  );
  assert.match(mainSource, /function detachAllBrowserPanelViews\(panel\)/);
});
