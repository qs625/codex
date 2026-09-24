const test = require("node:test");
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");

const {
  closeBrowserPanelTabLifecycle,
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

test("closeBrowserPanelTabLifecycle waits for loaded tab disposal before replacement", async () => {
  const events = [];
  let replacementCounter = 0;
  let resolveDispose;
  const panel = {
    tabs: [{ id: "loaded-tab" }],
    activeTabId: "loaded-tab",
    attachedTabId: "loaded-tab",
    visible: true,
    destroying: false,
  };

  const closePromise = closeBrowserPanelTabLifecycle(panel, "loaded-tab", {
    detachAttachedView(targetPanel) {
      events.push(`detach:${targetPanel.attachedTabId}`);
      targetPanel.attachedTabId = null;
    },
    disposeTab(_targetPanel, tab) {
      events.push(`dispose-start:${tab.id}`);
      return new Promise((resolve) => {
        resolveDispose = () => {
          events.push(`dispose-done:${tab.id}`);
          resolve();
        };
      });
    },
    createTab(targetPanel, { activate }) {
      const replacement = { id: `replacement-${++replacementCounter}` };
      events.push(`create:${replacement.id}:activate=${activate}`);
      targetPanel.tabs.push(replacement);
      if (activate) {
        targetPanel.activeTabId = replacement.id;
        targetPanel.attachedTabId = replacement.id;
      }
      return replacement;
    },
    attachActiveView(targetPanel, { raise }) {
      events.push(`attach:${targetPanel.activeTabId}:raise=${raise}`);
      targetPanel.attachedTabId = targetPanel.activeTabId;
      return true;
    },
  });

  await Promise.resolve();
  assert.deepEqual(events, ["detach:loaded-tab", "dispose-start:loaded-tab"]);
  assert.deepEqual(panel.tabs, []);
  assert.equal(panel.activeTabId, null);
  assert.equal(panel.attachedTabId, null);

  resolveDispose();
  assert.equal(await closePromise, true);
  assert.deepEqual(events, [
    "detach:loaded-tab",
    "dispose-start:loaded-tab",
    "dispose-done:loaded-tab",
    "create:replacement-1:activate=true",
  ]);
  assert.deepEqual(panel.tabs, [{ id: "replacement-1" }]);
  assert.equal(panel.activeTabId, "replacement-1");
  assert.equal(panel.attachedTabId, "replacement-1");
});

test("closeBrowserPanelTabLifecycle waits for active tab disposal before attaching neighbor", async () => {
  const events = [];
  let resolveDispose;
  const panel = {
    tabs: [{ id: "loaded-tab" }, { id: "blank-tab" }],
    activeTabId: "loaded-tab",
    attachedTabId: "loaded-tab",
    visible: true,
    destroying: false,
  };

  const closePromise = closeBrowserPanelTabLifecycle(panel, "loaded-tab", {
    detachAttachedView(targetPanel) {
      events.push(`detach:${targetPanel.attachedTabId}`);
      targetPanel.attachedTabId = null;
    },
    disposeTab(_targetPanel, tab) {
      events.push(`dispose-start:${tab.id}`);
      return new Promise((resolve) => {
        resolveDispose = () => {
          events.push(`dispose-done:${tab.id}`);
          resolve();
        };
      });
    },
    createTab() {
      events.push("create-unexpected");
    },
    attachActiveView(targetPanel, { raise }) {
      events.push(`attach:${targetPanel.activeTabId}:raise=${raise}`);
      targetPanel.attachedTabId = targetPanel.activeTabId;
      return true;
    },
  });

  await Promise.resolve();
  assert.deepEqual(events, ["detach:loaded-tab", "dispose-start:loaded-tab"]);
  assert.deepEqual(panel.tabs, [{ id: "blank-tab" }]);
  assert.equal(panel.activeTabId, "blank-tab");
  assert.equal(panel.attachedTabId, null);

  resolveDispose();
  assert.equal(await closePromise, true);
  assert.deepEqual(events, [
    "detach:loaded-tab",
    "dispose-start:loaded-tab",
    "dispose-done:loaded-tab",
    "attach:blank-tab:raise=true",
  ]);
  assert.equal(panel.activeTabId, "blank-tab");
  assert.equal(panel.attachedTabId, "blank-tab");
});

test("closeBrowserPanelTabLifecycle passes the active closing tab to detach", async () => {
  const detached = [];
  const panel = {
    tabs: [{ id: "tab-a" }, { id: "tab-b" }],
    activeTabId: "tab-a",
    attachedTabId: "tab-a",
    visible: true,
    destroying: false,
  };

  assert.equal(
    await closeBrowserPanelTabLifecycle(panel, "tab-a", {
      detachAttachedView: (_targetPanel, tab) => detached.push(tab.id),
      disposeTab: async () => {},
      createTab: () => {},
      attachActiveView: () => {},
    }),
    true,
  );

  assert.deepEqual(detached, ["tab-a"]);
});

test("closeBrowserPanelTabLifecycle leaves the active visible tab alone when closing background", async () => {
  const events = [];
  let resolveDispose;
  const panel = {
    tabs: [{ id: "active-tab" }, { id: "background-tab" }],
    activeTabId: "active-tab",
    attachedTabId: "active-tab",
    visible: true,
    destroying: false,
  };

  const closePromise = closeBrowserPanelTabLifecycle(panel, "background-tab", {
    detachAttachedView() {
      events.push("detach-unexpected");
    },
    disposeTab(_targetPanel, tab) {
      events.push(`dispose-start:${tab.id}`);
      return new Promise((resolve) => {
        resolveDispose = () => {
          events.push(`dispose-done:${tab.id}`);
          resolve();
        };
      });
    },
    createTab() {
      events.push("create-unexpected");
    },
    attachActiveView() {
      events.push("attach-unexpected");
    },
  });

  await Promise.resolve();
  assert.deepEqual(events, ["dispose-start:background-tab"]);
  assert.deepEqual(panel.tabs, [{ id: "active-tab" }]);
  assert.equal(panel.activeTabId, "active-tab");
  assert.equal(panel.attachedTabId, "active-tab");

  resolveDispose();
  assert.equal(await closePromise, true);
  assert.deepEqual(events, [
    "dispose-start:background-tab",
    "dispose-done:background-tab",
  ]);
  assert.equal(panel.activeTabId, "active-tab");
  assert.equal(panel.attachedTabId, "active-tab");
});

test("browser panel native view lifecycle raises only on explicit show or tab actions", () => {
  const mainSource = readFileSync(join(__dirname, "main.cjs"), "utf8");
  const surfaceSource = readFileSync(join(__dirname, "browserPanelSurface.cjs"), "utf8");

  assert.match(
    mainSource,
    /ipcMain\.handle\("codex:browser:show"[\s\S]*const surfaceId = browserSurfaceIdFromPayload\(bounds\);[\s\S]*const tabId = browserTabIdFromPayload\(bounds\);[\s\S]*setBrowserPanelBounds\(panel, browserBoundsFromPayload\(bounds\), \{[\s\S]*surfaceId,[\s\S]*tabId,[\s\S]*\}\);[\s\S]*attachBrowserPanel\(panel, \{[\s\S]*surfaceId,[\s\S]*tabId,[\s\S]*\}\);/,
  );
  assert.match(
    surfaceSource,
    /visibleSurfaceIds: new Set\(\)/,
  );
  assert.match(
    surfaceSource,
    /boundsBySurfaceId: new Map\(\[[\s\S]*DEFAULT_BROWSER_SURFACE_ID[\s\S]*initialBoundsUpdate\.bounds/,
  );
  assert.match(
    surfaceSource,
    /attachedTabIdBySurfaceId: new Map\(\)/,
  );
  assert.match(
    mainSource,
    /function attachBrowserPanel\([\s\S]*surfaceId = DEFAULT_BROWSER_SURFACE_ID[\s\S]*showBrowserPanelSurface\(panel, surfaceId\);[\s\S]*attachBrowserPanelTabView\(panel, \{[\s\S]*surfaceId,[\s\S]*tab:[\s\S]*tabId[\s\S]*activeBrowserPanelTab\(panel\),[\s\S]*raise,[\s\S]*\}\);/,
  );
  assert.match(
    mainSource,
    /function detachBrowserPanel\(panel, \{ surfaceId = null \} = \{\}\) \{[\s\S]*hideBrowserPanelSurface\(panel, surfaceId\);[\s\S]*detachAttachedBrowserPanelView\(panel, \{ surfaceId \}\);[\s\S]*detachAllBrowserPanelViews\(panel\);/,
  );
  assert.match(
    mainSource,
    /function setBrowserPanelBounds\([\s\S]*surfaceId = DEFAULT_BROWSER_SURFACE_ID[\s\S]*tabId = null[\s\S]*panel\.boundsBySurfaceId\.set\(surfaceId, update\.bounds\);[\s\S]*panel\.visibleSurfaceIds\.has\(surfaceId\)[\s\S]*attachBrowserPanelTabView\(panel, \{ surfaceId, tab \}\);/,
  );
  assert.doesNotMatch(
    mainSource.slice(
      mainSource.indexOf("function setBrowserPanelBounds("),
      mainSource.indexOf("function sendBrowserPanelState(panel)", mainSource.indexOf("function setBrowserPanelBounds(")),
    ),
    /attachActiveBrowserPanelView\(panel, \{ raise: true \}\)/,
    "passive Browser bounds refresh must not raise the native view and steal focus",
  );
  assert.match(
    mainSource,
    /function attachActiveBrowserPanelView\([\s\S]*surfaceId = DEFAULT_BROWSER_SURFACE_ID/,
  );
  const attachFunction = mainSource.slice(
    mainSource.indexOf("function attachBrowserPanelTabView("),
    mainSource.indexOf("function ensureBrowserPanelTabAttachedForNavigation", mainSource.indexOf("function attachBrowserPanelTabView(")),
  );
  assert.match(
    attachFunction,
    /const bounds = browserPanelBoundsForSurface\(panel, surfaceId\);[\s\S]*const boundsVisible = browserPanelBoundsAreVisible\(bounds\);[\s\S]*shouldAttachBrowserPanelView\(\{[\s\S]*boundsVisible,[\s\S]*panelVisible: panel\.visibleSurfaceIds\.has\(surfaceId\),[\s\S]*\}\)[\s\S]*panel\.window\.contentView\.addChildView\(tab\.view\);[\s\S]*rememberBrowserPanelAttachedTab\(panel, surfaceId, tab\.id\);/,
    "native Browser views must not attach before panel bounds are visible",
  );
  assert.match(
    attachFunction,
    /if \(!boundsVisible\) \{[\s\S]*detachAttachedBrowserPanelView\(panel, \{ surfaceId \}\);[\s\S]*\}/,
    "native Browser views must detach when bounds stop being visible",
  );
  assert.match(
    attachFunction,
    /isBrowserPanelTabAlreadyAttached\(\{[\s\S]*attachedTabId: panel\.attachedTabIdBySurfaceId\.get\(surfaceId\) \?\? null,[\s\S]*tabId: tab\.id,[\s\S]*\}\)[\s\S]*tab\.view\.setBounds\(bounds\);[\s\S]*return true;/,
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
    /function ensureBrowserPanelTabAttachedForNavigation\([\s\S]*surfaceId = DEFAULT_BROWSER_SURFACE_ID[\s\S]*attachBrowserPanelTabView\(panel, \{ surfaceId, tab, raise \}\)/,
  );
  assert.match(
    mainSource,
    /function ensureBrowserPanelTabAttachedForNavigation\([\s\S]*if \(!panel\.visibleSurfaceIds\.has\(surfaceId\)\) \{[\s\S]*throw new Error\("Browser page is not visible in the panel"\);/,
  );
  assert.match(
    mainSource,
    /function ensureBrowserPanelTabAttachedForNavigation\([\s\S]*browserPanelBoundsAreVisible\(browserPanelBoundsForSurface\(panel, surfaceId\)\)[\s\S]*throw new Error\("Browser page has no visible panel bounds"\);/,
  );
  const visibleNavigationTargetFunction = mainSource.slice(
    mainSource.indexOf("async function waitForBrowserPanelVisibleNavigationTarget("),
    mainSource.indexOf("async function waitForBrowserPanelNavigationTarget", mainSource.indexOf("async function waitForBrowserPanelVisibleNavigationTarget(")),
  );
  assert.match(
    visibleNavigationTargetFunction,
    /if \(!panel\.visibleSurfaceIds\.has\(surfaceId\)\) \{[\s\S]*throw new Error\("Browser page is not visible in the panel"\);[\s\S]*if \(!browserPanelBoundsAreVisible\(browserPanelBoundsForSurface\(panel, surfaceId\)\)\) \{[\s\S]*throw new Error\("Browser page has no visible panel bounds"\);[\s\S]*ensureBrowserPanelTabAttachedForNavigation\(panel, tab, \{[\s\S]*raise: true,[\s\S]*surfaceId,[\s\S]*\}\);/,
    "visible Browser navigation must still require a visible attached native view",
  );
  assert.match(
    visibleNavigationTargetFunction,
    /try \{[\s\S]*await waitForBrowserPanelDevToolsTarget\(tab\.view\.webContents\);[\s\S]*\} catch \(error\) \{[\s\S]*console\.warn\([\s\S]*"Browser panel DevTools target was not published for visible navigation"[\s\S]*error,[\s\S]*\);[\s\S]*\}/,
    "visible Browser navigation must not fail user page display when DevTools target publication is unavailable",
  );
  assert.match(
    mainSource,
    /async function loadBrowserPanelTabUrl\([\s\S]*\{ requireVisiblePanel = true, surfaceId = DEFAULT_BROWSER_SURFACE_ID \} = \{\},[\s\S]*tab\.pendingNavigationSurfaceId = surfaceId;[\s\S]*stopBrowserPanelWebContentsLoad\(tab\);[\s\S]*if \(requireVisiblePanel\) \{[\s\S]*ensureBrowserPanelTabAttachedForNavigation\(panel, tab, \{[\s\S]*raise: true,[\s\S]*surfaceId,[\s\S]*\}\);[\s\S]*tab\.view\.webContents\.loadURL\(normalized\.url\)[\s\S]*await waitForBrowserPanelNavigationTarget\(panel, tab, \{[\s\S]*requireVisiblePanel,[\s\S]*surfaceId,[\s\S]*\}\);[\s\S]*completeBrowserPanelNavigation\(panel, tab, navigationSequence\);/,
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
    /ipcMain\.handle\("codex:browser:reload"[\s\S]*tab\.pendingNavigationSequence = navigationSequence;[\s\S]*tab\.pendingNavigationRequiresVisiblePanel = true;[\s\S]*tab\.pendingNavigationSurfaceId = browserSurfaceIdFromPayload\(options\);[\s\S]*tab\.pendingNavigationTarget = tab\.view\.webContents\.getURL\(\) \|\| null;/,
  );
  assert.match(
    mainSource,
    /tab\.view\.webContents\.on\("did-start-loading", \(\) => \{[\s\S]*tab\.pendingNavigationSequence = navigationSequence;[\s\S]*tab\.pendingNavigationRequiresVisiblePanel = true;[\s\S]*tab\.pendingNavigationSurfaceId =[\s\S]*attachedTabId\]\) => attachedTabId === tab\.id,[\s\S]*tab\.pendingNavigationTarget = currentUrl \|\| null;/,
  );
  assert.match(
    mainSource,
    /function stopBrowserPanelNavigation\(tab\) \{[\s\S]*tab\.pendingNavigationSequence = null;[\s\S]*tab\.pendingNavigationRequiresVisiblePanel = true;[\s\S]*tab\.pendingNavigationSurfaceId = DEFAULT_BROWSER_SURFACE_ID;[\s\S]*tab\.pendingNavigationTarget = null;/,
  );
  assert.match(mainSource, /function detachBrowserPanelTabFromAllSurfaces\(panel, tab\)/);
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
    /async function waitForBrowserPanelNavigationTarget\([\s\S]*surfaceId = DEFAULT_BROWSER_SURFACE_ID,[\s\S]*if \(requireVisiblePanel\) \{[\s\S]*await waitForBrowserPanelVisibleNavigationTarget\(panel, tab, surfaceId\);[\s\S]*return;[\s\S]*\}[\s\S]*await waitForBrowserPanelDevToolsTarget\(tab\.view\.webContents\);[\s\S]*\}/,
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
  const destroyTabWebContentsFunction = mainSource.slice(
    mainSource.indexOf("function destroyBrowserPanelTabWebContents(tab)"),
    mainSource.indexOf(
      "function attachActiveBrowserPanelView",
      mainSource.indexOf("function destroyBrowserPanelTabWebContents(tab)"),
    ),
  );

  assert.match(
    mainSource,
    /ipcMain\.handle\("codex:browser:closeTab"[\s\S]*await closeBrowserPanelTab\(panel, tabId\)/,
    "close-tab IPC must wait for old WebContents teardown before returning replacement state",
  );
  assert.match(
    mainSource,
    /async function closeBrowserPanelTab\(panel, tabId\)/,
    "closeBrowserPanelTab must be async so callers can await tab disposal",
  );
  assert.match(
    closeFunction,
    /return closeBrowserPanelTabLifecycle\(panel, tabId, \{[\s\S]*detachAttachedView: detachBrowserPanelTabFromAllSurfaces,[\s\S]*disposeTab: disposeBrowserPanelTab,[\s\S]*createTab: createBrowserPanelTab,[\s\S]*attachActiveView: attachActiveBrowserPanelView,/,
    "closed tabs must use the async lifecycle helper so replacement/selection waits for disposal",
  );
  assert.match(
    destroyPanelFunction,
    /for \(const tab of \[\.\.\.panel\.tabs\]\) \{[\s\S]*void disposeBrowserPanelTab\(panel, tab\)\.catch\(\(error\) => \{[\s\S]*console\.warn\(/,
    "window teardown must dispose a tab snapshot with a rejection handler because destroy events can mutate panel.tabs",
  );
  assert.match(
    destroyPanelFunction,
    /failed to dispose Browser tab during window teardown/,
    "window teardown disposal failures must leave bounded diagnostic evidence",
  );
  assert.match(
    disposeFunction,
    /detachBrowserPanelTabFromAllSurfaces\(panel, tab\);[\s\S]*stopBrowserPanelNavigation\(tab\);[\s\S]*await destroyBrowserPanelTabWebContents\(tab\);/,
    "Browser tab disposal must detach native views, cancel pending navigation, and await WebContents destruction",
  );
  assert.match(
    destroyTabWebContentsFunction,
    /webContents\.once\("destroyed", cleanup\);[\s\S]*webContents\.destroy\(\);[\s\S]*if \(webContents\.isDestroyed\(\)\) \{[\s\S]*cleanup\(\);[\s\S]*\}/,
    "Browser tab disposal must resolve only after the old WebContents has reached destroyed state",
  );
  assert.doesNotMatch(
    disposeFunction,
    /webContents\.close/,
    "closing a Browser tab must not leave WebContentsView lifetime to page close semantics",
  );
});
