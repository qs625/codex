import test from "node:test";
import assert from "node:assert/strict";

import { buildContextUsageAnalysis } from "./contextUsage";
import type { Thread } from "../types";

function makeThread(items: Thread["turns"][number]["items"], skills: Thread["skills"] = []): Thread {
  return {
    id: "thread-1",
    sessionId: "session-1",
    forkedFromId: null,
    preview: "",
    ephemeral: false,
    modelProvider: "openai",
    model: "gpt-5",
    reasoningEffort: null,
    createdAt: 1,
    updatedAt: 1,
    status: "running",
    path: null,
    cwd: "/tmp",
    cliVersion: "test",
    source: "cli",
    threadSource: null,
    agentNickname: null,
    agentRole: null,
    gitInfo: null,
    name: null,
    skills,
    turns: [
      {
        id: "turn-1",
        items,
        itemsView: "full",
        status: "completed",
        error: null,
        startedAt: 1,
        completedAt: 1,
        durationMs: 0,
      },
    ],
  };
}

test("builds compact context usage analysis with loaded skill ratios and timeline", () => {
  const thread = makeThread(
    [
      {
        type: "userMessage",
        id: "user-1",
        content: [
          {
            type: "text",
            text: "Investigate tool selection and wire the new analysis panel.",
          },
          {
            type: "skill",
            name: "openai-docs",
            path: "/skills/openai-docs/SKILL.md",
          },
          {
            type: "skill",
            name: "openai-docs",
            path: "/skills/openai-docs/SKILL.md",
          },
        ],
      },
      {
        type: "agentMessage",
        id: "agent-1",
        text: "I reviewed the panel and will refactor the right rail content next.",
        phase: null,
        memoryCitation: null,
      },
      {
        type: "reasoning",
        id: "reasoning-1",
        summary: ["Need a compact budget section and a horizontal category ratio."],
        content: [],
      },
      {
        type: "builtinToolCall",
        id: "tool-1",
        tool: "view_image",
        arguments: { path: "/tmp/mock.png" },
        status: "completed",
        output: { ok: true },
      },
    ],
    [
      {
        name: "openai-docs",
        path: "/skills/openai-docs/SKILL.md",
        kind: "explicit",
      },
      {
        name: "skill-creator",
        path: "/skills/skill-creator/SKILL.md",
        kind: "implicit",
      },
    ],
  );

  const analysis = buildContextUsageAnalysis(thread, 12);

  assert.equal(analysis.loadedSkills, 2);
  assert.equal(analysis.totalSkills, 12);
  assert.equal(analysis.totalConcreteLoads, 3);
  assert.equal(analysis.hasBudgetData, false);
  assert.equal(analysis.turnTrend.turns.length, 1);
  assert.equal(analysis.turnTrend.rows.length, 8);
  assert.ok(
    (analysis.turnTrend.rows.find((row) => row.id === "userMessages")?.cells[0]?.units ?? 0) > 0,
  );
  assert.equal(analysis.categories.length, 8);
  assert.equal(analysis.loadedConcreteSkills[0]?.name, "openai-docs");
  assert.equal(analysis.loadedConcreteSkills[0]?.loadCount, 2);
  assert.equal(analysis.loadedConcreteSkills[1]?.name, "skill-creator");
  assert.equal(analysis.loadedConcreteSkills[1]?.loadCount, 1);
  assert.equal(analysis.budgetUsedPercent, 0);
  assert.ok(
    analysis.categories.some(
      (category) => category.id === "toolCalls" && category.sharePercent === 0,
    ),
  );
});

test("uses last token usage for budget percent and context usage ratios for token distribution", () => {
  const thread = makeThread(
    [
      {
        type: "userMessage",
        id: "user-1",
        content: [
          {
            type: "text",
            text: "Load the openai-docs skill and inspect the current thread context usage.",
          },
        ],
      },
      {
        type: "agentMessage",
        id: "agent-1",
        text: "Skill loaded. I am checking the panel state now.",
        phase: null,
        memoryCitation: null,
      },
    ],
    [
      {
        name: "openai-docs",
        path: "/skills/openai-docs/SKILL.md",
        kind: "explicit",
      },
    ],
  );

  thread.contextUsage = {
    totalBytes: 128430,
    budgetUsedPercent: 64,
    categories: {
      compact: 0,
      skillsMetadata: 18210,
      concreteSkills: 22563,
      toolsMetadata: 10398,
      toolCalls: 19565,
      userMessages: 24145,
      llmMessages: 25484,
      reasoning: 7865,
    },
    loadedSkills: {
      loadedCount: 0,
      totalCount: 12,
      skills: [],
    },
  };
  thread.tokenUsage = {
    total: {
      totalTokens: 100000,
      inputTokens: 76000,
      cachedInputTokens: 12000,
      outputTokens: 24000,
      reasoningOutputTokens: 8000,
    },
    last: {
      totalTokens: 18000,
      inputTokens: 13000,
      cachedInputTokens: 2000,
      outputTokens: 5000,
      reasoningOutputTokens: 1500,
    },
    modelContextWindow: 200000,
  };

  const analysis = buildContextUsageAnalysis(thread, 12);

  assert.equal(analysis.hasBudgetData, true);
  assert.equal(analysis.budgetUsedPercent, 9);
  assert.equal(analysis.loadedSkills, 1);
  assert.equal(analysis.loadedConcreteSkills.length, 1);
  assert.equal(analysis.loadedConcreteSkills[0]?.name, "openai-docs");
  assert.equal(analysis.loadedConcreteSkills[0]?.loadCount, 1);
  assert.equal(analysis.turnTrend.turns[0]?.label, "1");
  assert.equal(analysis.turnTrend.rows.find((row) => row.id === "llmMessages")?.cells.length, 1);
  assert.equal(analysis.categories.find((row) => row.id === "llmMessages")?.sharePercent, 19.9);
  assert.equal(analysis.categories.find((row) => row.id === "llmMessages")?.units, 19900);
});

test("uses last token usage instead of cumulative token usage for budget percent", () => {
  const thread = makeThread([]);

  thread.contextUsage = {
    totalBytes: 128430,
    budgetUsedPercent: 91,
    categories: {
      compact: 0,
      skillsMetadata: 0,
      concreteSkills: 0,
      toolsMetadata: 0,
      toolCalls: 0,
      userMessages: 0,
      llmMessages: 0,
      reasoning: 0,
    },
    loadedSkills: {
      loadedCount: 0,
      totalCount: 0,
      skills: [],
    },
  };
  thread.tokenUsage = {
    total: {
      totalTokens: 25000,
      inputTokens: 16000,
      cachedInputTokens: 5000,
      outputTokens: 9000,
      reasoningOutputTokens: 2000,
    },
    last: {
      totalTokens: 4000,
      inputTokens: 2500,
      cachedInputTokens: 700,
      outputTokens: 1500,
      reasoningOutputTokens: 400,
    },
    modelContextWindow: 100000,
  };

  const analysis = buildContextUsageAnalysis(thread, 0);

  assert.equal(analysis.hasBudgetData, true);
  assert.equal(analysis.budgetUsedPercent, 4);
});

test("caps turn trend to the most recent turns", () => {
  const thread = makeThread([]);
  thread.turns = Array.from({ length: 20 }, (_, index) => ({
    id: `turn-${index + 1}`,
    items: [],
    itemsView: "full",
    status: "completed",
    error: null,
    startedAt: index + 1,
    completedAt: index + 1,
    durationMs: 0,
  }));

  const analysis = buildContextUsageAnalysis(thread, 0);

  assert.equal(analysis.turnTrend.turns.length, 16);
  assert.equal(analysis.turnTrend.turns[0]?.label, "5");
  assert.equal(analysis.turnTrend.turns.at(-1)?.label, "20");
});
