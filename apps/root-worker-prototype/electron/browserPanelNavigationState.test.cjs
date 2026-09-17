const assert = require("node:assert/strict");
const test = require("node:test");

const {
  browserPanelLoadErrorMessage,
  browserPanelUrlsEqual,
  shouldCompleteRejectedBrowserPanelNavigation,
  shouldDeferBrowserPanelFailure,
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
