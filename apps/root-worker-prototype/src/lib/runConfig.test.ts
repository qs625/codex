import test from "node:test";
import assert from "node:assert/strict";

import {
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

test("resolveSelectionForModel keeps supported current effort", () => {
  assert.deepEqual(resolveSelectionForModel(makeModel({}), "low"), {
    model: "model-a",
    reasoningEffort: "low",
  });
});

test("resolveSelectionForModel falls back to model default effort", () => {
  assert.deepEqual(resolveSelectionForModel(makeModel({}), "high"), {
    model: "model-a",
    reasoningEffort: "medium",
  });
});
