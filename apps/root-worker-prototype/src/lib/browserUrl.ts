export type BrowserUrlResult =
  | {
      ok: true;
      url: string;
    }
  | {
      ok: false;
      reason: string;
    };

const URL_SCHEME_PATTERN = /^[A-Za-z][A-Za-z0-9+.-]*:/;

export function normalizeBrowserUrl(input: string): BrowserUrlResult {
  const trimmed = input.trim();
  if (!trimmed) {
    return { ok: false, reason: "Enter a URL to open." };
  }

  const candidate = browserTargetCandidate(trimmed);

  let parsed: URL;
  try {
    parsed = new URL(candidate);
  } catch {
    return { ok: false, reason: "Enter a valid URL." };
  }

  return { ok: true, url: parsed.toString() };
}

function browserTargetCandidate(target: string) {
  const schemeMatch = target.match(URL_SCHEME_PATTERN);
  if (!schemeMatch) {
    return `${defaultProtocolForBrowserTarget(target)}://${target}`;
  }

  return isHostPortTarget(target)
    ? `${defaultProtocolForBrowserTarget(target)}://${target}`
    : target;
}

function defaultProtocolForBrowserTarget(target: string) {
  return isLocalBrowserHost(target) ? "http" : "https";
}

function isHostPortTarget(target: string) {
  return /^[^\s:/?#]+:\d{1,5}([/?#]|$)/.test(target);
}

function isLocalBrowserHost(target: string) {
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
