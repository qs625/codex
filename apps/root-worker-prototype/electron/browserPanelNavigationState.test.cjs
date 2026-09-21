const assert = require("node:assert/strict");
const test = require("node:test");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");

const {
  browserPanelLoadErrorMessage,
  browserPanelNavigationTimeoutMessage,
  browserPanelUrlsEqual,
  shouldAcceptBrowserPanelCommittedNavigation,
  shouldCompleteBrowserPanelStoppedNavigation,
  shouldCompleteRejectedBrowserPanelNavigation,
  shouldDeferBrowserPanelFailure,
  shouldExposeBrowserPanelLoading,
  waitForBrowserPanelNavigationResult,
} = require("./browserPanelNavigationState.cjs");

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
  const mainSource = readFileSync(join(__dirname, "main.cjs"), "utf8");
  const observerIndex = mainSource.indexOf("function observeBrowserPanelTargetNavigation");
  const stopHandlerIndex = mainSource.indexOf("const handleStop = () => {", observerIndex);
  const observedStopListenerIndex = mainSource.indexOf(
    'webContents.on("did-stop-loading", handleStop);',
    observerIndex,
  );
  const globalStopIndex = mainSource.indexOf(
    'tab.view.webContents.on("did-stop-loading", () => {',
  );

  assert.notEqual(observerIndex, -1);
  assert.notEqual(stopHandlerIndex, -1);
  assert.notEqual(observedStopListenerIndex, -1);
  assert.notEqual(globalStopIndex, -1);
  assert.notEqual(
    mainSource.indexOf("committedUrlForSequence", stopHandlerIndex),
    -1,
  );
  assert.notEqual(
    mainSource.indexOf("committedUrl: committedUrlForSequence", stopHandlerIndex),
    -1,
  );
  assert.notEqual(
    mainSource.indexOf("completeBrowserPanelNavigationWhenTargetReady", globalStopIndex),
    -1,
  );
});

test("main browser navigation observes committed redirects before full load", () => {
  const mainSource = readFileSync(join(__dirname, "main.cjs"), "utf8");
  const observerIndex = mainSource.indexOf("function observeBrowserPanelTargetNavigation");
  const startListenerIndex = mainSource.indexOf(
    'webContents.on("did-start-navigation", handleStart);',
    observerIndex,
  );
  const frameNavigateListenerIndex = mainSource.indexOf(
    'webContents.on("did-frame-navigate", handleFrameNavigate);',
    observerIndex,
  );
  const committedHelperIndex = mainSource.indexOf(
    "shouldAcceptBrowserPanelCommittedNavigation",
    observerIndex,
  );

  assert.notEqual(observerIndex, -1);
  assert.notEqual(startListenerIndex, -1);
  assert.notEqual(frameNavigateListenerIndex, -1);
  assert.notEqual(committedHelperIndex, -1);
  assert.notEqual(
    mainSource.indexOf("browserNavigationDecision(url)", observerIndex),
    -1,
  );
  assert.notEqual(
    mainSource.indexOf("let committedUrlForSequence = null;", observerIndex),
    -1,
  );
  assert.notEqual(
    mainSource.indexOf("committedUrl: committedUrlForSequence", observerIndex),
    -1,
  );
});

test("main browser target wait maps devtools target id instead of page type", () => {
  const mainSource = readFileSync(join(__dirname, "main.cjs"), "utf8");
  const targetWaitIndex = mainSource.indexOf(
    "async function waitForBrowserPanelDevToolsTarget",
  );
  const targetWaitSource = mainSource.slice(
    targetWaitIndex,
    mainSource.indexOf("async function waitForBrowserPanelVisibleNavigationTarget"),
  );

  assert.notEqual(targetWaitIndex, -1);
  assert.match(targetWaitSource, /fromDevToolsTargetId\(candidate\.id\) === webContents/);
  assert.doesNotMatch(targetWaitSource, /candidate\.type !== "page"/);
});

test("main browser debug target creation can load hidden tabs while waiting for target publication", () => {
  const mainSource = readFileSync(join(__dirname, "main.cjs"), "utf8");
  const createTargetIndex = mainSource.indexOf(
    "async function createBrowserPanelDebugTarget",
  );
  const createTargetSource = mainSource.slice(
    createTargetIndex,
    mainSource.indexOf("async function loadBrowserPanelTabAboutBlankBootstrap"),
  );

  assert.notEqual(createTargetIndex, -1);
  assert.match(
    createTargetSource,
    /await loadBrowserPanelTabUrl\(panel, tab, targetRequest\.url, \{[\s\S]*requireVisiblePanel: false,[\s\S]*\}\);/,
  );
  assert.doesNotMatch(createTargetSource, /void loadBrowserPanelTabUrl/);
  assert.match(
    createTargetSource,
    /const targetId = await waitForBrowserPanelDevToolsTarget\(tab\.view\.webContents\);/,
  );
});
