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
    status: { type: "active", activeFlags: [] },
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

test("counts event command subscriptions and events in tool usage", () => {
  const analysis = buildContextUsageAnalysis(
    makeThread([
      {
        type: "eventCommandCall",
        id: "event-command-1",
        subscriptionId: "sub-1",
        command: "cargo test -p app-server",
        cwd: "/tmp/project",
        label: "app-server tests",
        status: "completed",
        output: { ok: true },
      },
      {
        type: "eventCommandEvent",
        id: "event-command-event-1",
        subscriptionId: "sub-1",
        kind: "output",
        label: "app-server tests",
        command: "cargo test -p app-server",
        cwd: "/tmp/project",
        line: "running 1 test",
        sequence: 1,
        exitCode: null,
        signal: null,
        message: null,
        truncated: false,
        createdAt: 2,
      },
    ]),
    0,
  );

  assert.ok(
    (analysis.turnTrend.rows.find((row) => row.id === "toolCalls")?.cells[0]?.units ?? 0) > 0,
  );
  assert.ok(
    (analysis.turnTrend.rows.find((row) => row.id === "toolsMetadata")?.cells[0]?.units ?? 0) > 0,
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
    toolBreakdown: {
      applyPatch: { input: 1200, output: 300 },
      fileOperations: { input: 600, output: 400 },
      commands: { input: 500, output: 1500 },
      interAgent: { input: 250, output: 250 },
      searchMedia: { input: 0, output: 0 },
      otherTools: { input: 0, output: 0 },
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
  assert.equal(analysis.usedTokens, 18000);
  assert.equal(analysis.contextWindowTokens, 200000);
  assert.equal(analysis.loadedSkills, 1);
  assert.equal(analysis.loadedConcreteSkills.length, 1);
  assert.equal(analysis.loadedConcreteSkills[0]?.name, "openai-docs");
  assert.equal(analysis.loadedConcreteSkills[0]?.loadCount, 1);
  assert.equal(analysis.turnTrend.turns[0]?.label, "1");
  assert.equal(analysis.turnTrend.rows.find((row) => row.id === "llmMessages")?.cells.length, 1);
  assert.equal(analysis.categories.find((row) => row.id === "llmMessages")?.sharePercent, 1.8);
  assert.equal(analysis.categories.find((row) => row.id === "llmMessages")?.units, 19900);
  assert.deepEqual(
    analysis.toolBreakdown.map((row) => row.id),
    ["applyPatch", "fileOperations", "commands", "interAgent"],
  );
  assert.equal(analysis.toolBreakdown.find((row) => row.id === "applyPatch")?.sharePercent, 30);
  assert.equal(analysis.toolBreakdown.find((row) => row.id === "commands")?.outputUnits, 1500);
});

test("omits tool breakdown when backend context usage has no breakdown data", () => {
  const thread = makeThread([
    {
      type: "builtinToolCall",
      id: "tool-1",
      tool: "apply_patch",
      arguments: { patch: "*** Begin Patch" },
      status: "completed",
      output: { ok: true },
    },
  ]);
  thread.contextUsage = {
    totalBytes: 10,
    budgetUsedPercent: null,
    categories: {
      compact: 0,
      skillsMetadata: 0,
      concreteSkills: 0,
      toolsMetadata: 0,
      toolCalls: 10,
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

  const analysis = buildContextUsageAnalysis(thread, 0);

  assert.deepEqual(analysis.toolBreakdown, []);
});

test("uses selected model context window override for budget display", () => {
  const thread = makeThread([]);
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

  const analysis = buildContextUsageAnalysis(
    thread,
    /*totalSkillMetadataCount*/ 0,
    100000,
  );

  assert.equal(analysis.hasBudgetData, true);
  assert.equal(analysis.budgetUsedPercent, 18);
  assert.equal(analysis.contextWindowTokens, 100000);
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
  assert.equal(analysis.usedTokens, 4000);
  assert.equal(analysis.contextWindowTokens, 100000);
});

test("ignores malformed non-string tool text in context usage estimation", () => {
  const thread = makeThread([
    {
      type: "collabAgentMessage",
      id: "msg-1",
      operation: "childCompletion",
      senderThreadId: "thread-2",
      senderPath: "/root/worker",
      recipientThreadId: "thread-1",
      recipientPath: "/root",
      otherRecipientPaths: [],
      content: { text: "done" } as never,
      triggerTurn: true,
    },
    {
      type: "imageGeneration",
      id: "image-1",
      status: "completed",
      revisedPrompt: { prompt: "draw" } as never,
      result: "saved",
      savedPath: null,
    },
  ]);

  const analysis = buildContextUsageAnalysis(thread, 0);

  assert.equal(analysis.turnTrend.turns.length, 1);
  assert.ok(
    (analysis.turnTrend.rows.find((row) => row.id === "toolCalls")?.cells[0]?.units ?? 0) > 0,
  );
});

test("turn trend only counts skill injected context as concrete skills", () => {
  const thread = makeThread(
    [
      {
        type: "injectedContext",
        id: "ctx-1",
        title: "openai-docs",
        preview: "Skill body preview",
        sections: [
          {
            label: "Skill: openai-docs",
            text: "Longer concrete skill instructions",
          },
          {
            label: "Developer instructions",
            text: "This should not inflate skill heatmap",
          },
        ],
      },
    ],
    [],
  );

  const analysis = buildContextUsageAnalysis(thread, 0);

  assert.ok((analysis.turnTrend.rows.find((row) => row.id === "concreteSkills")?.cells[0]?.units ?? 0) > 0);
  assert.equal(
    analysis.turnTrend.rows.find((row) => row.id === "skillsMetadata")?.cells[0]?.units ?? 0,
    0,
  );
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
