export const ANDROID_CONNECTION_PAYLOAD_TYPE = "morpheus.androidConnection";
export const ANDROID_CONNECTION_PAYLOAD_VERSION = 1;

export type AndroidConnectionPayload = {
  type: typeof ANDROID_CONNECTION_PAYLOAD_TYPE;
  version: typeof ANDROID_CONNECTION_PAYLOAD_VERSION;
  endpoint: string;
  token?: string;
};

export function normalizeAndroidConnectionEndpoint(endpoint: string) {
  return endpoint.trim();
}

export function validateAndroidConnectionEndpoint(endpoint: string) {
  const normalized = normalizeAndroidConnectionEndpoint(endpoint);
  if (!normalized) {
    return "Enter a ws:// or wss:// endpoint.";
  }
  if (/\s/.test(normalized)) {
    return "Endpoint cannot contain whitespace.";
  }
  try {
    const url = new URL(normalized);
    return url.protocol === "ws:" || url.protocol === "wss:"
      ? null
      : "Endpoint must start with ws:// or wss://.";
  } catch {
    return "Endpoint must be a valid ws:// or wss:// URL.";
  }
}

export function buildAndroidConnectionPayload({
  endpoint,
  token,
}: {
  endpoint: string;
  token: string;
}) {
  const normalizedEndpoint = normalizeAndroidConnectionEndpoint(endpoint);
  const normalizedToken = token.trim();
  const payload: AndroidConnectionPayload = {
    type: ANDROID_CONNECTION_PAYLOAD_TYPE,
    version: ANDROID_CONNECTION_PAYLOAD_VERSION,
    endpoint: normalizedEndpoint,
    ...(normalizedToken ? { token: normalizedToken } : {}),
  };
  return JSON.stringify(payload);
}

export function parseAndroidConnectionPayload(raw: string): AndroidConnectionPayload {
  const text = raw.trim();
  if (!text) {
    throw new Error("Connection QR is empty.");
  }
  if (text.startsWith("{")) {
    return parseJsonConnectionPayload(text);
  }
  if (text.startsWith("morpheus://")) {
    return parseUriConnectionPayload(text);
  }
  if (!validateAndroidConnectionEndpoint(text)) {
    return {
      type: ANDROID_CONNECTION_PAYLOAD_TYPE,
      version: ANDROID_CONNECTION_PAYLOAD_VERSION,
      endpoint: text,
    };
  }
  throw new Error("Connection QR must be a Morpheus connection payload.");
}

function parseJsonConnectionPayload(text: string): AndroidConnectionPayload {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    throw new Error("Connection QR contains invalid JSON.");
  }
  if (!parsed || typeof parsed !== "object") {
    throw new Error("Connection QR payload must be an object.");
  }
  const payload = parsed as Record<string, unknown>;
  if (payload.type !== ANDROID_CONNECTION_PAYLOAD_TYPE) {
    throw new Error("Connection QR is not a Morpheus Android payload.");
  }
  if (payload.version !== ANDROID_CONNECTION_PAYLOAD_VERSION) {
    throw new Error("Connection QR payload version is not supported.");
  }
  if (typeof payload.endpoint !== "string") {
    throw new Error("Connection QR payload is missing an endpoint.");
  }
  if (payload.token !== undefined && typeof payload.token !== "string") {
    throw new Error("Connection QR token must be a string.");
  }
  return normalizePayload(payload.endpoint, payload.token);
}

function parseUriConnectionPayload(text: string): AndroidConnectionPayload {
  let url: URL;
  try {
    url = new URL(text);
  } catch {
    throw new Error("Connection URI is invalid.");
  }
  if (url.protocol !== "morpheus:" || url.hostname !== "connect") {
    throw new Error("Connection URI is not a Morpheus connect URI.");
  }
  const endpoint = url.searchParams.get("endpoint");
  if (!endpoint) {
    throw new Error("Connection URI is missing an endpoint.");
  }
  return normalizePayload(endpoint, url.searchParams.get("token") ?? undefined);
}

function normalizePayload(endpoint: string, token: string | undefined) {
  const normalizedEndpoint = normalizeAndroidConnectionEndpoint(endpoint);
  const endpointError = validateAndroidConnectionEndpoint(normalizedEndpoint);
  if (endpointError) {
    throw new Error(endpointError);
  }
  const normalizedToken = token?.trim() ?? "";
  return {
    type: ANDROID_CONNECTION_PAYLOAD_TYPE,
    version: ANDROID_CONNECTION_PAYLOAD_VERSION,
    endpoint: normalizedEndpoint,
    ...(normalizedToken ? { token: normalizedToken } : {}),
  };
}
