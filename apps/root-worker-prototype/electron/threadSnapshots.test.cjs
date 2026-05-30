const test = require("node:test");
const assert = require("node:assert/strict");

const { mergeThreadSnapshots } = require("./threadSnapshots.cjs");

function makeUsage(totalTokens = 1200, budgetUsedPercent = 12) {
  const tokenUsage = {
    total: {
      totalTokens,
      inputTokens: 800,
      cachedInputTokens: 100,
      outputTokens: 400,
      reasoningOutputTokens: 50,
    },
    last: {
      totalTokens: 200,
      inputTokens: 120,
      cachedInputTokens: 20,
      outputTokens: 80,
      reasoningOutputTokens: 10,
    },
    modelContextWindow: 200000,
  };
  const contextUsage = {
    totalBytes: 1234,
    budgetUsedPercent,
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

  return {
    threadUsage: {
      tokenUsage,
      contextUsage,
    },
    tokenUsage,
    contextUsage,
  };
}

function makeThread(overrides = {}) {
  return {
    id: "thread-1",
    preview: "",
    turns: [],
    ...makeUsage(),
    ...overrides,
  };
}

test("mergeThreadSnapshots preserves restored usage when thread/read omits usage fields", () => {
  const restored = makeThread();
  const readThread = {
    ...makeThread({ preview: "fresh preview" }),
    threadUsage: undefined,
    tokenUsage: undefined,
    contextUsage: undefined,
  };

  const merged = mergeThreadSnapshots(restored, readThread);

  assert.equal(merged.preview, "fresh preview");
  assert.equal(merged.threadUsage?.tokenUsage?.total.totalTokens, 1200);
  assert.equal(merged.threadUsage?.contextUsage?.budgetUsedPercent, 12);
  assert.equal(merged.tokenUsage?.total.totalTokens, 1200);
  assert.equal(merged.contextUsage?.budgetUsedPercent, 12);
});

test("mergeThreadSnapshots preserves restored usage when a later snapshot sends null usage", () => {
  const restored = makeThread();
  const readThread = makeThread({
    tokenUsage: null,
    contextUsage: null,
    threadUsage: {
      tokenUsage: null,
      contextUsage: null,
    },
  });

  const merged = mergeThreadSnapshots(restored, readThread);

  assert.equal(merged.threadUsage?.tokenUsage?.total.totalTokens, 1200);
  assert.equal(merged.threadUsage?.contextUsage?.budgetUsedPercent, 12);
  assert.equal(merged.tokenUsage?.total.totalTokens, 1200);
  assert.equal(merged.contextUsage?.budgetUsedPercent, 12);
});

test("mergeThreadSnapshots prefers newer usage snapshots when thread/read has them", () => {
  const restored = makeThread();
  const readThread = makeThread({
    ...makeUsage(2400, 24),
  });

  const merged = mergeThreadSnapshots(restored, readThread);

  assert.equal(merged.threadUsage?.tokenUsage?.total.totalTokens, 2400);
  assert.equal(merged.threadUsage?.contextUsage?.budgetUsedPercent, 24);
  assert.equal(merged.tokenUsage?.total.totalTokens, 2400);
  assert.equal(merged.contextUsage?.budgetUsedPercent, 24);
});
