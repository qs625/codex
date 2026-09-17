const assert = require("node:assert/strict");
const test = require("node:test");

const {
  browserPanelLoadErrorMessage,
  browserPanelNavigationTimeoutMessage,
  browserPanelUrlsEqual,
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
