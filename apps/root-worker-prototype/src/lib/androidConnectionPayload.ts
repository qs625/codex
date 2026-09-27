export const ANDROID_CONNECTION_PAYLOAD_TYPE = "morpheus.androidConnection";
export const ANDROID_CONNECTION_PAYLOAD_VERSION = 1;

export type AndroidConnectionPayload = {
  type: typeof ANDROID_CONNECTION_PAYLOAD_TYPE;
  version: typeof ANDROID_CONNECTION_PAYLOAD_VERSION;
  endpoint: string;
  token?: string;
};

type AndroidConnectionPayloadSource =
  | { kind: "json"; text: string }
  | { kind: "uri"; text: string }
  | { kind: "endpoint"; endpoint: string };

type AndroidConnectionPayloadFields = {
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
  return JSON.stringify(
    buildAndroidConnectionPayloadObject({
      endpoint,
      token,
    }),
  );
}

export function parseAndroidConnectionPayload(
  raw: string,
): AndroidConnectionPayload {
  const source = classifyAndroidConnectionPayloadSource(raw);
  switch (source.kind) {
    case "json":
      return parseJsonConnectionPayload(source.text);
    case "uri":
      return parseUriConnectionPayload(source.text);
    case "endpoint":
      return normalizePayload(source.endpoint, undefined);
  }
}

function classifyAndroidConnectionPayloadSource(
  raw: string,
): AndroidConnectionPayloadSource {
  const text = raw.trim();
  if (!text) {
    throw new Error("Connection QR is empty.");
  }
  if (text.startsWith("{")) {
    return { kind: "json", text };
  }
  if (text.startsWith("morpheus://")) {
    return { kind: "uri", text };
  }
  if (!validateAndroidConnectionEndpoint(text)) {
    return { kind: "endpoint", endpoint: text };
  }
  throw new Error("Connection QR must be a Morpheus connection payload.");
}

function parseJsonConnectionPayload(text: string): AndroidConnectionPayload {
  const payload = parseJsonPayloadRecord(text);
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

function parseJsonPayloadRecord(text: string): Record<string, unknown> {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    throw new Error("Connection QR contains invalid JSON.");
  }
  if (!parsed || typeof parsed !== "object") {
    throw new Error("Connection QR payload must be an object.");
  }
  return parsed as Record<string, unknown>;
}

function parseUriConnectionPayload(text: string): AndroidConnectionPayload {
  const url = parseConnectionUri(text);
  if (url.protocol !== "morpheus:" || url.hostname !== "connect") {
    throw new Error("Connection URI is not a Morpheus connect URI.");
  }
  const endpoint = url.searchParams.get("endpoint");
  if (!endpoint) {
    throw new Error("Connection URI is missing an endpoint.");
  }
  return normalizePayload(endpoint, url.searchParams.get("token") ?? undefined);
}

function parseConnectionUri(text: string): URL {
  try {
    return new URL(text);
  } catch {
    throw new Error("Connection URI is invalid.");
  }
}

function normalizePayload(endpoint: string, token: string | undefined) {
  const normalizedEndpoint = normalizeAndroidConnectionEndpoint(endpoint);
  const endpointError = validateAndroidConnectionEndpoint(normalizedEndpoint);
  if (endpointError) {
    throw new Error(endpointError);
  }
  return buildAndroidConnectionPayloadObject({
    endpoint: normalizedEndpoint,
    token: token ?? "",
  });
}

function buildAndroidConnectionPayloadObject({
  endpoint,
  token,
}: AndroidConnectionPayloadFields): AndroidConnectionPayload {
  const normalizedEndpoint = normalizeAndroidConnectionEndpoint(endpoint);
  const normalizedToken = token?.trim() ?? "";
  return {
    type: ANDROID_CONNECTION_PAYLOAD_TYPE,
    version: ANDROID_CONNECTION_PAYLOAD_VERSION,
    endpoint: normalizedEndpoint,
    ...(normalizedToken ? { token: normalizedToken } : {}),
  };
}
