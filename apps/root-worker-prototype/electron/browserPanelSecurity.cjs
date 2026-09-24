const URL_SCHEME_PATTERN = /^[A-Za-z][A-Za-z0-9+.-]*:/;

function normalizeBrowserTarget(target) {
  if (typeof target !== "string" || !target.trim()) {
    return { ok: false, reason: "Enter a URL to open." };
  }

  const trimmed = target.trim();
  const candidate = browserTargetCandidate(trimmed);

  let parsed;
  try {
    parsed = new URL(candidate);
  } catch {
    return { ok: false, reason: "Enter a valid URL." };
  }

  return { ok: true, url: parsed.toString() };
}

function browserNavigationDecision(target) {
  const normalized = normalizeBrowserTarget(target);
  if (!normalized.ok) {
    return {
      allow: false,
      reason: normalized.reason,
    };
  }
  return {
    allow: true,
    url: normalized.url,
  };
}

function normalizeBrowserDebugTarget(target) {
  if (typeof target === "string" && target.trim() === "about:blank") {
    return { ok: true, url: null };
  }
  return normalizeBrowserTarget(target);
}

function browserNavigationEventDecision(event, targetDetails) {
  return browserNavigationDecision(browserNavigationEventTarget(event, targetDetails));
}

function browserNavigationEventTarget(event, targetDetails) {
  if (typeof targetDetails === "string") {
    return targetDetails;
  }
  if (targetDetails && typeof targetDetails.url === "string") {
    return targetDetails.url;
  }
  if (event && typeof event.url === "string") {
    return event.url;
  }
  return null;
}

function browserTargetCandidate(target) {
  const schemeMatch = target.match(URL_SCHEME_PATTERN);
  if (!schemeMatch) {
    return `${defaultBrowserProtocol(target)}://${target}`;
  }

  return isHostPortTarget(target)
    ? `${defaultBrowserProtocol(target)}://${target}`
    : target;
}

function defaultBrowserProtocol(target) {
  return isLocalBrowserHost(target) ? "http" : "https";
}

function isHostPortTarget(target) {
  return /^[^\s:/?#]+:\d{1,5}([/?#]|$)/.test(target);
}

function isLocalBrowserHost(target) {
  const host = target.split(/[/?#]/, 1)[0]?.split("@").pop() ?? "";
  const withoutPort = host.startsWith("[")
    ? host.slice(1, host.indexOf("]"))
    : host.split(":", 1)[0];
  const normalized = withoutPort.toLowerCase();
  return (
    normalized === "localhost" ||
    normalized === "127.0.0.1" ||
    normalized === "::1" ||
    normalized.endsWith(".localhost")
  );
}

module.exports = {
  browserNavigationDecision,
  browserNavigationEventDecision,
  browserNavigationEventTarget,
  normalizeBrowserDebugTarget,
  normalizeBrowserTarget,
};
