import type { RunModel, RunModelListResponse } from "../types";

export type RunConfigSelection = {
  model: string;
  reasoningEffort: string;
};

export function normalizeModelListResponse(
  response: RunModelListResponse,
): RunModel[] {
  return [...(response.data ?? [])]
    .filter((model) => !model.hidden)
    .sort((left, right) => {
      if (left.isDefault !== right.isDefault) {
        return left.isDefault ? -1 : 1;
      }
      return getRunModelLabel(left).localeCompare(getRunModelLabel(right));
    });
}

export function getRunModelLabel(model: RunModel) {
  return model.displayName || model.model || model.id;
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
