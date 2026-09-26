const assert = require("node:assert/strict");
const test = require("node:test");
const {
  readSource,
  sourceIndex,
  sourceSlice,
} = require("./sourceAssertions.cjs");

const {
  browserPanelLoadErrorMessage,
  browserPanelNavigationTimeoutMessage,
  browserPanelUrlIsSafeCommittedHttpUrl,
  browserPanelUrlsEqual,
  shouldAcceptBrowserPanelCommittedNavigation,
  shouldCompleteBrowserPanelStoppedNavigation,
  shouldCompleteBrowserPanelTimedOutNavigation,
  shouldCompleteRejectedBrowserPanelNavigation,
  shouldDeferBrowserPanelFailure,
  shouldExposeBrowserPanelLoading,
  shouldStopBrowserPanelLoadBeforeNavigation,
  waitForBrowserPanelNavigationResult,
} = require("./browserPanelNavigationState.cjs");

const mainSource = readSource("main.cjs");

test("browserPanelUrlsEqual compares normalized URL hrefs", () => {
  assert.equal(
    browserPanelUrlsEqual("https://example.com", "https://example.com/"),
    true,
  );
  assert.equal(
    browserPanelUrlsEqual("https://example.com/docs", "https://example.com/"),
    false,
  );
});

test("shouldDeferBrowserPanelFailure defers in-flight ERR_FAILED before target commits", () => {
  assert.equal(
    shouldDeferBrowserPanelFailure({
      errorCode: -2,
      validatedUrl: "https://example.com/",
      loading: true,
      currentUrl: "about:blank",
    }),
    true,
  );
});

test("shouldDeferBrowserPanelFailure keeps real failures immediate once target is current", () => {
  assert.equal(
    shouldDeferBrowserPanelFailure({
      errorCode: -2,
      validatedUrl: "https://example.com/",
      loading: true,
      currentUrl: "https://example.com/",
    }),
    false,
  );
});

test("shouldDeferBrowserPanelFailure does not defer non-transient failures", () => {
  assert.equal(
    shouldDeferBrowserPanelFailure({
      errorCode: -105,
      validatedUrl: "https://offline.invalid/",
      loading: true,
      currentUrl: "about:blank",
    }),
    false,
  );
});

test("shouldCompleteRejectedBrowserPanelNavigation requires finish evidence", () => {
  assert.equal(
    shouldCompleteRejectedBrowserPanelNavigation({
      navigationSequence: 4,
      finishedNavigationSequence: 0,
      finishedUrl: null,
      currentUrl: "https://example.com/",
      targetUrl: "https://example.com/",
    }),
    false,
  );
});

test("shouldCompleteRejectedBrowserPanelNavigation accepts matching finished target", () => {
  assert.equal(
    shouldCompleteRejectedBrowserPanelNavigation({
      navigationSequence: 4,
      finishedNavigationSequence: 4,
      finishedUrl: "https://example.com/",
      currentUrl: "https://example.com",
      targetUrl: "https://example.com/",
    }),
    true,
  );
});

test("shouldCompleteRejectedBrowserPanelNavigation rejects stale finish evidence", () => {
  assert.equal(
    shouldCompleteRejectedBrowserPanelNavigation({
      navigationSequence: 4,
      finishedNavigationSequence: 3,
      finishedUrl: "https://example.com/",
      currentUrl: "https://example.com/",
      targetUrl: "https://example.com/",
    }),
    false,
  );
});

test("shouldCompleteRejectedBrowserPanelNavigation rejects same-sequence non-target finish", () => {
  assert.equal(
    shouldCompleteRejectedBrowserPanelNavigation({
      navigationSequence: 4,
      finishedNavigationSequence: 4,
      finishedUrl: "https://previous.example/",
      currentUrl: "https://example.com/",
      targetUrl: "https://example.com/",
    }),
    false,
  );
});

test("shouldCompleteBrowserPanelStoppedNavigation accepts matching stopped target", () => {
  assert.equal(
    shouldCompleteBrowserPanelStoppedNavigation({
      navigationSequence: 4,
      pendingNavigationSequence: 4,
      currentUrl: "https://example.com",
      targetUrl: "https://example.com/",
    }),
    true,
  );
});

test("shouldCompleteBrowserPanelStoppedNavigation rejects stale or non-target stop evidence", () => {
  assert.equal(
    shouldCompleteBrowserPanelStoppedNavigation({
      navigationSequence: 4,
      pendingNavigationSequence: 3,
      currentUrl: "https://example.com/",
      targetUrl: "https://example.com/",
    }),
    false,
  );
  assert.equal(
    shouldCompleteBrowserPanelStoppedNavigation({
      navigationSequence: 4,
      pendingNavigationSequence: 4,
      currentUrl: "https://previous.example/",
      targetUrl: "https://example.com/",
    }),
    false,
  );
});

test("shouldCompleteBrowserPanelTimedOutNavigation accepts committed target URL", () => {
  assert.equal(
    shouldCompleteBrowserPanelTimedOutNavigation({
      currentUrl: "https://example.com",
      startUrl: "about:blank",
      targetUrl: "https://example.com/",
    }),
    true,
  );
});

test("shouldCompleteBrowserPanelTimedOutNavigation accepts safe committed redirects", () => {
  assert.equal(
    shouldCompleteBrowserPanelTimedOutNavigation({
      currentUrl: "https://www.baidu.com/",
      startUrl: "about:blank",
      targetUrl: "https://baidu.com/",
    }),
    true,
  );
});

test("shouldCompleteBrowserPanelTimedOutNavigation requires start evidence for redirects", () => {
  assert.equal(
    shouldCompleteBrowserPanelTimedOutNavigation({
      currentUrl: "https://www.baidu.com/",
      targetUrl: "https://baidu.com/",
    }),
    false,
  );
  assert.equal(
    shouldCompleteBrowserPanelTimedOutNavigation({
      currentUrl: "https://example.com/",
      targetUrl: "https://example.com/",
    }),
    true,
  );
});

test("shouldCompleteBrowserPanelTimedOutNavigation rejects non-committed timeout state", () => {
  assert.equal(
    shouldCompleteBrowserPanelTimedOutNavigation({
      currentUrl: "about:blank",
      startUrl: "about:blank",
      targetUrl: "https://example.com/",
    }),
    false,
  );
  assert.equal(
    shouldCompleteBrowserPanelTimedOutNavigation({
      currentUrl: "https://previous.example/",
      startUrl: "https://previous.example/",
      targetUrl: "https://example.com/",
    }),
    false,
  );
});

test("shouldCompleteBrowserPanelTimedOutNavigation rejects unsafe timeout URLs", () => {
  for (const currentUrl of [
    "file:///tmp/index.html",
    "data:text/html,hello",
    "custom-scheme:foo",
  ]) {
    assert.equal(
      shouldCompleteBrowserPanelTimedOutNavigation({
        currentUrl,
        startUrl: "about:blank",
        targetUrl: "https://example.com/",
      }),
      false,
    );
  }
  assert.equal(
    shouldCompleteBrowserPanelTimedOutNavigation({
      currentUrl: "https://example.com/",
      startUrl: "about:blank",
      targetUrl: "file:///tmp/index.html",
    }),
    false,
  );
});

test("browserPanelUrlIsSafeCommittedHttpUrl accepts only committed http URLs", () => {
  assert.equal(browserPanelUrlIsSafeCommittedHttpUrl("https://example.com/"), true);
  assert.equal(browserPanelUrlIsSafeCommittedHttpUrl("http://localhost:5173/"), true);
  assert.equal(browserPanelUrlIsSafeCommittedHttpUrl("about:blank"), false);
  assert.equal(browserPanelUrlIsSafeCommittedHttpUrl("file:///tmp/index.html"), false);
  assert.equal(browserPanelUrlIsSafeCommittedHttpUrl("custom-scheme:foo"), false);
});

test("shouldAcceptBrowserPanelCommittedNavigation accepts redirected committed URLs", () => {
  assert.equal(
    shouldAcceptBrowserPanelCommittedNavigation({
      navigationSequence: 4,
      pendingNavigationSequence: 4,
      navigationStarted: true,
      currentUrl: "https://www.baidu.com/",
    }),
    true,
  );
});

test("shouldAcceptBrowserPanelCommittedNavigation requires current started navigation", () => {
  assert.equal(
    shouldAcceptBrowserPanelCommittedNavigation({
      navigationSequence: 4,
      pendingNavigationSequence: 3,
      navigationStarted: true,
      currentUrl: "https://www.baidu.com/",
    }),
    false,
  );
  assert.equal(
    shouldAcceptBrowserPanelCommittedNavigation({
      navigationSequence: 4,
      pendingNavigationSequence: 4,
      navigationStarted: false,
      currentUrl: "https://www.baidu.com/",
    }),
    false,
  );
  assert.equal(
    shouldAcceptBrowserPanelCommittedNavigation({
      navigationSequence: 4,
      pendingNavigationSequence: 4,
      navigationStarted: true,
      currentUrl: "about:blank",
    }),
    false,
  );
  assert.equal(
    shouldAcceptBrowserPanelCommittedNavigation({
      navigationSequence: 4,
      pendingNavigationSequence: 4,
      navigationStarted: true,
      committedUrl: "https://www.baidu.com/",
      currentUrl: "https://previous.example/",
    }),
    false,
  );
  assert.equal(
    shouldAcceptBrowserPanelCommittedNavigation({
      navigationSequence: 4,
      pendingNavigationSequence: 4,
      navigationStarted: true,
      committedUrl: "https://www.baidu.com/",
      currentUrl: "https://www.baidu.com",
    }),
    true,
  );
});

test("browserPanelLoadErrorMessage includes Electron error code when present", () => {
  assert.equal(
    browserPanelLoadErrorMessage({
      errorCode: -105,
      errorDescription: "ERR_NAME_NOT_RESOLVED",
    }),
    "ERR_NAME_NOT_RESOLVED (-105)",
  );
});

test("browserPanelNavigationTimeoutMessage rounds timeout seconds", () => {
  assert.equal(
    browserPanelNavigationTimeoutMessage(15_000),
    "Browser navigation timed out after 15s",
  );
});

test("shouldExposeBrowserPanelLoading hides initial blank webContents loading", () => {
  assert.equal(
    shouldExposeBrowserPanelLoading({
      observedLoading: true,
      pendingNavigationSequence: null,
    }),
    false,
  );
});

test("shouldExposeBrowserPanelLoading shows active navigation loading", () => {
  assert.equal(
    shouldExposeBrowserPanelLoading({
      observedLoading: true,
      pendingNavigationSequence: 7,
    }),
    true,
  );
});

test("shouldStopBrowserPanelLoadBeforeNavigation skips cold blank bootstrap loads", () => {
  assert.equal(
    shouldStopBrowserPanelLoadBeforeNavigation({ startUrl: null }),
    false,
  );
  assert.equal(
    shouldStopBrowserPanelLoadBeforeNavigation({ startUrl: "" }),
    false,
  );
  assert.equal(
    shouldStopBrowserPanelLoadBeforeNavigation({ startUrl: "about:blank" }),
    false,
  );
  assert.equal(
    shouldStopBrowserPanelLoadBeforeNavigation({
      startUrl: "https://example.com/",
    }),
    true,
  );
});

test("waitForBrowserPanelNavigationResult rejects hung loadURL with bounded timeout", async () => {
  const timers = {
    setTimeout(callback) {
      callback();
      return 1;
    },
    clearTimeout() {},
  };

  await assert.rejects(
    waitForBrowserPanelNavigationResult(new Promise(() => {}), 3_000, timers),
    (error) => {
      assert.equal(error.code, "ERR_BROWSER_PANEL_NAVIGATION_TIMEOUT");
      assert.equal(error.message, "Browser navigation timed out after 3s");
      return true;
    },
  );
});

test("waitForBrowserPanelNavigationResult resolves completed loadURL before timeout", async () => {
  let timeoutScheduled = false;
  const timers = {
    setTimeout() {
      timeoutScheduled = true;
      return 1;
    },
    clearTimeout() {},
  };

  await waitForBrowserPanelNavigationResult(Promise.resolve(), 3_000, timers);
  assert.equal(timeoutScheduled, true);
});

test("waitForBrowserPanelNavigationResult resolves observed target navigation when loadURL hangs", async () => {
  let timeoutCallback = null;
  let clearedTimeout = null;
  const timers = {
    setTimeout(callback) {
      timeoutCallback = callback;
      return 7;
    },
    clearTimeout(timeout) {
      clearedTimeout = timeout;
    },
  };

  await waitForBrowserPanelNavigationResult(
    new Promise(() => {}),
    3_000,
    timers,
    Promise.resolve(),
  );

  assert.equal(typeof timeoutCallback, "function");
  assert.equal(clearedTimeout, 7);
});

test("waitForBrowserPanelNavigationResult rejects observed navigation failure before timeout", async () => {
  const timers = {
    setTimeout() {
      return 1;
    },
    clearTimeout() {},
  };

  await assert.rejects(
    waitForBrowserPanelNavigationResult(
      new Promise(() => {}),
      3_000,
      timers,
      Promise.reject(new Error("ERR_NAME_NOT_RESOLVED (-105)")),
    ),
    /ERR_NAME_NOT_RESOLVED \(-105\)/,
  );
});

test("main browser navigation gates stop completion on committed URL evidence", () => {
  const observerIndex = sourceIndex(mainSource, "function observeBrowserPanelTargetNavigation");
  const stopHandlerIndex = sourceIndex(mainSource, "const handleStop = () => {", observerIndex);
  sourceIndex(
    mainSource,
    'webContents.on("did-stop-loading", handleStop);',
    observerIndex,
  );
  const globalStopIndex = sourceIndex(
    mainSource,
    'tab.view.webContents.on("did-stop-loading", () => {',
  );

  sourceIndex(mainSource, "committedUrlForSequence", stopHandlerIndex);
  sourceIndex(mainSource, "committedUrl: committedUrlForSequence", stopHandlerIndex);
  sourceIndex(mainSource, "completeBrowserPanelNavigationWhenTargetReady", globalStopIndex);
});

test("main browser navigation observes committed redirects before full load", () => {
  const observerIndex = sourceIndex(mainSource, "function observeBrowserPanelTargetNavigation");
  sourceIndex(
    mainSource,
    'webContents.on("did-start-navigation", handleStart);',
    observerIndex,
  );
  sourceIndex(
    mainSource,
    'webContents.on("did-frame-navigate", handleFrameNavigate);',
    observerIndex,
  );
  sourceIndex(
    mainSource,
    "shouldAcceptBrowserPanelCommittedNavigation",
    observerIndex,
  );

  sourceIndex(mainSource, "browserNavigationDecision(url)", observerIndex);
  sourceIndex(mainSource, "let committedUrlForSequence = null;", observerIndex);
  sourceIndex(mainSource, "committedUrl: committedUrlForSequence", observerIndex);
});

test("main browser navigation completes timed out visible load when target committed", () => {
  const timeoutCatchIndex = sourceIndex(
    mainSource,
    "if (isBrowserPanelNavigationTimeoutError(error))",
  );
  const committedTargetIndex = sourceIndex(
    mainSource,
    "browserPanelTabHasCommittedTarget(tab, normalized.url)",
    timeoutCatchIndex,
  );
  sourceIndex(
    mainSource,
    "await waitForBrowserPanelNavigationTarget(panel, tab",
    committedTargetIndex,
  );
  sourceIndex(
    mainSource,
    "completeBrowserPanelNavigation(panel, tab, navigationSequence)",
    committedTargetIndex,
  );
  const stopLoadIndex = sourceIndex(
    mainSource,
    "stopBrowserPanelWebContentsLoad(tab);",
    timeoutCatchIndex,
  );

  assert.ok(
    committedTargetIndex < stopLoadIndex,
    "timeout handling must check committed native URL before stopping the BrowserView load",
  );
});

test("main browser navigation completes stopped redirected loads before timeout", () => {
  const stopHandlerIndex = sourceIndex(
    mainSource,
    'tab.view.webContents.on("did-stop-loading", () => {',
  );
  const stoppedAcceptedIndex = sourceIndex(
    mainSource,
    "browserPanelTabHasStoppedAtAcceptedUrl(tab)",
    stopHandlerIndex,
  );
  sourceIndex(
    mainSource,
    "completeBrowserPanelNavigationWhenTargetReady",
    stoppedAcceptedIndex,
  );
  const stoppedHelperIndex = sourceIndex(
    mainSource,
    "function browserPanelTabHasStoppedAtAcceptedUrl",
  );

  assert.match(
    mainSource.slice(
      stoppedHelperIndex,
      sourceIndex(mainSource, "function browserPanelTabHasCommittedTarget", stoppedHelperIndex),
    ),
    /shouldCompleteBrowserPanelStoppedNavigation[\s\S]*browserPanelTabHasCommittedTarget\(tab, tab\.pendingNavigationTarget\)/,
  );
});

test("main browser navigation observes frame finish when loadURL hangs", () => {
  const bindIndex = sourceIndex(mainSource, "function bindBrowserPanelTab");
  const observerIndex = sourceIndex(mainSource, "function observeBrowserPanelTargetNavigation");
  sourceIndex(
    mainSource,
    'tab.view.webContents.on("did-frame-finish-load"',
    bindIndex,
  );
  sourceIndex(
    mainSource,
    'webContents.on("did-frame-finish-load", handleFrameFinish);',
    observerIndex,
  );
  const acceptedCurrentIndex = sourceIndex(
    mainSource,
    "const acceptedCurrentCommittedUrl = () =>",
    observerIndex,
  );

  assert.match(
    mainSource.slice(acceptedCurrentIndex, sourceIndex(mainSource, "const handleStart", acceptedCurrentIndex)),
    /navigationStarted[\s\S]*shouldCompleteBrowserPanelTimedOutNavigation\(\{[\s\S]*startUrl: tab\.pendingNavigationStartUrl,[\s\S]*targetUrl: tab\.pendingNavigationTarget,/,
  );
});

test("main browser navigation records start URL for timeout fallback", () => {
  const loadUrlIndex = sourceIndex(mainSource, "async function loadBrowserPanelTabUrl");
  const timeoutTargetIndex = sourceIndex(
    mainSource,
    "function browserPanelTabHasCommittedTarget",
  );

  sourceIndex(mainSource, "pendingNavigationStartUrl: null");
  sourceIndex(mainSource, "const navigationStartUrl =", loadUrlIndex);
  sourceIndex(mainSource, "tab.pendingNavigationStartUrl = navigationStartUrl", loadUrlIndex);
  sourceIndex(mainSource, "startUrl: tab.pendingNavigationStartUrl", timeoutTargetIndex);
});

test("main browser navigation does not stop cold blank bootstrap before external load", () => {
  const loadUrlSource = sourceSlice(
    mainSource,
    "async function loadBrowserPanelTabUrl",
    "function bindBrowserPanelTab",
  );
  const navigationStartUrlIndex = sourceIndex(loadUrlSource, "const navigationStartUrl =");
  const stopDecisionIndex = sourceIndex(
    loadUrlSource,
    "shouldStopBrowserPanelLoadBeforeNavigation({ startUrl: navigationStartUrl })",
  );
  const stopCallIndex = sourceIndex(loadUrlSource, "stopBrowserPanelWebContentsLoad(tab)");
  const externalLoadIndex = sourceIndex(
    loadUrlSource,
    "tab.view.webContents.loadURL(normalized.url)",
  );

  assert.ok(
    navigationStartUrlIndex < stopDecisionIndex &&
      stopDecisionIndex < stopCallIndex &&
      stopCallIndex < externalLoadIndex,
    "existing loads may be stopped only after start URL is captured and before the external load",
  );
  assert.match(
    loadUrlSource,
    /if \(shouldStopBrowserPanelLoadBeforeNavigation\(\{ startUrl: navigationStartUrl \}\)\) \{[\s\S]*stopBrowserPanelWebContentsLoad\(tab\);[\s\S]*\}/,
    "cold or replacement about:blank tabs must not unconditionally stop their bootstrap load before external navigation",
  );
});

test("main browser navigation imports committed URL security decision helper", () => {
  const securityImportIndex = sourceIndex(
    mainSource,
    '} = require("./browserPanelSecurity.cjs");',
  );
  const securityImportSource = mainSource.slice(0, securityImportIndex);

  assert.match(securityImportSource, /\bbrowserNavigationDecision,\n/);
  assert.match(mainSource, /browserNavigationDecision\(url\)/);
});

test("main browser target wait maps devtools target id instead of page type", () => {
  const targetWaitSource = sourceSlice(
    mainSource,
    "async function waitForBrowserPanelDevToolsTarget",
    "async function waitForBrowserPanelVisibleNavigationTarget",
  );

  assert.match(targetWaitSource, /fromDevToolsTargetId\(candidate\.id\) === webContents/);
  assert.doesNotMatch(targetWaitSource, /candidate\.type !== "page"/);
});

test("main browser debug target creation starts hidden tab navigation before target publication", () => {
  const createTargetSource = sourceSlice(
    mainSource,
    "async function createBrowserPanelDebugTarget",
    "async function loadBrowserPanelTabAboutBlankBootstrap",
  );

  assert.match(
    createTargetSource,
    /void loadBrowserPanelTabUrl\(panel, tab, targetRequest\.url, \{[\s\S]*requireVisiblePanel: false,[\s\S]*\}\)\.catch/,
  );
  assert.doesNotMatch(
    createTargetSource,
    /await loadBrowserPanelTabUrl\(panel, tab, targetRequest\.url/,
  );
  assert.match(
    createTargetSource,
    /const targetId = await waitForBrowserPanelDevToolsTarget\(tab\.view\.webContents\);/,
  );
});

test("main browser navigation does not about:blank bootstrap visible target loads", () => {
  const createTabIndex = sourceIndex(mainSource, "function createBrowserPanelTab");
  const bootstrapIndex = sourceIndex(
    mainSource,
    "async function ensureBrowserPanelTabInitialNavigationBootstrap",
  );
  const loadUrlIndex = sourceIndex(mainSource, "async function loadBrowserPanelTabUrl");
  const normalizedOkIndex = sourceIndex(mainSource, "if (!normalized.ok)", loadUrlIndex);
  const navigationSequenceIndex = sourceIndex(
    mainSource,
    "const navigationSequence = ++tab.navigationSequence",
    loadUrlIndex,
  );
  const externalLoadIndex = sourceIndex(
    mainSource,
    "tab.view.webContents.loadURL(normalized.url)",
    loadUrlIndex,
  );

  assert.match(
    mainSource.slice(createTabIndex, loadUrlIndex),
    /initialNavigationBootstrapped: false/,
    "new Browser tabs must track cold WebContentsView bootstrap state",
  );
  assert.match(
    mainSource.slice(bootstrapIndex, loadUrlIndex),
    /loadBrowserPanelTabAboutBlankBootstrap\(panel, tab\)/,
    "hidden CDP-created Browser tabs still use about:blank bootstrap when needed",
  );
  assert.match(
    mainSource.slice(
      sourceIndex(mainSource, "async function loadBrowserPanelTabAboutBlankBootstrap"),
      bootstrapIndex,
    ),
    /tab\.initialNavigationBootstrapped = true;/,
    "about:blank bootstrap completion must be recorded",
  );
  assert.match(
    mainSource.slice(
      sourceIndex(mainSource, "async function loadBrowserPanelTabAboutBlankBootstrap"),
      bootstrapIndex,
    ),
    /await waitForBrowserPanelLoadStop\(tab\.view\.webContents\);[\s\S]*tab\.initialNavigationBootstrapped = true;/,
    "about:blank bootstrap must wait for Electron loading to stop before marking it complete",
  );
  const hiddenBootstrapIndex = sourceIndex(
    mainSource,
    "if (requireVisiblePanel === false)",
    loadUrlIndex,
  );
  const bootstrapCallIndex = sourceIndex(
    mainSource,
    "await ensureBrowserPanelTabInitialNavigationBootstrap(panel, tab);",
    hiddenBootstrapIndex,
  );
  assert.ok(
    normalizedOkIndex < hiddenBootstrapIndex &&
      hiddenBootstrapIndex < bootstrapCallIndex &&
      bootstrapCallIndex < navigationSequenceIndex &&
      navigationSequenceIndex < externalLoadIndex,
    "only hidden CDP navigation should bootstrap after URL validation and before pending navigation state",
  );
  assert.doesNotMatch(
    mainSource.slice(loadUrlIndex, hiddenBootstrapIndex),
    /await ensureBrowserPanelTabInitialNavigationBootstrap\(panel, tab\);/,
    "visible Browser panel navigation must not load about:blank before the requested target",
  );
});
