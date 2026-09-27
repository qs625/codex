import test from "node:test";
import assert from "node:assert/strict";

import {
  ANDROID_CONNECTION_PAYLOAD_TYPE,
  buildAndroidConnectionPayload,
  parseAndroidConnectionPayload,
  validateAndroidConnectionEndpoint,
} from "./androidConnectionPayload";

test("builds typed Android connection payloads without empty tokens", () => {
  const payload = buildAndroidConnectionPayload({
    endpoint: " wss://example.com/root-worker ",
    token: " ",
  });

  assert.deepEqual(JSON.parse(payload), {
    type: ANDROID_CONNECTION_PAYLOAD_TYPE,
    version: 1,
    endpoint: "wss://example.com/root-worker",
  });
});

test("parses valid JSON and URI Android connection payloads", () => {
  assert.deepEqual(
    parseAndroidConnectionPayload(
      JSON.stringify({
        type: ANDROID_CONNECTION_PAYLOAD_TYPE,
        version: 1,
        endpoint: "ws://192.168.1.2:8910",
        token: "capability-token",
      }),
    ),
    {
      type: ANDROID_CONNECTION_PAYLOAD_TYPE,
      version: 1,
      endpoint: "ws://192.168.1.2:8910",
      token: "capability-token",
    },
  );

  assert.deepEqual(
    parseAndroidConnectionPayload(
      "morpheus://connect?endpoint=wss%3A%2F%2Ftunnel.example%2Fws&token=abc",
    ),
    {
      type: ANDROID_CONNECTION_PAYLOAD_TYPE,
      version: 1,
      endpoint: "wss://tunnel.example/ws",
      token: "abc",
    },
  );
});

test("parses naked websocket endpoints for manual compatibility", () => {
  assert.deepEqual(parseAndroidConnectionPayload(" ws://192.168.1.2:8910 "), {
    type: ANDROID_CONNECTION_PAYLOAD_TYPE,
    version: 1,
    endpoint: "ws://192.168.1.2:8910",
  });
});

test("normalizes JSON and URI endpoint and token fields", () => {
  assert.deepEqual(
    parseAndroidConnectionPayload(
      JSON.stringify({
        type: ANDROID_CONNECTION_PAYLOAD_TYPE,
        version: 1,
        endpoint: " wss://json.example/ws ",
        token: " json-token ",
      }),
    ),
    {
      type: ANDROID_CONNECTION_PAYLOAD_TYPE,
      version: 1,
      endpoint: "wss://json.example/ws",
      token: "json-token",
    },
  );

  assert.deepEqual(
    parseAndroidConnectionPayload(
      "morpheus://connect?endpoint=%20wss%3A%2F%2Furi.example%2Fws%20&token=%20uri-token%20",
    ),
    {
      type: ANDROID_CONNECTION_PAYLOAD_TYPE,
      version: 1,
      endpoint: "wss://uri.example/ws",
      token: "uri-token",
    },
  );
});

test("rejects malformed or unsupported Android connection payloads", () => {
  assert.throws(() => parseAndroidConnectionPayload(""), /empty/);
  assert.throws(() => parseAndroidConnectionPayload("{"), /invalid JSON/);
  assert.throws(
    () => parseAndroidConnectionPayload(JSON.stringify(null)),
    /Morpheus connection payload/,
  );
  assert.throws(
    () => parseAndroidConnectionPayload("https://example.com"),
    /Morpheus connection payload/,
  );
  assert.throws(
    () =>
      parseAndroidConnectionPayload(
        JSON.stringify({ type: "other", version: 1, endpoint: "ws://x" }),
      ),
    /not a Morpheus Android payload/,
  );
  assert.throws(
    () =>
      parseAndroidConnectionPayload(
        JSON.stringify({
          type: ANDROID_CONNECTION_PAYLOAD_TYPE,
          version: 2,
          endpoint: "ws://x",
        }),
      ),
    /version is not supported/,
  );
  assert.throws(
    () =>
      parseAndroidConnectionPayload(
        JSON.stringify({
          type: ANDROID_CONNECTION_PAYLOAD_TYPE,
          version: 1,
        }),
      ),
    /missing an endpoint/,
  );
  assert.throws(
    () =>
      parseAndroidConnectionPayload(
        JSON.stringify({
          type: ANDROID_CONNECTION_PAYLOAD_TYPE,
          version: 1,
          endpoint: "ws://x",
          token: 42,
        }),
      ),
    /token must be a string/,
  );
  assert.throws(
    () =>
      parseAndroidConnectionPayload(
        JSON.stringify({
          type: ANDROID_CONNECTION_PAYLOAD_TYPE,
          version: 1,
          endpoint: "https://example.com",
        }),
      ),
    /ws:\/\/ or wss:\/\//,
  );
});

test("validates WebSocket endpoint drafts", () => {
  assert.equal(validateAndroidConnectionEndpoint("wss://example.com/ws"), null);
  assert.match(
    validateAndroidConnectionEndpoint("https://example.com") ?? "",
    /ws:\/\//,
  );
  assert.match(
    validateAndroidConnectionEndpoint("ws://example .com") ?? "",
    /whitespace/,
  );
});
