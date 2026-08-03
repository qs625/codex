import test from "node:test";
import assert from "node:assert/strict";

import {
  buildGlobalSettingsSections,
  buildConfigSaveParams,
  buildSettingsConfigState,
  createModelHubOptionEntry,
  createProviderRegistryEntry,
  getUnsetDraftValue,
  isModelOptionsDirty,
  isProviderRegistryDirty,
  isSettingsDirty,
  resetModelOptionDrafts,
  resetFieldDrafts,
  resetProviderRegistryDrafts,
  updateFieldDraft,
  validateSettingsDrafts,
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

test("buildSettingsConfigState maps supported global config fields", () => {
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
    state.fields.find((field) => field.keyPath === "model")?.section,
    "defaults",
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
    ["defaults", "execution", "desktop"],
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

test("configured model options are saved as catalog entries without changing global model", () => {
  const state = buildSettingsConfigState(
    readResponse({
      model: "gpt-5",
      model_provider: "openai",
      model_options: [
        {
          model: "gpt-5.5-2026-04-24",
          provider: "modelhub-gpt",
          base_url: "https://example.invalid/api/modelhub/online/v2/crawl",
          wire_api: "azure_chat_completions",
          ak: "abc",
          query_params: { region: "cn" },
          context_window: 200000,
        },
      ],
    }),
  );

  const modelOption = state.modelOptions[0];
  assert.equal(modelOption?.provider, "modelhub-gpt");
  assert.equal(modelOption?.model, "gpt-5.5-2026-04-24");
  assert.equal(modelOption?.contextWindow, "200000");

  const modelOptions = state.modelOptions.map((entry) =>
    entry.id === modelOption?.id
      ? { ...entry, model: "gpt-5.5-2026-05-01", maxTokens: "8192" }
      : entry,
  );
  const params = buildConfigSaveParams(
    state.fields,
    state.userVersion,
    state.providerRegistry,
    modelOptions,
  );

  assert.deepEqual(params?.edits, [
    {
      keyPath: "model_options",
      mergeStrategy: "replace",
      value: [
        {
          model: "gpt-5.5-2026-05-01",
          provider: "modelhub-gpt",
          base_url: "https://example.invalid/api/modelhub/online/v2/crawl",
          wire_api: "azure_chat_completions",
          ak: "abc",
          query_params: { region: "cn" },
          context_window: 200000,
          max_tokens: 8192,
        },
      ],
    },
  ]);
  assert.equal(
    params?.edits.some((edit) => edit.keyPath === "model"),
    false,
  );
  assert.equal(
    params?.edits.some((edit) => edit.keyPath === "model_provider"),
    false,
  );
});

test("creating a ModelHub option prepares a model_options write", () => {
  const state = buildSettingsConfigState(readResponse({}));
  const entry = {
    ...createModelHubOptionEntry(state.modelOptions),
    model: "modelhub-gpt-5",
    baseUrl: "https://example.invalid/modelhub",
    ak: "secret-ak",
    contextWindow: "128000",
  };

  assert.equal(entry.provider, "modelhub-gpt");
  assert.equal(isModelOptionsDirty([entry]), true);
  assert.deepEqual(
    buildConfigSaveParams(state.fields, state.userVersion, [], [entry])?.edits,
    [
      {
        keyPath: "model_options",
        mergeStrategy: "replace",
        value: [
          {
            model: "modelhub-gpt-5",
            provider: "modelhub-gpt",
            base_url: "https://example.invalid/modelhub",
            wire_api: "azure_chat_completions",
            ak: "secret-ak",
            context_window: 128000,
          },
        ],
      },
    ],
  );
});

test("custom provider registry writes provider paths and rejects reserved ids", () => {
  const state = buildSettingsConfigState(
    readResponse({
      model_providers: {
        openai: {
          name: "OpenAI",
          wire_api: "responses",
        },
        corp: {
          name: "Corp Gateway",
          base_url: "https://corp.example.invalid/v1",
          wire_api: "responses",
          env_key: "CORP_API_KEY",
        },
      },
    }),
  );

  assert.deepEqual(
    state.providerRegistry.map((entry) => entry.effectiveId),
    ["corp"],
  );

  const providerRegistry = state.providerRegistry.map((entry) => ({
    ...entry,
    draftId: "corp-east",
    baseUrl: "https://east.example.invalid/v1",
  }));
  const params = buildConfigSaveParams(
    state.fields,
    state.userVersion,
    providerRegistry,
    state.modelOptions,
  );

  assert.deepEqual(params?.edits, [
    { keyPath: "model_providers.corp", value: null, mergeStrategy: "replace" },
    {
      keyPath: "model_providers.corp-east",
      mergeStrategy: "replace",
      value: {
        name: "Corp Gateway",
        base_url: "https://east.example.invalid/v1",
        wire_api: "responses",
        env_key: "CORP_API_KEY",
      },
    },
  ]);
  assert.deepEqual(
    validateSettingsDrafts(
      [{ ...providerRegistry[0]!, draftId: "openai" }],
      state.modelOptions,
    ),
    ['Provider id "openai" is reserved.'],
  );
});

test("inline model option providers are not duplicated as custom registry entries", () => {
  const state = buildSettingsConfigState(
    readResponse({
      model_options: [
        {
          model: "gpt-5.5-2026-04-24",
          provider: "modelhub-gpt",
          base_url: "https://example.invalid/modelhub",
        },
      ],
      model_providers: {
        "modelhub-gpt": {
          name: "modelhub-gpt",
          base_url: "https://example.invalid/modelhub",
          wire_api: "azure_chat_completions",
        },
        corp: {
          name: "Corp Gateway",
          base_url: "https://corp.example.invalid/v1",
          wire_api: "responses",
        },
      },
    }),
  );

  assert.deepEqual(
    state.providerRegistry.map((entry) => entry.effectiveId),
    ["corp"],
  );
});

test("real custom provider with same id as inline model option remains visible", () => {
  const state = buildSettingsConfigState(
    readResponse({
      model_options: [
        {
          model: "gpt-5.5-2026-04-24",
          provider: "modelhub-gpt",
          base_url: "https://example.invalid/modelhub",
          ak: "inline-ak",
        },
      ],
      model_providers: {
        "modelhub-gpt": {
          name: "modelhub-gpt",
          base_url: "https://gateway.example.invalid/v1",
          wire_api: "azure_chat_completions",
          env_key: "MODELHUB_API_KEY",
        },
      },
    }),
  );

  assert.deepEqual(
    state.providerRegistry.map((entry) => entry.effectiveId),
    ["modelhub-gpt"],
  );
});

test("provider registry preserves advanced fields as readonly", () => {
  const state = buildSettingsConfigState(
    readResponse({
      model_providers: {
        corp: {
          name: "Corp Gateway",
          base_url: "https://corp.example.invalid/v1",
          wire_api: "responses",
          query_params: { tenant: "codex" },
        },
      },
    }),
  );
  const entry = state.providerRegistry[0];

  assert.equal(entry?.isReadonly, true);
  assert.equal(isProviderRegistryDirty(state.providerRegistry), false);
  assert.equal(
    buildConfigSaveParams(
      state.fields,
      state.userVersion,
      state.providerRegistry,
      state.modelOptions,
    ),
    null,
  );
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

test("reset helpers restore provider and model catalog drafts", () => {
  const state = buildSettingsConfigState(
    readResponse({
      model_options: [{ model: "a", provider: "modelhub-gpt" }],
      model_providers: {
        corp: {
          name: "Corp",
          base_url: "https://corp.example.invalid/v1",
          wire_api: "responses",
        },
      },
    }),
  );
  const providerDrafts = [
    { ...state.providerRegistry[0]!, name: "Changed" },
    createProviderRegistryEntry(state.providerRegistry),
  ];
  const modelDrafts = [
    { ...state.modelOptions[0]!, model: "changed" },
    createModelHubOptionEntry(state.modelOptions),
  ];

  assert.equal(resetProviderRegistryDrafts(providerDrafts).length, 1);
  assert.equal(resetProviderRegistryDrafts(providerDrafts)[0]?.name, "Corp");
  assert.equal(resetModelOptionDrafts(modelDrafts).length, 1);
  assert.equal(resetModelOptionDrafts(modelDrafts)[0]?.model, "a");
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

test("global settings sections keep model defaults outside provider registry", () => {
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
    ["defaults", "execution", "desktop"],
  );
  assert.equal(
    sections.some((section) =>
      section.fields.some((field) => field.keyPath === "model_provider"),
    ),
    true,
  );
});

test("model option validation rejects empty models and non-positive windows", () => {
  const entry = {
    ...createModelHubOptionEntry([]),
    model: "",
    provider: "modelhub-gpt",
    contextWindow: "0",
  };

  assert.deepEqual(validateSettingsDrafts([], [entry]), [
    "Configured model id is required.",
    "Configured model context window must be positive.",
  ]);
});
