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
  section: "defaults" | "execution" | "desktop";
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
  providerRegistry: ProviderRegistryEntry[];
  modelOptions: ModelOptionEntry[];
  globalSections: SettingsFieldSection[];
  userConfigPath: string | null;
  userVersion: string | null;
};

export type SettingsFieldSection = {
  id: "defaults" | "execution" | "desktop";
  title: string;
  fields: ConfigFieldState[];
};

export type ProviderRegistryEntry = {
  id: string;
  effectiveId: string;
  draftId: string;
  name: string;
  baseUrl: string;
  wireApi: string;
  envKey: string;
  isNew: boolean;
  isDeleted: boolean;
  isReadonly: boolean;
  readonlyReason: string | null;
  raw: Record<string, JsonConfigValue | undefined>;
};

export type ModelOptionEntry = {
  id: string;
  index: number | null;
  model: string;
  provider: string;
  baseUrl: string;
  wireApi: string;
  ak: string;
  envKey: string;
  contextWindow: string;
  maxContextWindow: string;
  autoCompactTokenLimit: string;
  maxTokens: string;
  isNew: boolean;
  isDeleted: boolean;
  raw: Record<string, JsonConfigValue | undefined>;
};

const UNSET_VALUE = "__codex_unset__";
const BUILT_IN_PROVIDER_IDS = new Set([
  "openai",
  "amazon-bedrock",
  "ollama",
  "ollama-chat",
  "lmstudio",
]);
const PROVIDER_ID_PATTERN = /^[a-z0-9][a-z0-9._-]*$/;
const WIRE_API_OPTIONS = new Set([
  "responses",
  "chat_completions",
  "azure_chat_completions",
]);

export const SUPPORTED_CONFIG_FIELDS: ConfigFieldDefinition[] = [
  {
    keyPath: "model",
    label: "Default model",
    section: "defaults",
    kind: "text",
    placeholder: "No global default",
    unsetLabel: "Config default",
  },
  {
    keyPath: "model_provider",
    label: "Default provider",
    section: "defaults",
    kind: "text",
    placeholder: "No global default",
    unsetLabel: "Config default",
  },
  {
    keyPath: "model_reasoning_effort",
    label: "Default reasoning effort",
    section: "defaults",
    kind: "select",
    unsetLabel: "Config default",
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
    label: "Default verbosity",
    section: "defaults",
    kind: "select",
    unsetLabel: "Config default",
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
  SettingsFieldSection["id"],
  string
> = {
  defaults: "Thread Defaults",
  execution: "Execution",
  desktop: "Desktop",
};

export function buildSettingsConfigState(
  response: ConfigReadResponse,
): SettingsConfigState {
  const fields = SUPPORTED_CONFIG_FIELDS.map((definition) =>
    buildFieldState(definition, response),
  );
  return {
    fields,
    providerRegistry: buildProviderRegistryEntries(response),
    modelOptions: buildModelOptionEntries(response),
    globalSections: buildGlobalSettingsSections(fields),
    userConfigPath: findUserConfigPath(response.layers, response.origins),
    userVersion: findUserConfigVersion(response.layers, response.origins),
  };
}

export function buildProviderRegistryEntries(
  response: ConfigReadResponse,
): ProviderRegistryEntry[] {
  const value = response.config.model_providers;
  if (!isJsonObject(value)) {
    return [];
  }
  const inlineModelProviders = inlineModelOptionProviderIds(response);
  return Object.entries(value)
    .filter(
      ([id, provider]) =>
        !BUILT_IN_PROVIDER_IDS.has(id) &&
        isJsonObject(provider) &&
        !isInlineModelOptionProvider(id, provider, inlineModelProviders),
    )
    .map(([id, provider]) => providerEntryFromRaw(id, provider, false));
}

export function buildModelOptionEntries(
  response: ConfigReadResponse,
): ModelOptionEntry[] {
  const value = response.config.model_options;
  if (!Array.isArray(value)) {
    return [];
  }
  return value
    .map((option, index) =>
      isJsonObject(option) ? modelOptionEntryFromRaw(index, option, false) : null,
    )
    .filter((option): option is ModelOptionEntry => option != null);
}

export function buildGlobalSettingsSections(
  fields: ConfigFieldState[],
): SettingsFieldSection[] {
  return (["defaults", "execution", "desktop"] as const)
    .map((section) => ({
      id: section,
      title: CONFIG_SECTION_LABELS[section],
      fields: fields.filter((field) => field.section === section),
    }))
    .filter((section) => section.fields.length > 0);
}

export function buildConfigSaveParams(
  fields: ConfigFieldState[],
  expectedVersion: string | null,
  providerRegistry: ProviderRegistryEntry[] = [],
  modelOptions: ModelOptionEntry[] = [],
): ConfigBatchWriteParams | null {
  const fieldEdits = fields
    .filter((field) => !field.isUnsupported && field.draftValue !== field.effectiveValue)
    .map((field) => ({
      keyPath: field.keyPath,
      value: valueFromDraft(field.draftValue),
      mergeStrategy: "replace" as const,
    }));
  const providerEdits = buildProviderRegistryEdits(providerRegistry);
  const modelOptionEdits = buildModelOptionsEdits(modelOptions);
  const edits = [...fieldEdits, ...providerEdits, ...modelOptionEdits];

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

export function isProviderRegistryDirty(providerRegistry: ProviderRegistryEntry[]) {
  return providerRegistry.some((entry) => providerEntryDirty(entry));
}

export function isModelOptionsDirty(modelOptions: ModelOptionEntry[]) {
  return modelOptions.some((entry) => modelOptionDirty(entry));
}

export function validateSettingsDrafts(
  providerRegistry: ProviderRegistryEntry[],
  modelOptions: ModelOptionEntry[],
): string[] {
  const errors: string[] = [];
  const providerIds = new Set<string>();
  for (const entry of providerRegistry) {
    if (entry.isDeleted) {
      continue;
    }
    const id = entry.draftId.trim();
    if (!id) {
      errors.push("Provider id is required.");
    } else if (!PROVIDER_ID_PATTERN.test(id)) {
      errors.push(`Provider id "${id}" must use lowercase letters, numbers, dots, underscores, or dashes.`);
    } else if (BUILT_IN_PROVIDER_IDS.has(id)) {
      errors.push(`Provider id "${id}" is reserved.`);
    } else if (providerIds.has(id)) {
      errors.push(`Provider id "${id}" is duplicated.`);
    }
    providerIds.add(id);
    if (!entry.name.trim()) {
      errors.push(`Provider "${id || entry.effectiveId}" needs a display name.`);
    }
    if (!entry.baseUrl.trim()) {
      errors.push(`Provider "${id || entry.effectiveId}" needs a base URL.`);
    }
    if (!WIRE_API_OPTIONS.has(entry.wireApi)) {
      errors.push(`Provider "${id || entry.effectiveId}" has an unsupported wire API.`);
    }
  }

  const modelPairs = new Set<string>();
  for (const entry of modelOptions) {
    if (entry.isDeleted) {
      continue;
    }
    const provider = entry.provider.trim();
    const model = entry.model.trim();
    if (!provider) {
      errors.push("Configured model provider is required.");
    } else if (BUILT_IN_PROVIDER_IDS.has(provider) && provider !== "openai") {
      errors.push(`Configured model provider "${provider}" is reserved.`);
    }
    if (!model) {
      errors.push("Configured model id is required.");
    }
    const pair = `${provider}/${model}`;
    if (provider && model && modelPairs.has(pair)) {
      errors.push(`Configured model "${pair}" is duplicated.`);
    }
    modelPairs.add(pair);
    for (const [label, value] of [
      ["context window", entry.contextWindow],
      ["max context window", entry.maxContextWindow],
      ["auto compact token limit", entry.autoCompactTokenLimit],
      ["max tokens", entry.maxTokens],
    ] as const) {
      if (value.trim() && (!/^\d+$/.test(value.trim()) || Number(value.trim()) <= 0)) {
        errors.push(`${model || "Configured model"} ${label} must be positive.`);
      }
    }
    if (entry.wireApi.trim() && !WIRE_API_OPTIONS.has(entry.wireApi)) {
      errors.push(`${model || "Configured model"} has an unsupported wire API.`);
    }
  }

  return errors;
}

export function createProviderRegistryEntry(
  existing: ProviderRegistryEntry[],
): ProviderRegistryEntry {
  const draftId = nextUniqueId("custom-provider", new Set(existing.map((entry) => entry.draftId)));
  return {
    id: `new-provider:${draftId}`,
    effectiveId: draftId,
    draftId,
    name: "Custom Provider",
    baseUrl: "",
    wireApi: "responses",
    envKey: "",
    isNew: true,
    isDeleted: false,
    isReadonly: false,
    readonlyReason: null,
    raw: {},
  };
}

export function createModelHubOptionEntry(
  existing: ModelOptionEntry[],
): ModelOptionEntry {
  const provider = nextUniqueId(
    "modelhub-gpt",
    new Set(existing.map((entry) => entry.provider)),
  );
  return {
    id: `new-model:${provider}`,
    index: null,
    model: "",
    provider,
    baseUrl: "",
    wireApi: "azure_chat_completions",
    ak: "",
    envKey: "",
    contextWindow: "",
    maxContextWindow: "",
    autoCompactTokenLimit: "",
    maxTokens: "",
    isNew: true,
    isDeleted: false,
    raw: {},
  };
}

export function resetProviderRegistryDrafts(
  entries: ProviderRegistryEntry[],
): ProviderRegistryEntry[] {
  return entries
    .filter((entry) => !entry.isNew)
    .map((entry) => providerEntryFromRaw(entry.effectiveId, entry.raw, false));
}

export function resetModelOptionDrafts(
  entries: ModelOptionEntry[],
): ModelOptionEntry[] {
  return entries
    .filter((entry) => !entry.isNew)
    .map((entry, fallbackIndex) =>
      modelOptionEntryFromRaw(entry.index ?? fallbackIndex, entry.raw, false),
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

function providerEntryFromRaw(
  id: string,
  raw: Record<string, JsonConfigValue | undefined>,
  isNew: boolean,
): ProviderRegistryEntry {
  const hasAdvancedFields = Object.keys(raw).some(
    (key) => !["name", "base_url", "wire_api", "env_key"].includes(key),
  );
  return {
    id,
    effectiveId: id,
    draftId: id,
    name: stringValue(raw.name),
    baseUrl: stringValue(raw.base_url),
    wireApi: stringValue(raw.wire_api) || "responses",
    envKey: stringValue(raw.env_key),
    isNew,
    isDeleted: false,
    isReadonly: hasAdvancedFields,
    readonlyReason: hasAdvancedFields ? "Contains advanced provider fields." : null,
    raw: { ...raw },
  };
}

function modelOptionEntryFromRaw(
  index: number,
  raw: Record<string, JsonConfigValue | undefined>,
  isNew: boolean,
): ModelOptionEntry {
  return {
    id: `${index}:${stringValue(raw.provider)}:${stringValue(raw.model)}`,
    index,
    model: stringValue(raw.model),
    provider: stringValue(raw.provider),
    baseUrl: stringValue(raw.base_url),
    wireApi: stringValue(raw.wire_api),
    ak: stringValue(raw.ak),
    envKey: stringValue(raw.env_key),
    contextWindow: stringValue(raw.context_window),
    maxContextWindow: stringValue(raw.max_context_window),
    autoCompactTokenLimit: stringValue(raw.auto_compact_token_limit),
    maxTokens: stringValue(raw.max_tokens),
    isNew,
    isDeleted: false,
    raw: { ...raw },
  };
}

function buildProviderRegistryEdits(providerRegistry: ProviderRegistryEntry[]) {
  return providerRegistry
    .filter((entry) => providerEntryDirty(entry))
    .flatMap((entry) => {
      const oldPath = `model_providers.${entry.effectiveId}`;
      if (entry.isDeleted) {
        return entry.isNew ? [] : [{ keyPath: oldPath, value: null, mergeStrategy: "replace" as const }];
      }
      const nextId = entry.draftId.trim();
      const nextValue = providerValueFromEntry(entry);
      if (!entry.isNew && nextId !== entry.effectiveId) {
        return [
          { keyPath: oldPath, value: null, mergeStrategy: "replace" as const },
          { keyPath: `model_providers.${nextId}`, value: nextValue, mergeStrategy: "replace" as const },
        ];
      }
      return [{ keyPath: `model_providers.${nextId}`, value: nextValue, mergeStrategy: "replace" as const }];
    });
}

function buildModelOptionsEdits(modelOptions: ModelOptionEntry[]) {
  if (!isModelOptionsDirty(modelOptions)) {
    return [];
  }
  return [
    {
      keyPath: "model_options",
      value: modelOptions
        .filter((entry) => !entry.isDeleted)
        .map((entry) => modelOptionValueFromEntry(entry)),
      mergeStrategy: "replace" as const,
    },
  ];
}

function providerValueFromEntry(entry: ProviderRegistryEntry): JsonConfigValue {
  return cleanObject({
    ...entry.raw,
    name: entry.name.trim(),
    base_url: entry.baseUrl.trim(),
    wire_api: entry.wireApi,
    env_key: entry.envKey.trim() || undefined,
  });
}

function modelOptionValueFromEntry(entry: ModelOptionEntry): JsonConfigValue {
  return cleanObject({
    ...entry.raw,
    model: entry.model.trim(),
    provider: entry.provider.trim(),
    base_url: entry.baseUrl.trim() || undefined,
    wire_api: entry.wireApi.trim() || undefined,
    ak: entry.ak.trim() || undefined,
    env_key: entry.envKey.trim() || undefined,
    context_window: numberOrUndefined(entry.contextWindow),
    max_context_window: numberOrUndefined(entry.maxContextWindow),
    auto_compact_token_limit: numberOrUndefined(entry.autoCompactTokenLimit),
    max_tokens: numberOrUndefined(entry.maxTokens),
  });
}

function providerEntryDirty(entry: ProviderRegistryEntry) {
  if (entry.isDeleted) {
    return true;
  }
  if (entry.isReadonly) {
    return false;
  }
  if (!entry.isNew && entry.draftId.trim() !== entry.effectiveId) {
    return true;
  }
  return stableJson(providerValueFromEntry(entry)) !== stableJson(entry.raw);
}

function modelOptionDirty(entry: ModelOptionEntry) {
  if (entry.isDeleted) {
    return true;
  }
  return stableJson(modelOptionValueFromEntry(entry)) !== stableJson(entry.raw);
}

function stringValue(value: JsonConfigValue | undefined): string {
  if (typeof value === "string") {
    return value;
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  return "";
}

function numberOrUndefined(value: string) {
  const trimmed = value.trim();
  return trimmed ? Number(trimmed) : undefined;
}

function cleanObject(
  value: Record<string, JsonConfigValue | undefined>,
): Record<string, JsonConfigValue | undefined> {
  return Object.fromEntries(
    Object.entries(value).filter(([, entryValue]) => entryValue !== undefined && entryValue !== ""),
  );
}

function isJsonObject(value: JsonConfigValue | undefined): value is Record<string, JsonConfigValue | undefined> {
  return value != null && typeof value === "object" && !Array.isArray(value);
}

function inlineModelOptionProviderIds(response: ConfigReadResponse) {
  const ids = new Map<string, Set<string>>();
  const modelOptions = response.config.model_options;
  if (!Array.isArray(modelOptions)) {
    return ids;
  }
  for (const option of modelOptions) {
    if (!isJsonObject(option) || !option.base_url) {
      continue;
    }
    const provider = stringValue(option.provider);
    if (provider) {
      const synthesizedProvider = synthesizedProviderFromModelOption(
        provider,
        option,
      );
      const existing = ids.get(provider) ?? new Set<string>();
      existing.add(stableJson(synthesizedProvider));
      ids.set(provider, existing);
    }
  }
  return ids;
}

function isInlineModelOptionProvider(
  id: string,
  provider: Record<string, JsonConfigValue | undefined>,
  inlineModelProviders: Map<string, Set<string>>,
) {
  return (
    inlineModelProviders
      .get(id)
      ?.has(stableJson(cleanObject({ ...provider }))) === true
  );
}

function synthesizedProviderFromModelOption(
  provider: string,
  option: Record<string, JsonConfigValue | undefined>,
) {
  const queryParams = queryParamsFromModelOption(option);
  return cleanObject({
    name: provider,
    base_url: option.base_url,
    env_key: option.env_key,
    env_key_instructions: option.env_key_instructions,
    experimental_bearer_token: option.experimental_bearer_token,
    wire_api: option.wire_api ?? "azure_chat_completions",
    query_params: queryParams,
    http_headers: option.http_headers,
    env_http_headers: option.env_http_headers,
    request_max_retries: option.request_max_retries,
    stream_max_retries: option.stream_max_retries,
    stream_idle_timeout_ms: option.stream_idle_timeout_ms,
    websocket_connect_timeout_ms: option.websocket_connect_timeout_ms,
  });
}

function queryParamsFromModelOption(
  option: Record<string, JsonConfigValue | undefined>,
) {
  const queryParams = isJsonObject(option.query_params)
    ? { ...option.query_params }
    : {};
  const ak = stringValue(option.ak);
  if (ak) {
    queryParams.ak = ak;
  }
  return Object.keys(queryParams).length > 0 ? queryParams : undefined;
}

function stableJson(value: unknown) {
  return JSON.stringify(sortJson(value));
}

function sortJson(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map((entry) => sortJson(entry));
  }
  if (!value || typeof value !== "object") {
    return value;
  }
  return Object.fromEntries(
    Object.entries(value)
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([key, entryValue]) => [key, sortJson(entryValue)]),
  );
}

function nextUniqueId(base: string, existing: Set<string>) {
  if (!existing.has(base)) {
    return base;
  }
  let suffix = 2;
  while (existing.has(`${base}-${suffix}`)) {
    suffix += 1;
  }
  return `${base}-${suffix}`;
}
