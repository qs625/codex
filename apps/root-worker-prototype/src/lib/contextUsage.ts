import type {
  Thread,
  ThreadContextUsage,
  ThreadItem,
  ThreadSkill,
  ThreadTokenUsage,
} from "../types";

export type ContextUsageCategoryId =
  | "compact"
  | "skillsMetadata"
  | "concreteSkills"
  | "toolsMetadata"
  | "toolCalls"
  | "fileWrites"
  | "fileReads"
  | "commands"
  | "interAgent"
  | "searchMedia"
  | "otherTools"
  | "userMessages"
  | "llmMessages"
  | "reasoning";

export type ContextUsageCategorySummary = {
  id: ContextUsageCategoryId;
  label: string;
  shortLabel: string;
  description: string;
  units: number;
  sharePercent: number;
};

export type LoadedSkillSummary = {
  name: string;
  path: string;
  kind: ThreadSkill["kind"];
  loadCount: number;
};

export type ContextUsageToolBreakdownId =
  | "applyPatch"
  | "fileOperations"
  | "commands"
  | "interAgent"
  | "searchMedia"
  | "otherTools";

export type ContextUsageToolBreakdownSummary = {
  id: ContextUsageToolBreakdownId;
  label: string;
  description: string;
  inputUnits: number;
  outputUnits: number;
  totalUnits: number;
  sharePercent: number;
};

export type ContextUsageTurnTrendCell = {
  turnId: string;
  label: string;
  units: number;
  intensity: number;
};

export type ContextUsageTurnTrendRow = {
  id: ContextUsageCategoryId;
  label: string;
  shortLabel: string;
  color: string;
  cells: ContextUsageTurnTrendCell[];
};

export type ContextUsageAnalysis = {
  hasBudgetData: boolean;
  budgetUsedPercent: number;
  usedTokens: number | null;
  contextWindowTokens: number | null;
  loadedSkills: number;
  totalSkills: number;
  totalConcreteLoads: number;
  reasoningSharePercent: number;
  categories: ContextUsageCategorySummary[];
  toolBreakdown: ContextUsageToolBreakdownSummary[];
  loadedConcreteSkills: LoadedSkillSummary[];
  turnTrend: {
    turns: Array<{
      turnId: string;
      label: string;
    }>;
    rows: ContextUsageTurnTrendRow[];
  };
};

const CATEGORY_ORDER: Array<{
  id: ContextUsageCategoryId;
  label: string;
  shortLabel: string;
  description: string;
}> = [
  {
    id: "compact",
    label: "Compact",
    shortLabel: "Compact",
    description: "Compacted summaries replacing older context",
  },
  {
    id: "skillsMetadata",
    label: "Skill Metadata",
    shortLabel: "Skill Meta",
    description: "Skill names, routing hints, and load directives kept in context",
  },
  {
    id: "concreteSkills",
    label: "Concrete Skills",
    shortLabel: "Skills",
    description: "Injected skill content actually brought into context",
  },
  {
    id: "toolsMetadata",
    label: "Tools Metadata",
    shortLabel: "Tool Meta",
    description: "Tool names, schemas, and affordance overhead",
  },
  {
    id: "toolCalls",
    label: "Tool Inputs & Results",
    shortLabel: "Tool I/O",
    description: "Tool arguments, returned results, and execution output kept in context",
  },
  {
    id: "fileWrites",
    label: "File Writes",
    shortLabel: "Writes",
    description: "Patch inputs and file write results kept in context",
  },
  {
    id: "fileReads",
    label: "File Reads",
    shortLabel: "Reads",
    description: "File inspection, reads, diffs, and listings kept in context",
  },
  {
    id: "commands",
    label: "Commands",
    shortLabel: "Cmd",
    description: "Shell command inputs, test output, build output, and git output",
  },
  {
    id: "interAgent",
    label: "Inter-Agent",
    shortLabel: "Agents",
    description: "Agent handoffs, messages, status, and collaboration tools",
  },
  {
    id: "searchMedia",
    label: "Search & Media",
    shortLabel: "Search",
    description: "Web search, image, screenshot, and media tool traffic",
  },
  {
    id: "otherTools",
    label: "Other Tools",
    shortLabel: "Other",
    description: "Tool traffic that did not match a specific bucket",
  },
  {
    id: "userMessages",
    label: "User Messages",
    shortLabel: "User",
    description: "Prompt and attachment payloads",
  },
  {
    id: "llmMessages",
    label: "LLM Messages",
    shortLabel: "LLM",
    description: "Assistant messages, plans, and summaries",
  },
  {
    id: "reasoning",
    label: "Reasoning",
    shortLabel: "Reason",
    description: "Reasoning summaries and internal thought traces",
  },
];

const CATEGORY_COLORS: Record<ContextUsageCategoryId, string> = {
  compact: "#9a8f7b",
  skillsMetadata: "#c0841a",
  concreteSkills: "#d97706",
  toolsMetadata: "#0f766e",
  toolCalls: "#0d9488",
  fileWrites: "#b45309",
  fileReads: "#0284c7",
  commands: "#16a34a",
  interAgent: "#7c3aed",
  searchMedia: "#db2777",
  otherTools: "#64748b",
  userMessages: "#4f46e5",
  llmMessages: "#6366f1",
  reasoning: "#8b5cf6",
};

const MAX_TREND_TURNS = 16;

const TOOL_BREAKDOWN_ORDER: Array<{
  id: ContextUsageToolBreakdownId;
  categoryId: ContextUsageCategoryId;
  label: string;
  description: string;
}> = [
  {
    id: "applyPatch",
    categoryId: "fileWrites",
    label: "File Writes",
    description: "Patch inputs and patch application results",
  },
  {
    id: "fileOperations",
    categoryId: "fileReads",
    label: "File Reads",
    description: "File inspection, reads, diffs, and listings",
  },
  {
    id: "commands",
    categoryId: "commands",
    label: "Commands",
    description: "Shell commands, tests, builds, and git actions",
  },
  {
    id: "interAgent",
    categoryId: "interAgent",
    label: "Inter-Agent",
    description: "Agent handoffs, messages, status, and collaboration tools",
  },
  {
    id: "searchMedia",
    categoryId: "searchMedia",
    label: "Search & Media",
    description: "Web search, image, screenshot, and media tools",
  },
  {
    id: "otherTools",
    categoryId: "otherTools",
    label: "Other Tools",
    description: "Tool traffic that did not match a specific bucket",
  },
];

export function getContextUsageCategoryColor(categoryId: ContextUsageCategoryId) {
  return CATEGORY_COLORS[categoryId];
}

export function buildContextUsageAnalysis(
  thread: Thread | null,
  totalSkillMetadataCount: number,
  modelContextWindowOverride?: number | null,
): ContextUsageAnalysis {
  if (!thread) {
    return {
      hasBudgetData: false,
      budgetUsedPercent: 0,
      usedTokens: null,
      contextWindowTokens: null,
      loadedSkills: 0,
      totalSkills: totalSkillMetadataCount,
      totalConcreteLoads: 0,
      reasoningSharePercent: 0,
      categories: buildCategorySummaries(
        initializeCategoryUnits(),
        0,
        0,
        0,
        0,
        false,
      ),
      toolBreakdown: [],
      loadedConcreteSkills: [],
      turnTrend: {
        turns: [],
        rows: CATEGORY_ORDER.map((category) => ({
          id: category.id,
          label: category.label,
          shortLabel: category.shortLabel,
          color: getContextUsageCategoryColor(category.id),
          cells: [],
        })),
      },
    };
  }

  const tokenUsage = thread.threadUsage?.tokenUsage ?? thread.tokenUsage;
  const contextUsage = thread.threadUsage?.contextUsage ?? thread.contextUsage;
  const { skillLoads, turnTrend } = collectThreadUsage(thread);

  if (contextUsage) {
    const loadedConcreteSkills = mergeLoadedSkills(skillLoads, contextUsage);
    const totalConcreteLoads = loadedConcreteSkills.reduce((sum, skill) => sum + skill.loadCount, 0);
    return buildContextUsageAnalysisFromBackend(
      contextUsage,
      tokenUsage,
      totalSkillMetadataCount,
      loadedConcreteSkills,
      totalConcreteLoads,
      turnTrend,
      modelContextWindowOverride,
    );
  }

  const loadedConcreteSkills = [...skillLoads.values()]
    .map((skill) => ({
      ...skill,
      loadCount: Math.max(skill.loadCount, 1),
    }))
    .sort((left, right) => right.loadCount - left.loadCount || left.name.localeCompare(right.name));
  const totalConcreteLoads = loadedConcreteSkills.reduce((sum, skill) => sum + skill.loadCount, 0);
  const normalizedTotalSkills = Math.max(totalSkillMetadataCount, loadedConcreteSkills.length);

  return {
    hasBudgetData: hasContextWindow(tokenUsage, modelContextWindowOverride),
    budgetUsedPercent: budgetPercentFromTokenUsage(
      tokenUsage,
      modelContextWindowOverride,
    ),
    usedTokens: usedTokensFromTokenUsage(tokenUsage),
    contextWindowTokens: contextWindowTokensFromTokenUsage(
      tokenUsage,
      modelContextWindowOverride,
    ),
    loadedSkills: loadedConcreteSkills.length,
    totalSkills: normalizedTotalSkills,
    totalConcreteLoads,
    reasoningSharePercent: 0,
    categories: buildCategorySummaries(
      initializeCategoryUnits(),
      0,
      0,
      0,
      0,
      false,
    ),
    toolBreakdown: [],
    loadedConcreteSkills,
    turnTrend,
  };
}

function buildContextUsageAnalysisFromBackend(
  contextUsage: ThreadContextUsage,
  tokenUsage: ThreadTokenUsage | null | undefined,
  totalSkillMetadataCount: number,
  loadedConcreteSkills: LoadedSkillSummary[],
  totalConcreteLoads: number,
  turnTrend: ContextUsageAnalysis["turnTrend"],
  modelContextWindowOverride?: number | null,
): ContextUsageAnalysis {
  const rawCategoryUnits = initializeCategoryUnits();
  rawCategoryUnits.compact = contextUsage.categories.compact;
  rawCategoryUnits.skillsMetadata = contextUsage.categories.skillsMetadata;
  rawCategoryUnits.concreteSkills = contextUsage.categories.concreteSkills;
  rawCategoryUnits.toolsMetadata = contextUsage.categories.toolsMetadata;
  const toolBucketUnits = buildToolCategoryUnits(contextUsage);
  if (toolBucketUnits) {
    for (const [categoryId, units] of Object.entries(toolBucketUnits) as Array<
      [ContextUsageCategoryId, number]
    >) {
      rawCategoryUnits[categoryId] = units;
    }
  } else {
    rawCategoryUnits.toolCalls = contextUsage.categories.toolCalls;
  }
  rawCategoryUnits.userMessages = contextUsage.categories.userMessages;
  rawCategoryUnits.llmMessages = contextUsage.categories.llmMessages;
  rawCategoryUnits.reasoning = contextUsage.categories.reasoning;

  const totalUnits = sumCategoryUnits(rawCategoryUnits);
  const totalUsedTokens = categoryDistributionTokensFromTokenUsage(tokenUsage);
  const lastUsedTokens = usedTokensFromTokenUsage(tokenUsage) ?? 0;
  const contextWindowTokens =
    contextWindowTokensFromTokenUsage(tokenUsage, modelContextWindowOverride) ?? 0;
  const categories = buildCategorySummaries(
    rawCategoryUnits,
    totalUnits,
    totalUsedTokens,
    lastUsedTokens,
    contextWindowTokens,
    toolBucketUnits != null,
  );
  const reasoningSharePercent = categories.find((category) => category.id === "reasoning")?.sharePercent ?? 0;
  const normalizedTotalSkills =
    contextUsage.loadedSkills.totalCount ?? Math.max(totalSkillMetadataCount, loadedConcreteSkills.length);

  return {
    hasBudgetData: hasContextWindow(tokenUsage, modelContextWindowOverride),
    budgetUsedPercent: budgetPercentFromTokenUsage(
      tokenUsage,
      modelContextWindowOverride,
    ),
    usedTokens: usedTokensFromTokenUsage(tokenUsage),
    contextWindowTokens: contextWindowTokensFromTokenUsage(
      tokenUsage,
      modelContextWindowOverride,
    ),
    loadedSkills: Math.max(contextUsage.loadedSkills.loadedCount, loadedConcreteSkills.length),
    totalSkills: normalizedTotalSkills,
    totalConcreteLoads,
    reasoningSharePercent,
    categories,
    toolBreakdown: [],
    loadedConcreteSkills,
    turnTrend,
  };
}

function buildToolCategoryUnits(
  contextUsage: ThreadContextUsage,
): Partial<Record<ContextUsageCategoryId, number>> | null {
  const raw = contextUsage.toolBreakdown;
  if (!raw) {
    return null;
  }

  const rows = TOOL_BREAKDOWN_ORDER.map((bucket) => {
    const value = raw[bucket.id];
    const inputUnits = sanitizeUnitCount(value?.input);
    const outputUnits = sanitizeUnitCount(value?.output);
    return {
      ...bucket,
      inputUnits,
      outputUnits,
      totalUnits: inputUnits + outputUnits,
    };
  });
  const totalUnits = rows.reduce((sum, row) => sum + row.totalUnits, 0);
  if (totalUnits <= 0) {
    return null;
  }

  const toolCallUnits = sanitizeUnitCount(contextUsage.categories.toolCalls);
  if (toolCallUnits <= 0) {
    return null;
  }
  const nonZeroRows = rows.filter((row) => row.totalUnits > 0);
  let allocatedUnits = 0;
  return nonZeroRows.reduce<Partial<Record<ContextUsageCategoryId, number>>>(
    (unitsByCategory, row, index) => {
      const isLastRow = index === nonZeroRows.length - 1;
      const units = isLastRow
        ? Math.max(0, toolCallUnits - allocatedUnits)
        : Math.round((row.totalUnits / totalUnits) * toolCallUnits);
      allocatedUnits += units;
      unitsByCategory[row.categoryId] = units;
      return unitsByCategory;
    },
    {},
  );
}

function buildCategorySummaries(
  rawCategoryUnits: Record<ContextUsageCategoryId, number>,
  totalUnits: number,
  totalUsedTokens: number,
  lastUsedTokens: number,
  contextWindowTokens: number,
  hasToolBuckets: boolean,
): ContextUsageCategorySummary[] {
  return CATEGORY_ORDER.filter((category) =>
    shouldIncludeCategory(category.id, rawCategoryUnits[category.id], hasToolBuckets),
  ).map((category) => {
    const units = rawCategoryUnits[category.id];
    const mixSharePercent = totalUnits > 0 ? roundPercent((units / totalUnits) * 100) : 0;
    const categoryTokens = totalUsedTokens > 0 ? Math.round((mixSharePercent / 100) * totalUsedTokens) : 0;
    const lastCategoryTokens =
      lastUsedTokens > 0 ? Math.round((mixSharePercent / 100) * lastUsedTokens) : 0;
    const sharePercent =
      contextWindowTokens > 0 && lastCategoryTokens > 0
        ? roundPercent((lastCategoryTokens / contextWindowTokens) * 100)
        : mixSharePercent;
    return {
      ...category,
      units: categoryTokens,
      sharePercent,
    };
  });
}

function shouldIncludeCategory(
  categoryId: ContextUsageCategoryId,
  units: number,
  hasToolBuckets: boolean,
) {
  if (categoryId === "toolCalls" && hasToolBuckets) {
    return false;
  }
  if (isToolBucketCategory(categoryId)) {
    return units > 0;
  }
  return true;
}

function isToolBucketCategory(categoryId: ContextUsageCategoryId) {
  return (
    categoryId === "fileWrites" ||
    categoryId === "fileReads" ||
    categoryId === "commands" ||
    categoryId === "interAgent" ||
    categoryId === "searchMedia" ||
    categoryId === "otherTools"
  );
}

function sanitizeUnitCount(value: number | null | undefined) {
  return Number.isFinite(value) && value && value > 0 ? Math.round(value) : 0;
}

function initializeCategoryUnits(): Record<ContextUsageCategoryId, number> {
  return {
    compact: 0,
    skillsMetadata: 0,
    concreteSkills: 0,
    toolsMetadata: 0,
    toolCalls: 0,
    fileWrites: 0,
    fileReads: 0,
    commands: 0,
    interAgent: 0,
    searchMedia: 0,
    otherTools: 0,
    userMessages: 0,
    llmMessages: 0,
    reasoning: 0,
  };
}

function mergeLoadedSkills(
  skillLoads: Map<string, LoadedSkillSummary>,
  contextUsage: ThreadContextUsage,
) {
  const merged = new Map<string, LoadedSkillSummary>();

  for (const skill of skillLoads.values()) {
    merged.set(skill.path, {
      ...skill,
      loadCount: Math.max(skill.loadCount, 1),
    });
  }

  for (const skill of contextUsage.loadedSkills.skills ?? []) {
    merged.set(skill.path, {
      name: skill.name,
      path: skill.path,
      kind: skill.kind,
      loadCount: Math.max(skill.loadCount, merged.get(skill.path)?.loadCount ?? 0, 1),
    });
  }

  return [...merged.values()].sort(
    (left, right) => right.loadCount - left.loadCount || left.name.localeCompare(right.name),
  );
}

function buildTurnTrend(thread: Thread) {
  return collectThreadUsage(thread).turnTrend;
}

function collectThreadUsage(thread: Thread) {
  const categoryUnits = initializeCategoryUnits();
  const turnCategoryUnits: Array<{
    turnId: string;
    label: string;
    units: Record<ContextUsageCategoryId, number>;
  }> = [];
  const skillLoads = new Map<string, LoadedSkillSummary>();

  for (const skill of thread.skills) {
    skillLoads.set(skill.path, {
      name: skill.name,
      path: skill.path,
      kind: skill.kind,
      loadCount: 0,
    });
  }

  thread.turns.forEach((turn, index) => {
    const perTurnUnits = initializeCategoryUnits();

    for (const item of turn.items) {
      accumulateItemUnits(item, perTurnUnits, skillLoads);
    }

    for (const category of CATEGORY_ORDER) {
      categoryUnits[category.id] += perTurnUnits[category.id];
    }

    turnCategoryUnits.push({
      turnId: turn.id,
      label: String(index + 1),
      units: perTurnUnits,
    });
  });

  return {
    categoryUnits,
    skillLoads,
    turnTrend: buildTurnTrendRows(turnCategoryUnits),
  };
}

function buildTurnTrendRows(
  turns: Array<{
    turnId: string;
    label: string;
    units: Record<ContextUsageCategoryId, number>;
  }>,
) {
  const visibleTurns = turns.slice(-MAX_TREND_TURNS);
  const trendCategories = CATEGORY_ORDER.filter(
    (category) =>
      !isToolBucketCategory(category.id) ||
      visibleTurns.some((turn) => turn.units[category.id] > 0),
  );
  const maxCategoryUnits = Math.max(
    ...visibleTurns.flatMap((turn) =>
      trendCategories.map((category) => turn.units[category.id]),
    ),
    0,
  );

  return {
    turns: visibleTurns.map((turn) => ({
      turnId: turn.turnId,
      label: turn.label,
    })),
    rows: trendCategories.map((category) => ({
      id: category.id,
      label: category.label,
      shortLabel: category.shortLabel,
      color: getContextUsageCategoryColor(category.id),
      cells: visibleTurns.map((turn) => {
        const units = turn.units[category.id];
        return {
          turnId: turn.turnId,
          label: turn.label,
          units,
          intensity: maxCategoryUnits > 0 ? units / maxCategoryUnits : 0,
        };
      }),
    })),
  };
}

function sumCategoryUnits(units: Record<ContextUsageCategoryId, number>) {
  return Object.values(units).reduce((sum, value) => sum + value, 0);
}

function accumulateItemUnits(
  item: ThreadItem,
  units: Record<ContextUsageCategoryId, number>,
  skillLoads: Map<string, LoadedSkillSummary>,
) {
  if (item.type === "userMessage") {
    for (const content of item.content) {
      if (content.type === "text") {
        units.userMessages += estimateTextUnits(content.text);
        continue;
      }

      if (content.type === "skill") {
        units.skillsMetadata += 36;
        units.concreteSkills += 120;
        if (!content.path) {
          continue;
        }
        const existing = skillLoads.get(content.path);
        if (existing) {
          existing.loadCount += 1;
        } else {
          skillLoads.set(content.path, {
            name: content.name ?? "Unnamed skill",
            path: content.path,
            kind: "explicit",
            loadCount: 1,
          });
        }
        continue;
      }

      if (content.type === "image") {
        units.userMessages += 48;
      }
    }
    return;
  }

  if (item.type === "agentMessage") {
    units.llmMessages += estimateTextUnits(item.text);
    return;
  }

  if (item.type === "plan") {
    units.llmMessages += estimateTextUnits(item.text);
    return;
  }

  if (item.type === "reasoning") {
    units.reasoning += estimateTextUnits(item.summary.join("\n"));
    units.reasoning += estimateTextUnits(item.content.join("\n"));
    return;
  }

  if (item.type === "injectedContext") {
    for (const section of item.sections) {
      if (!section.label.startsWith("Skill: ")) {
        continue;
      }
      units.concreteSkills += estimateTextUnits(section.label) + estimateTextUnits(section.text);
    }
    return;
  }

  if (item.type === "commandExecution") {
    units.toolsMetadata += 28;
    units.toolCalls += estimateTextUnits(item.command) + estimateTextUnits(item.cwd);
    units.toolCalls += estimateTextUnits(item.aggregatedOutput);
    return;
  }

  if (item.type === "dynamicToolCall") {
    units.toolsMetadata += 30 + estimateTextUnits(item.namespace);
    units.toolCalls += estimateObjectUnits(item.arguments) + estimateObjectUnits(item.contentItems) + 24;
    return;
  }

  if (item.type === "builtinToolCall") {
    units.toolsMetadata += 24;
    units.toolCalls += estimateObjectUnits(item.arguments) + estimateObjectUnits(item.output) + 20;
    return;
  }

  if (item.type === "eventCommandCall") {
    units.toolsMetadata += 30;
    units.toolCalls +=
      estimateTextUnits(item.command) +
      estimateTextUnits(item.cwd) +
      estimateTextUnits(item.label) +
      estimateObjectUnits(item.output) +
      24;
    return;
  }

  if (item.type === "eventCommandEvent") {
    units.toolsMetadata += 18;
    units.toolCalls +=
      estimateTextUnits(item.command) +
      estimateTextUnits(item.cwd) +
      estimateTextUnits(item.label) +
      estimateTextUnits(item.line) +
      estimateTextUnits(item.message) +
      estimateTextUnits(item.signal) +
      20;
    return;
  }

  if (item.type === "mcpToolCall") {
    units.toolsMetadata += 32 + estimateTextUnits(item.server);
    units.toolCalls += estimateObjectUnits(item.arguments) + estimateObjectUnits(item.result) + estimateObjectUnits(item.error);
    return;
  }

  if (item.type === "collabAgentToolCall") {
    units.toolsMetadata += 30;
    units.toolCalls += estimateTextUnits(item.tool) + estimateTextUnits(item.prompt) + estimateObjectUnits(item.agentsStates);
    return;
  }

  if (item.type === "collabAgentMessage") {
    units.toolCalls += estimateTextUnits(item.content) + estimateTextUnits(item.operation) + 32;
    return;
  }

  if (item.type === "collabAgentStatusUpdate") {
    units.toolCalls += estimateObjectUnits(item.status) + 24;
    return;
  }

  if (item.type === "fileChange") {
    units.toolCalls += item.changes.length * 26;
    return;
  }

  if (item.type === "webSearch") {
    units.toolsMetadata += 18;
    units.toolCalls += estimateTextUnits(item.query) + estimateTextUnits(item.action) + 24;
    return;
  }

  if (item.type === "imageGeneration") {
    units.toolCalls += estimateTextUnits(item.revisedPrompt) + estimateTextUnits(item.result) + 36;
    return;
  }

  if (item.type === "imageView") {
    units.toolCalls += estimateTextUnits(item.path) + 16;
    return;
  }
}

function estimateTextUnits(value: unknown) {
  if (typeof value !== "string" || value.length === 0) {
    return 0;
  }

  return value.trim().length;
}

function estimateObjectUnits(value: unknown) {
  if (value == null) {
    return 0;
  }

  try {
    return JSON.stringify(value).length;
  } catch {
    return 0;
  }
}

function roundPercent(value: number) {
  return Math.round(value * 10) / 10;
}

function sanitizePercent(value: number | null | undefined) {
  if (!Number.isFinite(value)) {
    return 0;
  }
  return Math.max(0, Math.min(100, roundPercent(value ?? 0)));
}

function budgetPercentFromTokenUsage(
  tokenUsage: ThreadTokenUsage | null | undefined,
  modelContextWindowOverride?: number | null,
) {
  const totalTokens = usedTokensFromTokenUsage(tokenUsage) ?? 0;
  const modelContextWindow =
    contextWindowTokensFromTokenUsage(tokenUsage, modelContextWindowOverride) ?? 0;

  if (modelContextWindow <= 0 || totalTokens <= 0) {
    return 0;
  }

  return sanitizePercent((totalTokens / modelContextWindow) * 100);
}

function hasContextWindow(
  tokenUsage: ThreadTokenUsage | null | undefined,
  modelContextWindowOverride?: number | null,
) {
  return (
    contextWindowTokensFromTokenUsage(tokenUsage, modelContextWindowOverride) ??
    0
  ) > 0;
}

function usedTokensFromTokenUsage(tokenUsage: ThreadTokenUsage | null | undefined) {
  const totalTokens = tokenUsage?.last.totalTokens ?? 0;
  return totalTokens > 0 ? totalTokens : null;
}

function categoryDistributionTokensFromTokenUsage(
  tokenUsage: ThreadTokenUsage | null | undefined,
) {
  const totalTokens = tokenUsage?.total.totalTokens ?? 0;
  return totalTokens > 0 ? totalTokens : 0;
}

function contextWindowTokensFromTokenUsage(
  tokenUsage: ThreadTokenUsage | null | undefined,
  modelContextWindowOverride?: number | null,
) {
  if (modelContextWindowOverride && modelContextWindowOverride > 0) {
    return modelContextWindowOverride;
  }
  const modelContextWindow = tokenUsage?.modelContextWindow ?? 0;
  return modelContextWindow > 0 ? modelContextWindow : null;
}
