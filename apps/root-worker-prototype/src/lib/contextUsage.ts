import type { Thread, ThreadContextUsage, ThreadItem, ThreadSkill } from "../types";

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

export type ContextUsageAnalysis = {
  budgetUsedPercent: number;
  loadedSkills: number;
  totalSkills: number;
  totalConcreteLoads: number;
  reasoningSharePercent: number;
  categories: ContextUsageCategorySummary[];
  loadedConcreteSkills: LoadedSkillSummary[];
  turnTrend: Array<{
    turnId: string;
    label: string;
    intensity: number;
  }>;
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
    label: "Skills Metadata",
    shortLabel: "Meta",
    description: "Skill registry and invocation metadata",
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
    label: "Tool Calls",
    shortLabel: "Calls",
    description: "Arguments, outputs, and execution traces",
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

const ESTIMATED_BUDGET_UNITS = 18000;

export function getContextUsageCategoryColor(categoryId: ContextUsageCategoryId) {
  return CATEGORY_COLORS[categoryId];
}

export function buildContextUsageAnalysis(
  thread: Thread | null,
  totalSkillMetadataCount: number,
): ContextUsageAnalysis {
  if (thread?.contextUsage) {
    return buildContextUsageAnalysisFromBackend(
      thread.contextUsage,
      thread,
      totalSkillMetadataCount,
    );
  }

  if (!thread) {
    return {
      budgetUsedPercent: 0,
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
      turnTrend: [],
    };
  }

  const categoryUnits = initializeCategoryUnits();
  const turnUnits: Array<{ turnId: string; totalUnits: number }> = [];
  const skillLoads = new Map<string, LoadedSkillSummary>();

  for (const skill of thread.skills) {
    skillLoads.set(skill.path, {
      name: skill.name,
      path: skill.path,
      kind: skill.kind,
      loadCount: 0,
    });
  }

  for (const turn of thread.turns) {
    const perTurnUnits = initializeCategoryUnits();

    for (const item of turn.items) {
      accumulateItemUnits(item, perTurnUnits, skillLoads);
    }

    for (const category of CATEGORY_ORDER) {
      categoryUnits[category.id] += perTurnUnits[category.id];
    }

    turnUnits.push({
      turnId: turn.id,
      totalUnits: sumCategoryUnits(perTurnUnits),
    });
  }

  const totalUnits = sumCategoryUnits(categoryUnits);
  const categories = CATEGORY_ORDER.map((category) => {
    const units = categoryUnits[category.id];
    return {
      ...category,
      units,
      sharePercent: totalUnits > 0 ? roundPercent((units / totalUnits) * 100) : 0,
    };
  });
  const loadedConcreteSkills = [...skillLoads.values()]
    .map((skill) => ({
      ...skill,
      loadCount: Math.max(skill.loadCount, 1),
    }))
    .sort((left, right) => right.loadCount - left.loadCount || left.name.localeCompare(right.name));
  const totalConcreteLoads = loadedConcreteSkills.reduce((sum, skill) => sum + skill.loadCount, 0);
  const reasoningSharePercent = categories.find((category) => category.id === "reasoning")?.sharePercent ?? 0;
  const normalizedTotalSkills = Math.max(totalSkillMetadataCount, loadedConcreteSkills.length);
  const maxTurnUnits = Math.max(...turnUnits.map((turn) => turn.totalUnits), 0);

  return {
    budgetUsedPercent:
      totalUnits > 0 ? Math.min(100, Math.max(1, roundPercent((totalUnits / ESTIMATED_BUDGET_UNITS) * 100))) : 0,
    loadedSkills: loadedConcreteSkills.length,
    totalSkills: normalizedTotalSkills,
    totalConcreteLoads,
    reasoningSharePercent,
    categories,
    loadedConcreteSkills,
    turnTrend: turnUnits.map((turn, index) => ({
      turnId: turn.turnId,
      label: String(index + 1),
      intensity: maxTurnUnits > 0 ? turn.totalUnits / maxTurnUnits : 0,
    })),
  };
}

function buildContextUsageAnalysisFromBackend(
  contextUsage: ThreadContextUsage,
  thread: Thread,
  totalSkillMetadataCount: number,
): ContextUsageAnalysis {
  const categoryUnits = initializeCategoryUnits();
  categoryUnits.compact = contextUsage.categories.compact;
  categoryUnits.skillsMetadata = contextUsage.categories.skillsMetadata;
  categoryUnits.concreteSkills = contextUsage.categories.concreteSkills;
  categoryUnits.toolsMetadata = contextUsage.categories.toolsMetadata;
  categoryUnits.toolCalls = contextUsage.categories.toolCalls;
  categoryUnits.userMessages = contextUsage.categories.userMessages;
  categoryUnits.llmMessages = contextUsage.categories.llmMessages;
  categoryUnits.reasoning = contextUsage.categories.reasoning;

  const totalUnits = sumCategoryUnits(categoryUnits);
  const categories = CATEGORY_ORDER.map((category) => {
    const units = categoryUnits[category.id];
    return {
      ...category,
      units,
      sharePercent: totalUnits > 0 ? roundPercent((units / totalUnits) * 100) : 0,
    };
  });
  const loadedConcreteSkills = [...(contextUsage.loadedSkills.skills ?? [])]
    .map((skill) => ({
      name: skill.name,
      path: skill.path,
      kind: skill.kind,
      loadCount: skill.loadCount,
    }))
    .sort((left, right) => right.loadCount - left.loadCount || left.name.localeCompare(right.name));
  const totalConcreteLoads = loadedConcreteSkills.reduce((sum, skill) => sum + skill.loadCount, 0);
  const reasoningSharePercent = categories.find((category) => category.id === "reasoning")?.sharePercent ?? 0;
  const normalizedTotalSkills =
    contextUsage.loadedSkills.totalCount ?? Math.max(totalSkillMetadataCount, loadedConcreteSkills.length);

  return {
    budgetUsedPercent: contextUsage.budgetUsedPercent ?? 0,
    loadedSkills: contextUsage.loadedSkills.loadedCount,
    totalSkills: normalizedTotalSkills,
    totalConcreteLoads,
    reasoningSharePercent,
    categories,
    loadedConcreteSkills,
    turnTrend: buildTurnTrend(thread),
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

function buildTurnTrend(thread: Thread) {
  const turnUnits = thread.turns.map((turn) => ({
    turnId: turn.id,
    totalUnits: turn.items.length,
  }));
  const maxTurnUnits = Math.max(...turnUnits.map((turn) => turn.totalUnits), 0);

  return turnUnits.map((turn, index) => ({
    turnId: turn.turnId,
    label: String(index + 1),
    intensity: maxTurnUnits > 0 ? turn.totalUnits / maxTurnUnits : 0,
  }));
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
    units.concreteSkills += estimateTextUnits(item.preview) + 80;
    for (const section of item.sections) {
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
