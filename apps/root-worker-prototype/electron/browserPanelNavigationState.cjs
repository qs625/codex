function browserPanelLoadErrorMessage(failure) {
  const description = failure.errorDescription || "Page failed to load";
  return typeof failure.errorCode === "number"
    ? `${description} (${failure.errorCode})`
    : description;
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

module.exports = {
  browserPanelLoadErrorMessage,
  browserPanelUrlsEqual,
  shouldCompleteRejectedBrowserPanelNavigation,
  shouldDeferBrowserPanelFailure,
};
