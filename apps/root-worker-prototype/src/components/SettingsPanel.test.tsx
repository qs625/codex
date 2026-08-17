import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import {
  AndroidConnectionSection,
  applyAndroidConnectionInfoDraft,
} from "./SettingsPanel";

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

test("Android connection draft follows refreshed automatic endpoint", () => {
  const initial = applyAndroidConnectionInfoDraft(
    {
      endpoint: "",
      token: "",
      autoEndpoint: null,
      autoToken: null,
    },
    {
      enabled: true,
      bindEndpoint: "ws://0.0.0.0:8910",
      endpoint: "ws://192.168.1.10:8910/",
      token: "token-a",
      auth: "capability-token",
    },
  );
  const refreshed = applyAndroidConnectionInfoDraft(initial, {
    enabled: true,
    bindEndpoint: "ws://0.0.0.0:8910",
    endpoint: "ws://192.168.1.44:8910/",
    token: "token-a",
    auth: "capability-token",
  });

  assert.equal(refreshed.endpoint, "ws://192.168.1.44:8910/");
  assert.equal(refreshed.token, "token-a");
  assert.equal(refreshed.autoEndpoint, "ws://192.168.1.44:8910/");
});

test("Android connection draft preserves manual tunnel endpoint across refresh", () => {
  const initial = applyAndroidConnectionInfoDraft(
    {
      endpoint: "",
      token: "",
      autoEndpoint: null,
      autoToken: null,
    },
    {
      enabled: true,
      bindEndpoint: "ws://0.0.0.0:8910",
      endpoint: "ws://192.168.1.10:8910/",
      token: "token-a",
      auth: "capability-token",
    },
  );
  const manual = {
    ...initial,
    endpoint: "wss://tunnel.example/root-worker",
  };
  const refreshed = applyAndroidConnectionInfoDraft(manual, {
    enabled: true,
    bindEndpoint: "ws://0.0.0.0:8910",
    endpoint: "ws://192.168.1.44:8910/",
    token: "token-b",
    auth: "capability-token",
  });

  assert.equal(refreshed.endpoint, "wss://tunnel.example/root-worker");
  assert.equal(refreshed.token, "token-b");
  assert.equal(refreshed.autoEndpoint, "ws://192.168.1.44:8910/");
  assert.equal(refreshed.autoToken, "token-b");
});
