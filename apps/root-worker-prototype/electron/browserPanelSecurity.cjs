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
    return { ok: false, reason: "Enter a valid http or https URL." };
  }

  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
    return { ok: false, reason: "Only http and https URLs can open here." };
  }
  if (!parsed.hostname) {
    return { ok: false, reason: "Enter a URL with a host." };
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

function browserNavigationEventDecision(event, legacyUrl) {
  return browserNavigationDecision(browserNavigationEventTarget(event, legacyUrl));
}

function browserNavigationEventTarget(event, legacyUrl) {
  if (typeof legacyUrl === "string") {
    return legacyUrl;
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

  const protocol = schemeMatch[0].toLowerCase();
  if (protocol === "http:" || protocol === "https:") {
    return target;
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
  normalizeBrowserTarget,
};
