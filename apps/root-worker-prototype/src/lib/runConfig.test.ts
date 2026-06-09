import test from "node:test";
import assert from "node:assert/strict";

import {
  ensureCurrentModelVisible,
  normalizeModelListResponse,
  resolveSelectionForModel,
} from "./runConfig";
import type { RunModel } from "../types";

function makeModel(overrides: Partial<RunModel>): RunModel {
  return {
    id: "model-a",
    model: "model-a",
    displayName: "Model A",
    description: "",
    hidden: false,
    supportedReasoningEfforts: [
      { reasoningEffort: "low", description: "" },
      { reasoningEffort: "medium", description: "" },
    ],
    defaultReasoningEffort: "medium",
    isDefault: false,
    ...overrides,
  };
}

test("normalizeModelListResponse filters hidden models and sorts default first", () => {
  const models = normalizeModelListResponse({
    data: [
      makeModel({
        id: "hidden",
        model: "hidden",
        displayName: "Hidden",
        hidden: true,
      }),
      makeModel({
        id: "secondary",
        model: "secondary",
        displayName: "Secondary",
      }),
      makeModel({
        id: "default",
        model: "default",
        displayName: "Default",
        isDefault: true,
      }),
    ],
  });

  assert.deepEqual(
    models.map((model) => model.model),
    ["default", "secondary"],
  );
});

test("normalizeModelListResponse keeps configured models near the top", () => {
  const models = normalizeModelListResponse({
    data: [
      makeModel({
        id: "secondary",
        model: "secondary",
        displayName: "Secondary",
      }),
      makeModel({
        id: "configured:corp:configured",
        model: "configured",
        displayName: "Configured",
        description: "当前配置中的模型 · Corp Gateway",
      }),
      makeModel({
        id: "default",
        model: "default",
        displayName: "Default",
        isDefault: true,
      }),
    ],
  });

  assert.deepEqual(
    models.map((model) => model.model),
    ["default", "configured", "secondary"],
  );
  assert.equal(models[1]?.configured, true);
});

test("ensureCurrentModelVisible keeps an unknown current model selected", () => {
  const models = ensureCurrentModelVisible(
    normalizeModelListResponse({
      data: [
        makeModel({
          id: "default",
          model: "default",
          displayName: "Default",
          isDefault: true,
        }),
      ],
    }),
    "thread-model",
    "provider-a",
    "high",
  );

  assert.deepEqual(
    models.map((model) => model.model),
    ["thread-model", "default"],
  );
  assert.deepEqual(models[0], {
    id: "current:provider-a:thread-model",
    model: "thread-model",
    modelProvider: "provider-a",
    displayName: "thread-model",
    description: "当前 thread 的模型，未出现在 model/list",
    hidden: false,
    supportedReasoningEfforts: [{ reasoningEffort: "high", description: "" }],
    defaultReasoningEffort: "high",
    isDefault: false,
    current: true,
  });
});

test("ensureCurrentModelVisible does not duplicate an existing current model", () => {
  const models = ensureCurrentModelVisible(
    normalizeModelListResponse({
      data: [makeModel({ model: "model-a" })],
    }),
    "model-a",
    null,
    "high",
  );

  assert.deepEqual(
    models.map((model) => model.model),
    ["model-a"],
  );
});

test("ensureCurrentModelVisible keeps missing reasoning unselectable", () => {
  const models = ensureCurrentModelVisible([], "thread-model", null, null);

  assert.deepEqual(models, [
    {
      id: "current:thread-model",
      model: "thread-model",
      modelProvider: null,
      displayName: "thread-model",
      description: "当前 thread 的模型，未出现在 model/list",
      hidden: false,
      supportedReasoningEfforts: [],
      defaultReasoningEffort: "unknown",
      isDefault: false,
      current: true,
    },
  ]);
});

test("resolveSelectionForModel keeps supported current effort", () => {
  assert.deepEqual(
    resolveSelectionForModel(makeModel({ modelProvider: "provider-a" }), "low"),
    {
      model: "model-a",
      modelProvider: "provider-a",
      reasoningEffort: "low",
    },
  );
});

test("resolveSelectionForModel falls back to model default effort", () => {
  assert.deepEqual(resolveSelectionForModel(makeModel({}), "high"), {
    model: "model-a",
    modelProvider: null,
    reasoningEffort: "medium",
  });
});

test("ensureCurrentModelVisible distinguishes providers for the same model", () => {
  const models = ensureCurrentModelVisible(
    normalizeModelListResponse({
      data: [makeModel({ model: "model-a", modelProvider: "openai" })],
    }),
    "model-a",
    "corp",
    "high",
  );

  assert.deepEqual(
    models.map((model) => model.modelProvider),
    ["corp", "openai"],
  );
});
