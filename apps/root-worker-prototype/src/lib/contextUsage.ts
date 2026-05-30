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
  userMessages: "#4f46e5",
  llmMessages: "#6366f1",
  reasoning: "#8b5cf6",
};

const MAX_TREND_TURNS = 16;

export function getContextUsageCategoryColor(categoryId: ContextUsageCategoryId) {
  return CATEGORY_COLORS[categoryId];
}

export function buildContextUsageAnalysis(
  thread: Thread | null,
  totalSkillMetadataCount: number,
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
      categories: CATEGORY_ORDER.map((category) => ({
        ...category,
        units: 0,
        sharePercent: 0,
      })),
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
    hasBudgetData: hasTokenUsage(tokenUsage),
    budgetUsedPercent: budgetPercentFromTokenUsage(tokenUsage),
    usedTokens: usedTokensFromTokenUsage(tokenUsage),
    contextWindowTokens: contextWindowTokensFromTokenUsage(tokenUsage),
    loadedSkills: loadedConcreteSkills.length,
    totalSkills: normalizedTotalSkills,
    totalConcreteLoads,
    reasoningSharePercent: 0,
    categories: CATEGORY_ORDER.map((category) => ({
      ...category,
      units: 0,
      sharePercent: 0,
    })),
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
): ContextUsageAnalysis {
  const rawCategoryUnits = initializeCategoryUnits();
  rawCategoryUnits.compact = contextUsage.categories.compact;
  rawCategoryUnits.skillsMetadata = contextUsage.categories.skillsMetadata;
  rawCategoryUnits.concreteSkills = contextUsage.categories.concreteSkills;
  rawCategoryUnits.toolsMetadata = contextUsage.categories.toolsMetadata;
  rawCategoryUnits.toolCalls = contextUsage.categories.toolCalls;
  rawCategoryUnits.userMessages = contextUsage.categories.userMessages;
  rawCategoryUnits.llmMessages = contextUsage.categories.llmMessages;
  rawCategoryUnits.reasoning = contextUsage.categories.reasoning;

  const totalUnits = sumCategoryUnits(rawCategoryUnits);
  const totalUsedTokens = usedTokensFromTokenUsage(tokenUsage) ?? 0;
  const contextWindowTokens = contextWindowTokensFromTokenUsage(tokenUsage) ?? 0;
  const categories = CATEGORY_ORDER.map((category) => {
    const units = rawCategoryUnits[category.id];
    const mixSharePercent = totalUnits > 0 ? roundPercent((units / totalUnits) * 100) : 0;
    const categoryTokens = totalUsedTokens > 0 ? Math.round((mixSharePercent / 100) * totalUsedTokens) : 0;
    const sharePercent =
      contextWindowTokens > 0 && categoryTokens > 0
        ? roundPercent((categoryTokens / contextWindowTokens) * 100)
        : mixSharePercent;
    return {
      ...category,
      units: categoryTokens,
      sharePercent,
    };
  });
  const reasoningSharePercent = categories.find((category) => category.id === "reasoning")?.sharePercent ?? 0;
  const normalizedTotalSkills =
    contextUsage.loadedSkills.totalCount ?? Math.max(totalSkillMetadataCount, loadedConcreteSkills.length);

  return {
    hasBudgetData: hasTokenUsage(tokenUsage),
    budgetUsedPercent: budgetPercentFromTokenUsage(tokenUsage),
    usedTokens: usedTokensFromTokenUsage(tokenUsage),
    contextWindowTokens: contextWindowTokensFromTokenUsage(tokenUsage),
    loadedSkills: Math.max(contextUsage.loadedSkills.loadedCount, loadedConcreteSkills.length),
    totalSkills: normalizedTotalSkills,
    totalConcreteLoads,
    reasoningSharePercent,
    categories,
    loadedConcreteSkills,
    turnTrend,
  };
}

function initializeCategoryUnits(): Record<ContextUsageCategoryId, number> {
  return {
    compact: 0,
    skillsMetadata: 0,
    concreteSkills: 0,
    toolsMetadata: 0,
    toolCalls: 0,
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
  const maxCategoryUnits = Math.max(
    ...visibleTurns.flatMap((turn) => CATEGORY_ORDER.map((category) => turn.units[category.id])),
    0,
  );

  return {
    turns: visibleTurns.map((turn) => ({
      turnId: turn.turnId,
      label: turn.label,
    })),
    rows: CATEGORY_ORDER.map((category) => ({
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

function estimateTextUnits(value: string | null | undefined) {
  if (!value) {
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

function budgetPercentFromTokenUsage(tokenUsage: ThreadTokenUsage | null | undefined) {
  const totalTokens = usedTokensFromTokenUsage(tokenUsage) ?? 0;
  const modelContextWindow = contextWindowTokensFromTokenUsage(tokenUsage) ?? 0;

  if (modelContextWindow <= 0 || totalTokens <= 0) {
    return 0;
  }

  return sanitizePercent((totalTokens / modelContextWindow) * 100);
}

function hasTokenUsage(tokenUsage: ThreadTokenUsage | null | undefined) {
  return (contextWindowTokensFromTokenUsage(tokenUsage) ?? 0) > 0
    && (usedTokensFromTokenUsage(tokenUsage) ?? 0) > 0;
}

function usedTokensFromTokenUsage(tokenUsage: ThreadTokenUsage | null | undefined) {
  const totalTokens = tokenUsage?.last.totalTokens ?? 0;
  return totalTokens > 0 ? totalTokens : null;
}

function contextWindowTokensFromTokenUsage(tokenUsage: ThreadTokenUsage | null | undefined) {
  const modelContextWindow = tokenUsage?.modelContextWindow ?? 0;
  return modelContextWindow > 0 ? modelContextWindow : null;
}
