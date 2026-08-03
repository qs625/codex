import test from "node:test";
import assert from "node:assert/strict";

import {
  buildGlobalSettingsSections,
  buildConfigSaveParams,
  buildProviderSettingsGroups,
  buildSettingsConfigState,
  getUnsetDraftValue,
  isSettingsDirty,
  resetFieldDrafts,
  updateFieldDraft,
} from "./configSettings";
import type { ConfigReadResponse } from "../types";

function readResponse(
  config: ConfigReadResponse["config"],
  overrides: Partial<ConfigReadResponse> = {},
): ConfigReadResponse {
  return {
    config,
    origins: {
      model: {
        name: { type: "user", file: "/home/.codex/config.toml", profile: null },
        version: "v1",
      },
      sandbox_mode: {
        name: { type: "project", dotCodexFolder: "/work/.codex" },
        version: "project-v1",
      },
      ...overrides.origins,
    },
    layers: [
      {
        name: { type: "user", file: "/home/.codex/config.toml", profile: null },
        version: "v1",
        config: {},
        disabledReason: null,
      },
      ...(overrides.layers ?? []),
    ],
  };
}

test("buildSettingsConfigState maps supported scalar config fields", () => {
  const state = buildSettingsConfigState(
    readResponse({
      model: "gpt-5",
      model_provider: "openai",
      approval_policy: "on-request",
      sandbox_mode: "workspace-write",
      desktop: { appearanceTheme: "dark" },
    }),
  );

  assert.equal(state.userConfigPath, "/home/.codex/config.toml");
  assert.equal(state.userVersion, "v1");
  assert.equal(
    state.fields.find((field) => field.keyPath === "model")?.draftValue,
    "gpt-5",
  );
  assert.equal(
    state.fields.find((field) => field.keyPath === "desktop.appearanceTheme")
      ?.draftValue,
    "dark",
  );
  assert.equal(
    state.fields.find((field) => field.keyPath === "sandbox_mode")?.originLabel,
    "Project",
  );
  assert.deepEqual(
    state.globalSections.map((section) => section.id),
    ["execution", "desktop"],
  );
});

test("buildConfigSaveParams saves multiple dirty fields with reloadUserConfig", () => {
  const state = buildSettingsConfigState(
    readResponse({
      model: "gpt-5",
      approval_policy: "on-request",
    }),
  );
  const fields = updateFieldDraft(
    updateFieldDraft(state.fields, "model", "gpt-5.1"),
    "approval_policy",
    "never",
  );

  const params = buildConfigSaveParams(fields, state.userVersion);

  assert.deepEqual(params, {
    edits: [
      { keyPath: "model", value: "gpt-5.1", mergeStrategy: "replace" },
      { keyPath: "approval_policy", value: "never", mergeStrategy: "replace" },
    ],
    expectedVersion: "v1",
    reloadUserConfig: true,
  });
});

test("unset draft values clear config paths", () => {
  const state = buildSettingsConfigState(readResponse({ model: "gpt-5" }));
  const fields = updateFieldDraft(
    state.fields,
    "model",
    getUnsetDraftValue(),
  );

  const params = buildConfigSaveParams(fields, state.userVersion);

  assert.equal(params?.edits[0]?.value, null);
});

test("write failure can keep dirty drafts by leaving helper state untouched", () => {
  const state = buildSettingsConfigState(readResponse({ model: "gpt-5" }));
  const fields = updateFieldDraft(state.fields, "model", "gpt-5.1");

  assert.equal(isSettingsDirty(fields), true);
  assert.equal(
    fields.find((field) => field.keyPath === "model")?.draftValue,
    "gpt-5.1",
  );
});

test("resetFieldDrafts reverts dirty fields to effective values", () => {
  const state = buildSettingsConfigState(readResponse({ model: "gpt-5" }));
  const fields = updateFieldDraft(state.fields, "model", "gpt-5.1");

  assert.equal(isSettingsDirty(fields), true);
  assert.equal(isSettingsDirty(resetFieldDrafts(fields)), false);
});

test("missing optional fields and unsupported nested values do not crash", () => {
  const state = buildSettingsConfigState(
    readResponse({
      approval_policy: {
        granular: {
          sandbox_approval: true,
          rules: true,
          skill_approval: true,
          request_permissions: false,
          mcp_elicitations: true,
        },
      },
    }),
  );

  const model = state.fields.find((field) => field.keyPath === "model");
  const approval = state.fields.find(
    (field) => field.keyPath === "approval_policy",
  );

  assert.equal(model?.draftValue, getUnsetDraftValue());
  assert.equal(approval?.isUnsupported, true);
  assert.match(approval?.unsupportedValue ?? "", /granular/);
});

test("unsupported text field objects and arrays stay readonly and unsaved", () => {
  const objectState = buildSettingsConfigState(
    readResponse({
      model: { unexpected: "nested" },
    }),
  );
  const objectModel = objectState.fields.find(
    (field) => field.keyPath === "model",
  );

  assert.equal(objectModel?.isUnsupported, true);
  assert.match(objectModel?.unsupportedValue ?? "", /nested/);
  assert.equal(buildConfigSaveParams(objectState.fields, objectState.userVersion), null);

  const arrayState = buildSettingsConfigState(
    readResponse({
      model_provider: ["openai"],
    }),
  );
  const arrayProvider = arrayState.fields.find(
    (field) => field.keyPath === "model_provider",
  );

  assert.equal(arrayProvider?.isUnsupported, true);
  assert.match(arrayProvider?.unsupportedValue ?? "", /openai/);
  assert.equal(buildConfigSaveParams(arrayState.fields, arrayState.userVersion), null);
});

test("provider groups expose OpenAI and ModelHub without losing unknown providers", () => {
  const openAiState = buildSettingsConfigState(
    readResponse({
      model_provider: "openai",
      model: "gpt-5",
    }),
  );
  const openAiGroups = buildProviderSettingsGroups(openAiState.fields);

  assert.equal(openAiGroups[0]?.id, "openai");
  assert.equal(openAiGroups[0]?.status, "active");
  assert.equal(openAiGroups[1]?.id, "modelhub");
  assert.equal(openAiGroups[1]?.status, "available");
  assert.deepEqual(
    openAiGroups[0]?.fields.map((field) => field.keyPath),
    ["model", "model_reasoning_effort", "model_verbosity"],
  );

  const customState = buildSettingsConfigState(
    readResponse({
      model_provider: "local-proxy",
      model: "llama",
    }),
  );
  const customGroups = buildProviderSettingsGroups(customState.fields);
  const customGroup = customGroups.find((group) => group.id === "custom");

  assert.equal(customGroup?.status, "custom");
  assert.equal(customGroup?.providerValue, "local-proxy");
  assert.ok(
    customGroup?.fields.some((field) => field.keyPath === "model_provider"),
  );
});

test("global settings sections exclude provider fields", () => {
  const state = buildSettingsConfigState(
    readResponse({
      model_provider: "modelhub",
      sandbox_mode: "workspace-write",
      desktop: { appearanceTheme: "dark" },
    }),
  );
  const sections = buildGlobalSettingsSections(state.fields);

  assert.deepEqual(
    sections.map((section) => section.id),
    ["execution", "desktop"],
  );
  assert.equal(
    sections.some((section) =>
      section.fields.some((field) => field.keyPath === "model_provider"),
    ),
    false,
  );
});
