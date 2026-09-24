const test = require("node:test");
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");

const {
  isBrowserPanelTabAlreadyAttached,
  nextBrowserTabIdAfterClose,
  shouldAttachBrowserPanelView,
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

test("shouldAttachBrowserPanelView requires a visible panel with visible bounds", () => {
  assert.equal(
    shouldAttachBrowserPanelView({
      boundsVisible: true,
      tabMissing: false,
      tabDestroyed: false,
      panelVisible: true,
      windowDestroyed: false,
    }),
    true,
  );
  assert.equal(
    shouldAttachBrowserPanelView({
      boundsVisible: true,
      tabMissing: false,
      tabDestroyed: false,
      panelVisible: false,
      windowDestroyed: false,
    }),
    false,
  );
  assert.equal(
    shouldAttachBrowserPanelView({
      boundsVisible: false,
      tabMissing: false,
      tabDestroyed: false,
      panelVisible: true,
      windowDestroyed: false,
    }),
    false,
  );
  assert.equal(
    shouldAttachBrowserPanelView({
      boundsVisible: true,
      tabMissing: true,
      tabDestroyed: false,
      panelVisible: true,
      windowDestroyed: false,
    }),
    false,
  );
  assert.equal(
    shouldAttachBrowserPanelView({
      boundsVisible: true,
      tabMissing: false,
      tabDestroyed: true,
      panelVisible: true,
      windowDestroyed: false,
    }),
    false,
  );
  assert.equal(
    shouldAttachBrowserPanelView({
      boundsVisible: true,
      tabMissing: false,
      tabDestroyed: false,
      panelVisible: true,
      windowDestroyed: true,
    }),
    false,
  );
});

test("isBrowserPanelTabAlreadyAttached detects stable same-tab refreshes", () => {
  assert.equal(
    isBrowserPanelTabAlreadyAttached({
      attachedTabId: "browser-tab-1",
      tabId: "browser-tab-1",
    }),
    true,
  );
  assert.equal(
    isBrowserPanelTabAlreadyAttached({
      attachedTabId: "browser-tab-1",
      tabId: "browser-tab-2",
    }),
    false,
  );
  assert.equal(
    isBrowserPanelTabAlreadyAttached({
      attachedTabId: null,
      tabId: "browser-tab-2",
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
  const attachFunction = mainSource.slice(
    mainSource.indexOf("function attachActiveBrowserPanelView(panel, { raise = false } = {})"),
    mainSource.indexOf("function ensureBrowserPanelTabAttachedForNavigation", mainSource.indexOf("function attachActiveBrowserPanelView(panel, { raise = false } = {})")),
  );
  assert.match(
    attachFunction,
    /const boundsVisible = browserPanelBoundsAreVisible\(panel\.bounds\);[\s\S]*shouldAttachBrowserPanelView\(\{[\s\S]*boundsVisible,[\s\S]*\}\)[\s\S]*panel\.window\.contentView\.addChildView\(tab\.view\);/,
    "native Browser views must not attach before panel bounds are visible",
  );
  assert.match(
    attachFunction,
    /if \(!boundsVisible\) \{[\s\S]*detachAttachedBrowserPanelView\(panel\);[\s\S]*\}/,
    "native Browser views must detach when bounds stop being visible",
  );
  assert.match(
    attachFunction,
    /isBrowserPanelTabAlreadyAttached\(\{[\s\S]*attachedTabId: panel\.attachedTabId,[\s\S]*tabId: tab\.id,[\s\S]*\}\)[\s\S]*tab\.view\.setBounds\(panel\.bounds\);[\s\S]*return true;/,
    "same-tab BrowserView refreshes must not remove and re-add the WebContentsView before navigation",
  );
  assert.doesNotMatch(
    attachFunction.slice(
      attachFunction.indexOf("isBrowserPanelTabAlreadyAttached"),
      attachFunction.indexOf("return true;", attachFunction.indexOf("isBrowserPanelTabAlreadyAttached")) + "return true;".length,
    ),
    /removeChildView|addChildView|detachBrowserPanelTabView|detachAttachedBrowserPanelView/,
    "same-tab raises should avoid native view reparenting churn",
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
  const visibleNavigationTargetFunction = mainSource.slice(
    mainSource.indexOf("async function waitForBrowserPanelVisibleNavigationTarget(panel, tab)"),
    mainSource.indexOf("async function waitForBrowserPanelNavigationTarget", mainSource.indexOf("async function waitForBrowserPanelVisibleNavigationTarget(panel, tab)")),
  );
  assert.match(
    visibleNavigationTargetFunction,
    /if \(!panel\.visible\) \{[\s\S]*throw new Error\("Browser page is not visible in the panel"\);[\s\S]*if \(!browserPanelBoundsAreVisible\(panel\.bounds\)\) \{[\s\S]*throw new Error\("Browser page has no visible panel bounds"\);[\s\S]*ensureBrowserPanelTabAttachedForNavigation\(panel, tab, \{ raise: true \}\);/,
    "visible Browser navigation must still require a visible attached native view",
  );
  assert.match(
    visibleNavigationTargetFunction,
    /try \{[\s\S]*await waitForBrowserPanelDevToolsTarget\(tab\.view\.webContents\);[\s\S]*\} catch \(error\) \{[\s\S]*console\.warn\([\s\S]*"Browser panel DevTools target was not published for visible navigation"[\s\S]*error,[\s\S]*\);[\s\S]*\}/,
    "visible Browser navigation must not fail user page display when DevTools target publication is unavailable",
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

test("direct CDP-created Browser tabs do not force native attach while hidden", () => {
  const mainSource = readFileSync(join(__dirname, "main.cjs"), "utf8");
  const createTargetFunction = mainSource.slice(
    mainSource.indexOf("async function createBrowserPanelDebugTarget(target)"),
    mainSource.indexOf("async function loadBrowserPanelTabAboutBlankBootstrap", mainSource.indexOf("async function createBrowserPanelDebugTarget(target)")),
  );

  assert.match(
    createTargetFunction,
    /createBrowserPanelTab\(panel, \{ activate: true \}\)/,
    "direct CDP target creation still creates Browser-panel-managed tab state",
  );
  assert.match(
    createTargetFunction,
    /loadBrowserPanelTabUrl\(panel, tab, targetRequest\.url, \{\s*requireVisiblePanel: false,\s*\}\)/,
    "direct CDP target navigation must not require the panel to be visible",
  );
  assert.doesNotMatch(
    createTargetFunction,
    /ensureBrowserPanelTabAttachedForNavigation|attachActiveBrowserPanelView/,
    "direct CDP target creation must not directly attach or raise the native view",
  );
  assert.match(
    mainSource,
    /async function waitForBrowserPanelNavigationTarget\([\s\S]*if \(requireVisiblePanel\) \{[\s\S]*await waitForBrowserPanelVisibleNavigationTarget\(panel, tab\);[\s\S]*return;[\s\S]*\}[\s\S]*await waitForBrowserPanelDevToolsTarget\(tab\.view\.webContents\);[\s\S]*\}/,
    "hidden CDP-created Browser targets must still require DevTools target publication",
  );
});

test("closing Browser tabs destroys webContents before replacement tabs navigate", () => {
  const mainSource = readFileSync(join(__dirname, "main.cjs"), "utf8");
  const closeFunction = mainSource.slice(
    mainSource.indexOf("function closeBrowserPanelTab(panel, tabId)"),
    mainSource.indexOf(
      "function removeDestroyedBrowserPanelTab",
      mainSource.indexOf("function closeBrowserPanelTab(panel, tabId)"),
    ),
  );
  const destroyPanelFunction = mainSource.slice(
    mainSource.indexOf("function destroyBrowserPanel(window)"),
    mainSource.indexOf(
      "function setBrowserPanelBounds",
      mainSource.indexOf("function destroyBrowserPanel(window)"),
    ),
  );
  const disposeFunction = mainSource.slice(
    mainSource.indexOf("function disposeBrowserPanelTab(panel, tab)"),
    mainSource.indexOf(
      "function attachActiveBrowserPanelView",
      mainSource.indexOf("function disposeBrowserPanelTab(panel, tab)"),
    ),
  );

  assert.match(
    closeFunction,
    /panel\.tabs\.splice\(index, 1\);[\s\S]*disposeBrowserPanelTab\(panel, tab\);[\s\S]*if \(panel\.tabs\.length === 0 && !panel\.destroying\) \{[\s\S]*createBrowserPanelTab\(panel, \{ activate: true \}\);/,
    "closed tabs must be disposed before an automatic replacement tab can be created",
  );
  assert.match(
    destroyPanelFunction,
    /for \(const tab of \[\.\.\.panel\.tabs\]\) \{[\s\S]*disposeBrowserPanelTab\(panel, tab\);[\s\S]*\}/,
    "window teardown must dispose a tab snapshot because destroy events can mutate panel.tabs",
  );
  assert.match(
    disposeFunction,
    /if \(panel\.attachedTabId === tab\.id\) \{[\s\S]*detachAttachedBrowserPanelView\(panel\);[\s\S]*\}[\s\S]*stopBrowserPanelNavigation\(tab\);[\s\S]*tab\.view\.webContents\.destroy\(\);/,
    "Browser tab disposal must detach native views, cancel pending navigation, and destroy the WebContents",
  );
  assert.doesNotMatch(
    disposeFunction,
    /webContents\.close/,
    "closing a Browser tab must not leave WebContentsView lifetime to page close semantics",
  );
});
