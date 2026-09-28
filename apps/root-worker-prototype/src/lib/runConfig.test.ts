import test from "node:test";
import assert from "node:assert/strict";

import {
  ensureCurrentModelVisible,
  getRunModelLabel,
  normalizeModelListResponse,
  resolveRunConfigDisplaySummary,
  resolveSelectionForModel,
} from "./runConfig";
import type { RunModel, Thread } from "../types";

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

function makeThread(overrides: Partial<Thread> = {}): Thread {
  return {
    id: "thread-1",
    sessionId: "session-1",
    forkedFromId: null,
    preview: "",
    ephemeral: false,
    modelProvider: "openai",
    model: null,
    reasoningEffort: null,
    createdAt: 1,
    updatedAt: 1,
    lifecycleStatus: { type: "final", result: { type: "completed" } },
    path: null,
    cwd: "/tmp",
    cliVersion: "test",
    source: "appServer",
    threadSource: null,
    agentNickname: null,
    agentRole: null,
    gitInfo: null,
    name: null,
    skills: [],
    turns: [],
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
      id: "current::thread-model",
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
      contextWindow: null,
      maxContextWindow: null,
      autoCompactTokenLimit: null,
    },
  );
});

test("resolveSelectionForModel falls back to model default effort", () => {
  assert.deepEqual(resolveSelectionForModel(makeModel({}), "high"), {
    model: "model-a",
    modelProvider: null,
    reasoningEffort: "medium",
    contextWindow: null,
    maxContextWindow: null,
    autoCompactTokenLimit: null,
  });
});

test("resolveSelectionForModel falls back to first supported effort when default is not listed", () => {
  assert.deepEqual(
    resolveSelectionForModel(
      makeModel({
        supportedReasoningEfforts: [
          { reasoningEffort: "low", description: "" },
          { reasoningEffort: "high", description: "" },
        ],
        defaultReasoningEffort: "medium",
      }),
      null,
    ),
    {
      model: "model-a",
      modelProvider: null,
      reasoningEffort: "low",
      contextWindow: null,
      maxContextWindow: null,
      autoCompactTokenLimit: null,
    },
  );
});

test("resolveSelectionForModel carries model context metadata", () => {
  assert.deepEqual(
    resolveSelectionForModel(
      makeModel({
        contextWindow: 128000,
        maxContextWindow: 256000,
        autoCompactTokenLimit: 90000,
      }),
      "medium",
    ),
    {
      model: "model-a",
      modelProvider: null,
      reasoningEffort: "medium",
      contextWindow: 128000,
      maxContextWindow: 256000,
      autoCompactTokenLimit: 90000,
    },
  );
});

test("resolveRunConfigDisplaySummary resolves inherited default model", () => {
  assert.deepEqual(
    resolveRunConfigDisplaySummary(makeThread(), [
      makeModel({
        model: "gpt-5.6",
        displayName: "GPT-5.6",
        modelProvider: "openai",
        defaultReasoningEffort: "high",
        supportedReasoningEfforts: [
          { reasoningEffort: "medium", description: "" },
          { reasoningEffort: "high", description: "" },
        ],
        isDefault: true,
      }),
    ]),
    {
      modelLabel: "GPT-5.6 · openai",
      reasoningLabel: "high",
      selection: {
        model: "gpt-5.6",
        modelProvider: "openai",
        reasoningEffort: "high",
        contextWindow: null,
        maxContextWindow: null,
        autoCompactTokenLimit: null,
      },
      provenance: "inherited",
    },
  );
});

test("resolveRunConfigDisplaySummary keeps explicit current-only model concrete", () => {
  assert.deepEqual(
    resolveRunConfigDisplaySummary(
      makeThread({
        model: "thread-model",
        modelProvider: "provider-a",
        reasoningEffort: null,
      }),
      [],
    ),
    {
      modelLabel: "thread-model · provider-a",
      reasoningLabel: "unresolved reasoning",
      selection: null,
      provenance: "explicit",
    },
  );
});

test("resolveRunConfigDisplaySummary reports unresolved inherited config", () => {
  assert.deepEqual(resolveRunConfigDisplaySummary(makeThread(), []), {
    modelLabel: "unresolved model (openai)",
    reasoningLabel: "unresolved reasoning",
    selection: null,
    provenance: "unresolved",
  });
});

test("resolveRunConfigDisplaySummary does not use another provider default", () => {
  assert.deepEqual(
    resolveRunConfigDisplaySummary(makeThread({ modelProvider: "openai" }), [
      makeModel({
        model: "claude-sonnet",
        displayName: "Claude Sonnet",
        modelProvider: "anthropic",
        isDefault: true,
      }),
    ]),
    {
      modelLabel: "unresolved model (openai)",
      reasoningLabel: "unresolved reasoning",
      selection: null,
      provenance: "unresolved",
    },
  );
});

test("getRunModelLabel includes provider when present", () => {
  assert.equal(
    getRunModelLabel(
      makeModel({ displayName: "Model A", modelProvider: "modelhub-gpt" }),
    ),
    "Model A · modelhub-gpt",
  );
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
