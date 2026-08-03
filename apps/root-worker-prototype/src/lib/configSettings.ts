import type {
  ConfigBatchWriteParams,
  ConfigLayer,
  ConfigLayerMetadata,
  ConfigReadResponse,
  JsonConfigValue,
} from "../types";

export type ConfigFieldKind = "text" | "select";

export type ConfigFieldDefinition = {
  keyPath: string;
  label: string;
  section: "model" | "execution" | "desktop";
  kind: ConfigFieldKind;
  options?: Array<{ value: string; label: string }>;
  placeholder?: string;
  unsetLabel?: string;
};

export type ConfigFieldState = ConfigFieldDefinition & {
  effectiveValue: string;
  draftValue: string;
  originLabel: string;
  isUnsupported: boolean;
  unsupportedValue: string | null;
};

export type SettingsConfigState = {
  fields: ConfigFieldState[];
  userConfigPath: string | null;
  userVersion: string | null;
};

const UNSET_VALUE = "__codex_unset__";

export const SUPPORTED_CONFIG_FIELDS: ConfigFieldDefinition[] = [
  {
    keyPath: "model",
    label: "Model",
    section: "model",
    kind: "text",
    placeholder: "Provider default",
    unsetLabel: "Provider default",
  },
  {
    keyPath: "model_provider",
    label: "Model provider",
    section: "model",
    kind: "text",
    placeholder: "Configured provider",
    unsetLabel: "Configured provider",
  },
  {
    keyPath: "model_reasoning_effort",
    label: "Reasoning effort",
    section: "model",
    kind: "select",
    unsetLabel: "Model default",
    options: [
      { value: "none", label: "None" },
      { value: "minimal", label: "Minimal" },
      { value: "low", label: "Low" },
      { value: "medium", label: "Medium" },
      { value: "high", label: "High" },
      { value: "xhigh", label: "XHigh" },
      { value: "max", label: "Max" },
      { value: "ultra", label: "Ultra" },
    ],
  },
  {
    keyPath: "model_verbosity",
    label: "Verbosity",
    section: "model",
    kind: "select",
    unsetLabel: "Model default",
    options: [
      { value: "low", label: "Low" },
      { value: "medium", label: "Medium" },
      { value: "high", label: "High" },
    ],
  },
  {
    keyPath: "approval_policy",
    label: "Approval policy",
    section: "execution",
    kind: "select",
    unsetLabel: "Config default",
    options: [
      { value: "untrusted", label: "Untrusted" },
      { value: "on-failure", label: "On failure" },
      { value: "on-request", label: "On request" },
      { value: "never", label: "Never" },
    ],
  },
  {
    keyPath: "sandbox_mode",
    label: "Sandbox mode",
    section: "execution",
    kind: "select",
    unsetLabel: "Config default",
    options: [
      { value: "read-only", label: "Read only" },
      { value: "workspace-write", label: "Workspace write" },
      { value: "danger-full-access", label: "Danger full access" },
    ],
  },
  {
    keyPath: "web_search",
    label: "Web search",
    section: "execution",
    kind: "select",
    unsetLabel: "Config default",
    options: [
      { value: "disabled", label: "Disabled" },
      { value: "cached", label: "Cached" },
      { value: "live", label: "Live" },
    ],
  },
  {
    keyPath: "desktop.appearanceTheme",
    label: "Appearance theme",
    section: "desktop",
    kind: "select",
    unsetLabel: "Unset",
    options: [
      { value: "system", label: "System" },
      { value: "light", label: "Light" },
      { value: "dark", label: "Dark" },
    ],
  },
];

export const CONFIG_SECTION_LABELS: Record<
  ConfigFieldDefinition["section"],
  string
> = {
  model: "Model",
  execution: "Execution",
  desktop: "Desktop",
};

export function buildSettingsConfigState(
  response: ConfigReadResponse,
): SettingsConfigState {
  return {
    fields: SUPPORTED_CONFIG_FIELDS.map((definition) =>
      buildFieldState(definition, response),
    ),
    userConfigPath: findUserConfigPath(response.layers, response.origins),
    userVersion: findUserConfigVersion(response.layers, response.origins),
  };
}

export function buildConfigSaveParams(
  fields: ConfigFieldState[],
  expectedVersion: string | null,
): ConfigBatchWriteParams | null {
  const edits = fields
    .filter((field) => !field.isUnsupported && field.draftValue !== field.effectiveValue)
    .map((field) => ({
      keyPath: field.keyPath,
      value: valueFromDraft(field.draftValue),
      mergeStrategy: "replace" as const,
    }));

  if (edits.length === 0) {
    return null;
  }

  return {
    edits,
    expectedVersion,
    reloadUserConfig: true,
  };
}

export function updateFieldDraft(
  fields: ConfigFieldState[],
  keyPath: string,
  draftValue: string,
): ConfigFieldState[] {
  return fields.map((field) =>
    field.keyPath === keyPath ? { ...field, draftValue } : field,
  );
}

export function resetFieldDrafts(fields: ConfigFieldState[]): ConfigFieldState[] {
  return fields.map((field) => ({
    ...field,
    draftValue: field.effectiveValue,
  }));
}

export function isSettingsDirty(fields: ConfigFieldState[]) {
  return fields.some(
    (field) => !field.isUnsupported && field.draftValue !== field.effectiveValue,
  );
}

export function getUnsetDraftValue() {
  return UNSET_VALUE;
}

function buildFieldState(
  definition: ConfigFieldDefinition,
  response: ConfigReadResponse,
): ConfigFieldState {
  const value = valueAtPath(response.config, definition.keyPath);
  const scalar = scalarConfigValue(value);
  const isUnset = value === undefined || value === null;
  const supportsValue =
    isUnset ||
    (definition.kind === "text" && scalar !== null) ||
    (definition.kind === "select" &&
      definition.options?.some((option) => option.value === scalar) === true);
  const effectiveValue = isUnset || scalar === null ? UNSET_VALUE : scalar;

  return {
    ...definition,
    effectiveValue,
    draftValue: effectiveValue,
    originLabel: originLabel(response.origins[definition.keyPath]),
    isUnsupported: !supportsValue,
    unsupportedValue: supportsValue ? null : formatConfigValue(value),
  };
}

function valueAtPath(
  root: Record<string, JsonConfigValue | undefined>,
  keyPath: string,
): JsonConfigValue | undefined {
  let current: JsonConfigValue | undefined = root;
  for (const segment of keyPath.split(".")) {
    if (!current || typeof current !== "object" || Array.isArray(current)) {
      return undefined;
    }
    current = current[segment];
  }
  return current;
}

function scalarConfigValue(value: JsonConfigValue | undefined): string | null {
  if (value === undefined || value === null) {
    return null;
  }
  if (typeof value === "string") {
    return value;
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  return null;
}

function valueFromDraft(value: string): JsonConfigValue {
  return value === UNSET_VALUE ? null : value;
}

function originLabel(metadata: ConfigLayerMetadata | undefined) {
  const source = metadata?.name;
  if (!source) {
    return "Default";
  }
  switch (source.type) {
    case "user":
      return source.profile ? `User profile ${source.profile}` : "User";
    case "project":
      return "Project";
    case "system":
      return "System";
    case "mdm":
    case "legacyManagedConfigTomlFromFile":
    case "legacyManagedConfigTomlFromMdm":
      return "Managed";
    case "sessionFlags":
      return "Session";
    default:
      return "Config";
  }
}

function findUserConfigPath(
  layers: ConfigLayer[] | null,
  origins: Record<string, ConfigLayerMetadata | undefined>,
) {
  const userLayer = layers?.find((layer) => layer.name.type === "user");
  if (userLayer?.name.type === "user") {
    return userLayer.name.file;
  }
  for (const metadata of Object.values(origins)) {
    if (metadata?.name.type === "user") {
      return metadata.name.file;
    }
  }
  return null;
}

function findUserConfigVersion(
  layers: ConfigLayer[] | null,
  origins: Record<string, ConfigLayerMetadata | undefined>,
) {
  const userLayer = layers?.find((layer) => layer.name.type === "user");
  if (userLayer?.name.type === "user") {
    return userLayer.version;
  }
  for (const metadata of Object.values(origins)) {
    if (metadata?.name.type === "user") {
      return metadata.version;
    }
  }
  return null;
}

function formatConfigValue(value: JsonConfigValue | undefined) {
  if (value === undefined) {
    return "Unset";
  }
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}
