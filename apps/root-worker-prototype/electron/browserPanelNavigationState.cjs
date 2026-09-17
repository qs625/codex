function browserPanelLoadErrorMessage(failure) {
  const description = failure.errorDescription || "Page failed to load";
  return typeof failure.errorCode === "number"
    ? `${description} (${failure.errorCode})`
    : description;
}

function browserPanelNavigationTimeoutMessage(timeoutMs) {
  const seconds = Math.max(1, Math.round(timeoutMs / 1_000));
  return `Browser navigation timed out after ${seconds}s`;
}

function browserPanelUrlsEqual(left, right) {
  if (!left || !right) {
    return false;
  }
  try {
    return new URL(left).href === new URL(right).href;
  } catch {
    return left === right;
  }
}

function shouldDeferBrowserPanelFailure({
  errorCode,
  validatedUrl,
  loading,
  currentUrl,
}) {
  return (
    errorCode === -2 &&
    Boolean(validatedUrl) &&
    Boolean(loading) &&
    !browserPanelUrlsEqual(currentUrl, validatedUrl)
  );
}

function shouldCompleteRejectedBrowserPanelNavigation({
  navigationSequence,
  finishedNavigationSequence,
  finishedUrl,
  currentUrl,
  targetUrl,
}) {
  return (
    navigationSequence === finishedNavigationSequence &&
    browserPanelUrlsEqual(finishedUrl, targetUrl) &&
    browserPanelUrlsEqual(currentUrl, targetUrl)
  );
}

function shouldExposeBrowserPanelLoading({
  observedLoading,
  pendingNavigationSequence,
}) {
  return Boolean(observedLoading && pendingNavigationSequence !== null);
}

function waitForBrowserPanelNavigationResult(
  loadPromise,
  timeoutMs,
  timers = {},
  observedNavigationPromise = null,
) {
  const setTimer = timers.setTimeout ?? setTimeout;
  const clearTimer = timers.clearTimeout ?? clearTimeout;
  let timeout = null;

  const timeoutPromise = new Promise((_, reject) => {
    timeout = setTimer(() => {
      const error = new Error(browserPanelNavigationTimeoutMessage(timeoutMs));
      error.code = "ERR_BROWSER_PANEL_NAVIGATION_TIMEOUT";
      reject(error);
    }, timeoutMs);
  });

  const navigationSignals = [loadPromise, timeoutPromise];
  if (observedNavigationPromise) {
    navigationSignals.push(observedNavigationPromise);
  }

  return Promise.race(navigationSignals).finally(() => {
    if (timeout !== null) {
      clearTimer(timeout);
    }
  });
}

module.exports = {
  browserPanelLoadErrorMessage,
  browserPanelNavigationTimeoutMessage,
  browserPanelUrlsEqual,
  shouldCompleteRejectedBrowserPanelNavigation,
  shouldDeferBrowserPanelFailure,
  shouldExposeBrowserPanelLoading,
  waitForBrowserPanelNavigationResult,
};
