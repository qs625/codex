import type { RunModel, RunModelListResponse } from "../types";

export type RunConfigSelection = {
  model: string;
  reasoningEffort: string;
};

export function normalizeModelListResponse(
  response: RunModelListResponse,
): RunModel[] {
  return [...(response.data ?? [])]
    .map((model) => ({
      ...model,
      configured: model.configured ?? isConfiguredModel(model),
    }))
    .filter((model) => !model.hidden)
    .sort(compareRunModels);
}

export function ensureCurrentModelVisible(
  models: RunModel[],
  currentModel: string | null,
  currentReasoningEffort: string | null,
): RunModel[] {
  if (!currentModel || models.some((model) => model.model === currentModel)) {
    return models;
  }

  return [
    makeCurrentModel(currentModel, currentReasoningEffort),
    ...models,
  ].sort(compareRunModels);
}

export function getRunModelLabel(model: RunModel) {
  return model.displayName || model.model || model.id;
}

function compareRunModels(left: RunModel, right: RunModel) {
  if (Boolean(left.current) !== Boolean(right.current)) {
    return left.current ? -1 : 1;
  }
  if (left.isDefault !== right.isDefault) {
    return left.isDefault ? -1 : 1;
  }
  if (Boolean(left.configured) !== Boolean(right.configured)) {
    return left.configured ? -1 : 1;
  }
  return getRunModelLabel(left).localeCompare(getRunModelLabel(right));
}

function isConfiguredModel(model: RunModel) {
  return (
    model.id.startsWith("configured:") ||
    model.description.includes("当前配置")
  );
}

function makeCurrentModel(
  model: string,
  currentReasoningEffort: string | null,
): RunModel {
  const reasoningEffort = currentReasoningEffort ?? "unknown";
  return {
    id: `current:${model}`,
    model,
    displayName: model,
    description: "当前 thread 的模型，未出现在 model/list",
    hidden: false,
    supportedReasoningEfforts: currentReasoningEffort
      ? [{ reasoningEffort: currentReasoningEffort, description: "" }]
      : [],
    defaultReasoningEffort: reasoningEffort,
    isDefault: false,
    current: true,
  };
}

export function getSupportedReasoningEfforts(model: RunModel): string[] {
  const efforts = model.supportedReasoningEfforts
    .map((option) => option.reasoningEffort)
    .filter((effort): effort is string => Boolean(effort));
  if (efforts.length > 0) {
    return efforts;
  }
  return [model.defaultReasoningEffort];
}

export function resolveReasoningEffortForModel(
  model: RunModel,
  currentEffort: string | null,
) {
  const supportedEfforts = getSupportedReasoningEfforts(model);
  if (currentEffort && supportedEfforts.includes(currentEffort)) {
    return currentEffort;
  }
  if (supportedEfforts.includes(model.defaultReasoningEffort)) {
    return model.defaultReasoningEffort;
  }
  return supportedEfforts[0] ?? model.defaultReasoningEffort;
}

export function resolveSelectionForModel(
  model: RunModel,
  currentEffort: string | null,
): RunConfigSelection {
  return {
    model: model.model,
    reasoningEffort: resolveReasoningEffortForModel(model, currentEffort),
  };
}
