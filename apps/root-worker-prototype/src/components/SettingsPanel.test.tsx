import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { AndroidConnectionSection } from "./SettingsPanel";

test("Android connection section renders typed QR payload inputs", () => {
  const markup = renderToStaticMarkup(
    <AndroidConnectionSection
      endpoint="wss://tunnel.example/root-worker"
      onEndpointChange={() => {}}
      onTokenChange={() => {}}
      token="capability-token"
    />,
  );

  assert.match(markup, /Android Companion/);
  assert.match(markup, /Mobile connect QR/);
  assert.match(markup, /wss:\/\/tunnel.example\/root-worker/);
  assert.match(markup, /capability-token/);
  assert.match(markup, /morpheus.androidConnection/);
  assert.match(markup, /Copy Payload/);
});

test("Android connection section omits empty tokens from generated payload", () => {
  const markup = renderToStaticMarkup(
    <AndroidConnectionSection
      endpoint="ws://192.168.1.2:8910"
      onEndpointChange={() => {}}
      onTokenChange={() => {}}
      token=" "
    />,
  );

  assert.match(markup, /ws:\/\/192.168.1.2:8910/);
  assert.doesNotMatch(markup, /&quot;token&quot;/);
});

test("Android connection section shows validation state for non-websocket endpoints", () => {
  const markup = renderToStaticMarkup(
    <AndroidConnectionSection
      endpoint="https://example.com"
      onEndpointChange={() => {}}
      onTokenChange={() => {}}
      token=""
    />,
  );

  assert.match(markup, /Endpoint must start with ws:\/\/ or wss:\/\//);
  assert.doesNotMatch(markup, /morpheus.androidConnection/);
});
