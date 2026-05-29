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
  assert.equal(analysis.turnTrend.length, 1);
  assert.equal(analysis.turnTrend[0]?.intensity, 1);
  assert.equal(analysis.categories.length, 8);
  assert.equal(analysis.loadedConcreteSkills[0]?.name, "openai-docs");
  assert.equal(analysis.loadedConcreteSkills[0]?.loadCount, 2);
  assert.equal(analysis.loadedConcreteSkills[1]?.name, "skill-creator");
  assert.equal(analysis.loadedConcreteSkills[1]?.loadCount, 1);
  assert.ok(analysis.budgetUsedPercent > 0);
  assert.ok(
    analysis.categories.some(
      (category) => category.id === "toolCalls" && category.sharePercent > 0,
    ),
  );
});
