import type { RunModel, RunModelListResponse } from "../types";

export type RunConfigSelection = {
  model: string;
  modelProvider: string | null;
  reasoningEffort: string;
  contextWindow: number | null;
  maxContextWindow: number | null;
  autoCompactTokenLimit: number | null;
};

export function normalizeModelListResponse(
  response: RunModelListResponse,
): RunModel[] {
  return RunModelCatalog.fromResponse(response).models();
}

export function ensureCurrentModelVisible(
  models: RunModel[],
  currentModel: string | null,
  currentModelProvider: string | null,
  currentReasoningEffort: string | null,
): RunModel[] {
  return RunModelCatalog.fromModels(models)
    .ensureCurrentModel({
      model: currentModel,
      modelProvider: currentModelProvider,
      reasoningEffort: currentReasoningEffort,
    })
    .models();
}

export function getRunModelLabel(model: RunModel) {
  const label = model.displayName || model.model || model.id;
  return model.modelProvider ? `${label} · ${model.modelProvider}` : label;
}

export function getRunModelKey(model: RunModel) {
  return `${model.modelProvider ?? ""}:${model.model}`;
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
    model.id.startsWith("configured:") || model.description.includes("当前配置")
  );
}

function makeCurrentModel(
  model: string,
  currentModelProvider: string | null,
  currentReasoningEffort: string | null,
): RunModel {
  const reasoningEffort = currentReasoningEffort ?? "unknown";
  return {
    id: `current:${currentModelProvider ?? ""}:${model}`,
    model,
    modelProvider: currentModelProvider,
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
  return RunModelEffortOptions.fromModel(model).supportedEfforts();
}

export function resolveReasoningEffortForModel(
  model: RunModel,
  currentEffort: string | null,
) {
  return RunModelEffortOptions.fromModel(model).resolve(currentEffort);
}

export function resolveSelectionForModel(
  model: RunModel,
  currentEffort: string | null,
): RunConfigSelection {
  return {
    model: model.model,
    modelProvider: model.modelProvider ?? null,
    reasoningEffort: resolveReasoningEffortForModel(model, currentEffort),
    contextWindow: model.contextWindow ?? null,
    maxContextWindow: model.maxContextWindow ?? null,
    autoCompactTokenLimit: model.autoCompactTokenLimit ?? null,
  };
}

export function isSameRunModel(
  model: RunModel,
  currentModel: string | null,
  currentModelProvider: string | null,
) {
  return (
    model.model === currentModel &&
    (model.modelProvider ?? null) === currentModelProvider
  );
}

type CurrentRunModelTarget = {
  model: string | null;
  modelProvider: string | null;
  reasoningEffort: string | null;
};

class RunModelCatalog {
  private constructor(private readonly values: RunModel[]) {}

  static fromResponse(response: RunModelListResponse) {
    return new RunModelCatalog(response.data ?? []).normalized();
  }

  static fromModels(models: RunModel[]) {
    return new RunModelCatalog(models);
  }

  models() {
    return this.values;
  }

  ensureCurrentModel(target: CurrentRunModelTarget) {
    if (!target.model || this.hasModel(target.model, target.modelProvider)) {
      return this;
    }

    return new RunModelCatalog([
      makeCurrentModel(
        target.model,
        target.modelProvider,
        target.reasoningEffort,
      ),
      ...this.values,
    ]).sorted();
  }

  private normalized() {
    return new RunModelCatalog(
      this.values
        .map((model) => ({
          ...model,
          configured: model.configured ?? isConfiguredModel(model),
        }))
        .filter((model) => !model.hidden),
    ).sorted();
  }

  private hasModel(model: string, modelProvider: string | null) {
    return this.values.some((entry) =>
      isSameRunModel(entry, model, modelProvider),
    );
  }

  private sorted() {
    return new RunModelCatalog([...this.values].sort(compareRunModels));
  }
}

class RunModelEffortOptions {
  private constructor(
    private readonly defaultEffort: string,
    private readonly efforts: string[],
  ) {}

  static fromModel(model: RunModel) {
    const efforts = model.supportedReasoningEfforts
      .map((option) => option.reasoningEffort)
      .filter((effort): effort is string => Boolean(effort));
    return new RunModelEffortOptions(
      model.defaultReasoningEffort,
      efforts.length > 0 ? efforts : [model.defaultReasoningEffort],
    );
  }

  supportedEfforts() {
    return this.efforts;
  }

  resolve(currentEffort: string | null) {
    if (currentEffort && this.efforts.includes(currentEffort)) {
      return currentEffort;
    }
    if (this.efforts.includes(this.defaultEffort)) {
      return this.defaultEffort;
    }
    return this.efforts[0] ?? this.defaultEffort;
  }
}
