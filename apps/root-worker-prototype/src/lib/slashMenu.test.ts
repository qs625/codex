import test from "node:test";
import assert from "node:assert/strict";

import {
  buildComposerSlashSuggestions,
  getActiveComposerSlashQuery,
} from "./slashMenu";
import type { ThreadSkill } from "../types";

function makeSkill(overrides: Partial<ThreadSkill> = {}): ThreadSkill {
  return {
    name: "review",
    path: "/skills/review/SKILL.md",
    kind: "explicit",
    ...overrides,
  };
}

test("detects active composer slash query at the start of the draft", () => {
  assert.equal(getActiveComposerSlashQuery("/"), "");
  assert.equal(getActiveComposerSlashQuery("  /cle"), "cle");
  assert.equal(getActiveComposerSlashQuery("/clear now"), null);
  assert.equal(getActiveComposerSlashQuery("Use /clear"), null);
});

test("shows built-in commands and skills for an empty slash query", () => {
  const suggestions = buildComposerSlashSuggestions({
    availableSkills: [makeSkill()],
    draftSkills: [],
    query: "",
  });

  assert.deepEqual(
    suggestions.map((suggestion) =>
      suggestion.type === "command"
        ? `${suggestion.type}:${suggestion.commandId}`
        : `${suggestion.type}:${suggestion.skill.name}`,
    ),
    ["command:clear", "skill:review"],
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
      description: "Archive this root session and start a fresh root",
      aliases: ["reset", "new"],
    },
  ]);
});

test("filters commands and skills from the same query", () => {
  const suggestions = buildComposerSlashSuggestions({
    availableSkills: [
      makeSkill({ name: "review" }),
      makeSkill({ name: "openai-docs", path: "/skills/openai-docs/SKILL.md" }),
    ],
    draftSkills: [],
    query: "doc",
  });

  assert.deepEqual(
    suggestions.map((suggestion) =>
      suggestion.type === "command"
        ? `${suggestion.type}:${suggestion.commandId}`
        : `${suggestion.type}:${suggestion.skill.name}`,
    ),
    ["skill:openai-docs"],
  );
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
        : `${suggestion.type}:${suggestion.skill.name}`,
    ),
    ["command:clear", "skill:test"],
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
