import test from "node:test";
import assert from "node:assert/strict";

import {
  pendingLoginFromResponse,
  resolveOpenAiAuthState,
} from "./accountSettings";

test("resolveOpenAiAuthState maps unauthenticated required account", () => {
  const state = resolveOpenAiAuthState({
    account: null,
    requiresOpenaiAuth: true,
  });

  assert.equal(state.status, "required");
  assert.equal(state.label, "Authentication required");
});

test("resolveOpenAiAuthState maps authenticated api key and chatgpt accounts", () => {
  const apiKey = resolveOpenAiAuthState({
    account: { type: "apiKey" },
    requiresOpenaiAuth: true,
  });
  const chatgpt = resolveOpenAiAuthState({
    account: { type: "chatgpt", email: "user@example.com", planType: "pro" },
    requiresOpenaiAuth: true,
  });

  assert.equal(apiKey.status, "authenticated");
  assert.equal(apiKey.label, "API key connected");
  assert.equal(chatgpt.status, "authenticated");
  assert.match(chatgpt.detail, /user@example.com/);
});

test("resolveOpenAiAuthState maps provider states that do not require OpenAI", () => {
  const state = resolveOpenAiAuthState({
    account: null,
    requiresOpenaiAuth: false,
  });

  assert.equal(state.status, "notRequired");
});

test("resolveOpenAiAuthState does not treat non-OpenAI accounts as OpenAI auth", () => {
  const required = resolveOpenAiAuthState({
    account: { type: "amazonBedrock" },
    requiresOpenaiAuth: true,
  });
  const notRequired = resolveOpenAiAuthState({
    account: { type: "amazonBedrock" },
    requiresOpenaiAuth: false,
  });

  assert.equal(required.status, "required");
  assert.match(required.detail, /not an OpenAI credential/);
  assert.equal(notRequired.status, "notRequired");
});

test("pendingLoginFromResponse maps managed login responses without secrets", () => {
  const browser = pendingLoginFromResponse({
    type: "chatgpt",
    loginId: "login-1",
    authUrl: "https://chatgpt.example/auth",
  });
  const device = pendingLoginFromResponse({
    type: "chatgptDeviceCode",
    loginId: "login-2",
    verificationUrl: "https://auth.example/device",
    userCode: "ABCD-1234",
  });
  const apiKey = pendingLoginFromResponse({ type: "apiKey" });

  assert.deepEqual(browser, {
    loginId: "login-1",
    mode: "chatgpt",
    authUrl: "https://chatgpt.example/auth",
  });
  assert.deepEqual(device, {
    loginId: "login-2",
    mode: "device",
    verificationUrl: "https://auth.example/device",
    userCode: "ABCD-1234",
  });
  assert.equal(apiKey, null);
});
