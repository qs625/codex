import test from "node:test";
import assert from "node:assert/strict";

import {
  buildComposerSlashSuggestions,
  getActiveComposerSlashQuery,
} from "./slashMenu";
import type { ThreadSkill, WorkflowSummary } from "../types";

function makeSkill(overrides: Partial<ThreadSkill> = {}): ThreadSkill {
  return {
    name: "review",
    path: "/skills/review/SKILL.md",
    kind: "explicit",
    ...overrides,
  };
}

function makeWorkflow(
  overrides: Partial<WorkflowSummary> = {},
): WorkflowSummary {
  return {
    id: "feature-dev",
    name: "Feature Development",
    description: "Develop a feature with review and verification",
    source: "project",
    path: "/repo/.codex/workflows/feature-dev",
    entry: "workflow.ts",
    version: null,
    whenToUse: ["new feature development"],
    inputs: {
      objective: {
        type: "string",
        description: "Development objective",
      },
      cwd: {
        type: "string",
        description: "Worktree path",
      },
    },
    ...overrides,
  };
}

test("detects active composer slash query at the start of the draft", () => {
  assert.equal(getActiveComposerSlashQuery("/"), "");
  assert.equal(getActiveComposerSlashQuery("  /cle"), "cle");
  assert.equal(getActiveComposerSlashQuery("/clear now"), null);
  assert.equal(getActiveComposerSlashQuery("Use /clear"), null);
});

test("detects goal subcommand slash query before complete commands", () => {
  assert.equal(getActiveComposerSlashQuery("/goal "), "goal ");
  assert.equal(getActiveComposerSlashQuery("/goal p"), "goal p");
  assert.equal(getActiveComposerSlashQuery("/goal pause"), null);
  assert.equal(getActiveComposerSlashQuery("/goal pause this migration"), null);
  assert.equal(getActiveComposerSlashQuery("/goal ship"), null);
});

test("shows built-in commands and skills for an empty slash query", () => {
  const suggestions = buildComposerSlashSuggestions({
    availableSkills: [makeSkill()],
    availableWorkflows: [makeWorkflow()],
    draftSkills: [],
    query: "",
  });

  assert.deepEqual(
    suggestions.map((suggestion) =>
      suggestion.type === "command"
        ? `${suggestion.type}:${suggestion.commandId}`
        : suggestion.type === "workflow"
          ? `${suggestion.type}:${suggestion.workflow.id}`
          : `${suggestion.type}:${suggestion.skill.name}`,
    ),
    [
      "command:clear",
      "command:goalCreate",
      "command:goalPause",
      "command:goalResume",
      "command:goalCancel",
      "workflow:feature-dev",
      "skill:review",
    ],
  );
});

test("keeps built-in commands visible when no skills are available", () => {
  const suggestions = buildComposerSlashSuggestions({
    availableSkills: [],
    draftSkills: [],
    query: "",
  });

  assert.deepEqual(suggestions, [
    {
      type: "command",
      commandId: "clear",
      token: "clear",
      label: "/clear",
      description: "Archive this project session and start a fresh project chat",
      aliases: ["reset", "new"],
    },
    {
      type: "command",
      commandId: "goalCreate",
      token: "goal objective",
      label: "/goal <objective>",
      description: "Create or update this thread goal",
      aliases: ["goal", "objective", "create goal", "set goal"],
      draftText: "/goal ",
    },
    {
      type: "command",
      commandId: "goalPause",
      token: "goal pause",
      label: "/goal pause",
      description: "Pause the current thread goal",
      aliases: ["pause goal", "goal"],
      draftText: "/goal pause",
    },
    {
      type: "command",
      commandId: "goalResume",
      token: "goal resume",
      label: "/goal resume",
      description: "Resume the current thread goal",
      aliases: ["resume goal", "goal"],
      draftText: "/goal resume",
    },
    {
      type: "command",
      commandId: "goalCancel",
      token: "goal cancel",
      label: "/goal cancel",
      description: "Cancel the current thread goal",
      aliases: ["cancel-goal", "goal clear", "clear goal", "goal", "cancel"],
      draftText: "/goal cancel",
    },
  ]);
});

test("filters commands and skills from the same query", () => {
  const suggestions = buildComposerSlashSuggestions({
    availableSkills: [
      makeSkill({ name: "review" }),
      makeSkill({ name: "openai-docs", path: "/skills/openai-docs/SKILL.md" }),
    ],
    availableWorkflows: [
      makeWorkflow(),
      makeWorkflow({
        id: "release-triage",
        name: "Release Triage",
        path: "/repo/.codex/workflows/release-triage",
        whenToUse: ["release stabilization"],
      }),
    ],
    draftSkills: [],
    query: "release",
  });

  assert.deepEqual(
    suggestions.map((suggestion) =>
      suggestion.type === "command"
        ? `${suggestion.type}:${suggestion.commandId}`
        : suggestion.type === "workflow"
          ? `${suggestion.type}:${suggestion.workflow.id}`
          : `${suggestion.type}:${suggestion.skill.name}`,
    ),
    ["workflow:release-triage"],
  );
});

test("filters skills from the same query when no workflow matches", () => {
  const suggestions = buildComposerSlashSuggestions({
    availableSkills: [
      makeSkill({ name: "review" }),
      makeSkill({ name: "openai-docs", path: "/skills/openai-docs/SKILL.md" }),
    ],
    availableWorkflows: [makeWorkflow()],
    draftSkills: [],
    query: "doc",
  });

  assert.deepEqual(
    suggestions.map((suggestion) =>
      suggestion.type === "command"
        ? `${suggestion.type}:${suggestion.commandId}`
        : suggestion.type === "workflow"
          ? `${suggestion.type}:${suggestion.workflow.id}`
          : `${suggestion.type}:${suggestion.skill.name}`,
    ),
    ["skill:openai-docs"],
  );
});

test("builds workflow suggestions from discovered workflows", () => {
  const [suggestion] = buildComposerSlashSuggestions({
    availableSkills: [],
    availableWorkflows: [makeWorkflow()],
    draftSkills: [],
    query: "feature",
  });

  assert.deepEqual(suggestion, {
    type: "workflow",
    workflow: makeWorkflow(),
    draftText: "Use the feature-dev workflow with objective: , cwd: ",
  });
});

test("does not show workflow suggestions when no workflows are discovered", () => {
  const suggestions = buildComposerSlashSuggestions({
    availableSkills: [],
    availableWorkflows: [],
    commandsEnabled: false,
    draftSkills: [],
    query: "workflow",
  });

  assert.deepEqual(suggestions, []);
});

test("does not suggest skills already attached to the draft", () => {
  const suggestions = buildComposerSlashSuggestions({
    availableSkills: [
      makeSkill({ name: "review" }),
      makeSkill({ name: "test", path: "/skills/test/SKILL.md" }),
    ],
    draftSkills: [{ name: "review", path: "/skills/review/SKILL.md" }],
    query: "",
  });

  assert.deepEqual(
    suggestions.map((suggestion) =>
      suggestion.type === "command"
        ? `${suggestion.type}:${suggestion.commandId}`
        : suggestion.type === "workflow"
          ? `${suggestion.type}:${suggestion.workflow.id}`
          : `${suggestion.type}:${suggestion.skill.name}`,
    ),
    [
      "command:clear",
      "command:goalCreate",
      "command:goalPause",
      "command:goalResume",
      "command:goalCancel",
      "skill:test",
    ],
  );
});

test("can suppress built-in commands when draft attachments would block execution", () => {
  const suggestions = buildComposerSlashSuggestions({
    availableSkills: [
      makeSkill({ name: "test", path: "/skills/test/SKILL.md" }),
    ],
    commandsEnabled: false,
    draftSkills: [],
    query: "",
  });

  assert.deepEqual(
    suggestions.map((suggestion) =>
      suggestion.type === "command"
        ? `${suggestion.type}:${suggestion.commandId}`
        : suggestion.type === "workflow"
          ? `${suggestion.type}:${suggestion.workflow.id}`
          : `${suggestion.type}:${suggestion.skill.name}`,
    ),
    ["skill:test"],
  );
});

test("clear is modeled as a command rather than a skill payload", () => {
  const [suggestion] = buildComposerSlashSuggestions({
    availableSkills: [makeSkill({ name: "clear", path: "/skills/clear.md" })],
    draftSkills: [],
    query: "clear",
  });

  if (!suggestion || suggestion.type !== "command") {
    assert.fail("expected the first clear suggestion to be a command");
  } else {
    assert.equal(suggestion.commandId, "clear");
  }
});

test("goal cancel is modeled as a command and can be found by alias", () => {
  const [suggestion] = buildComposerSlashSuggestions({
    availableSkills: [],
    draftSkills: [],
    query: "cancel-goal",
  });

  if (!suggestion || suggestion.type !== "command") {
    assert.fail("expected goal cancel suggestion to be a command");
  } else {
    assert.equal(suggestion.commandId, "goalCancel");
  }
});

test("goal family commands are displayed as subcommands", () => {
  const suggestions = buildComposerSlashSuggestions({
    availableSkills: [],
    draftSkills: [],
    query: "goal",
  });

  assert.deepEqual(
    suggestions.map((suggestion) =>
      suggestion.type === "command" ? suggestion.label : suggestion.skill.name,
    ),
    ["/goal <objective>", "/goal pause", "/goal resume", "/goal cancel"],
  );
});

test("filters goal subcommands from a secondary query", () => {
  const suggestions = buildComposerSlashSuggestions({
    availableSkills: [],
    draftSkills: [],
    query: getActiveComposerSlashQuery("/goal p"),
  });

  assert.deepEqual(
    suggestions.map((suggestion) =>
      suggestion.type === "command" ? suggestion.label : suggestion.skill.name,
    ),
    ["/goal pause"],
  );
});
