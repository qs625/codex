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

function makeCollabMessageTurn(overrides = {}) {
  return {
    id: "turn-1",
    items: [
      {
        type: "collabAgentMessage",
        id: "item-1",
        operation: "send_message",
        senderThreadId: "thread-2",
        senderPath: "/root/worker",
        recipientThreadId: "thread-1",
        recipientPath: "/root",
        otherRecipientPaths: [],
        content: "same backend message",
        triggerTurn: true,
      },
    ],
    itemsView: "full",
    status: "running",
    error: null,
    startedAt: 10,
    completedAt: null,
    durationMs: null,
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

test("mergeThreadSnapshots keeps resume-only in-flight items missing from a stale read", () => {
  const restoredTurn = makeCollabMessageTurn();
  const restored = makeThread({ turns: [restoredTurn] });
  const readThread = makeThread({ turns: [] });

  const merged = mergeThreadSnapshots(restored, readThread);

  assert.deepEqual(merged.turns, [restoredTurn]);
});

test("mergeThreadSnapshots drops duplicate in-flight items already present in the read snapshot", () => {
  const restoredTurn = makeCollabMessageTurn({
    id: "restored-turn",
    items: [
      {
        ...makeCollabMessageTurn().items[0],
        id: "restored-item",
      },
    ],
  });
  const readTurn = makeCollabMessageTurn({
    id: "read-turn",
    items: [
      {
        ...restoredTurn.items[0],
        id: "read-item",
      },
    ],
    status: "completed",
    completedAt: 12,
    durationMs: 2000,
  });

  const merged = mergeThreadSnapshots(
    makeThread({ turns: [restoredTurn] }),
    makeThread({ turns: [readTurn] }),
  );

  assert.deepEqual(merged.turns, [readTurn]);
});

test("mergeThreadSnapshots drops duplicate completed live agent turns already present in the read snapshot", () => {
  const restoredTurn = {
    id: "restored-turn",
    items: [
      {
        type: "agentMessage",
        id: "restored-item",
        text: "same response",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "notLoaded",
    status: "completed",
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  };
  const readTurn = {
    ...restoredTurn,
    id: "read-turn",
    items: [
      {
        ...restoredTurn.items[0],
        id: "read-item",
      },
    ],
    itemsView: "full",
  };

  const merged = mergeThreadSnapshots(
    makeThread({ turns: [restoredTurn] }),
    makeThread({ turns: [readTurn] }),
  );

  assert.deepEqual(merged.turns, [readTurn]);
});

test("mergeThreadSnapshots preserves completed full agent turns with matching content", () => {
  const restoredTurn = {
    id: "restored-turn",
    items: [
      {
        type: "agentMessage",
        id: "restored-item",
        text: "same response",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "full",
    status: "completed",
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  };
  const readTurn = {
    ...restoredTurn,
    id: "read-turn",
    items: [
      {
        ...restoredTurn.items[0],
        id: "read-item",
      },
    ],
  };

  const merged = mergeThreadSnapshots(
    makeThread({ turns: [restoredTurn] }),
    makeThread({ turns: [readTurn] }),
  );

  assert.deepEqual(merged.turns, [readTurn, restoredTurn]);
});

test("mergeThreadSnapshots only matches one existing item per semantic read item", () => {
  const restoredTurn = makeCollabMessageTurn({
    id: "restored-turn",
    items: [
      {
        ...makeCollabMessageTurn().items[0],
        id: "restored-item-1",
      },
      {
        ...makeCollabMessageTurn().items[0],
        id: "restored-item-2",
      },
    ],
  });
  const readTurn = makeCollabMessageTurn({
    id: "read-turn",
    items: [
      {
        ...restoredTurn.items[0],
        id: "read-item",
      },
    ],
    status: "completed",
    completedAt: 12,
    durationMs: 2000,
  });

  const merged = mergeThreadSnapshots(
    makeThread({ turns: [restoredTurn] }),
    makeThread({ turns: [readTurn] }),
  );

  assert.deepEqual(merged.turns, [
    readTurn,
    {
      ...restoredTurn,
      items: [restoredTurn.items[1]],
    },
  ]);
});

test("mergeThreadSnapshots drops duplicate placeholder turns with no timing metadata", () => {
  const restoredTurn = makeCollabMessageTurn({
    id: "restored-turn",
    startedAt: null,
    completedAt: null,
    durationMs: null,
    items: [
      {
        ...makeCollabMessageTurn().items[0],
        id: "restored-item",
      },
    ],
  });
  const readTurn = makeCollabMessageTurn({
    id: "read-turn",
    items: [
      {
        ...restoredTurn.items[0],
        id: "read-item",
      },
    ],
    status: "completed",
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  });

  const merged = mergeThreadSnapshots(
    makeThread({ turns: [restoredTurn] }),
    makeThread({ turns: [readTurn] }),
  );

  assert.deepEqual(merged.turns, [readTurn]);
});

test("mergeThreadSnapshots preserves distinct in-flight items with matching content", () => {
  const readTurn = makeCollabMessageTurn({
    id: "read-turn",
    items: [
      {
        ...makeCollabMessageTurn().items[0],
        id: "read-item",
      },
    ],
    status: "completed",
    completedAt: 12,
    durationMs: 2000,
  });
  const liveTurn = makeCollabMessageTurn({
    id: "live-turn",
    items: [
      {
        ...readTurn.items[0],
        id: "live-item",
      },
    ],
    startedAt: 20,
  });

  const merged = mergeThreadSnapshots(
    makeThread({ turns: [liveTurn] }),
    makeThread({ turns: [readTurn] }),
  );

  assert.deepEqual(merged.turns, [readTurn, liveTurn]);
});

test("mergeThreadSnapshots preserves duplicate context compaction markers", () => {
  const restoredTurn = makeCollabMessageTurn({
    id: "restored-turn",
    items: [
      {
        type: "contextCompaction",
        id: "restored-item",
      },
    ],
  });
  const readTurn = makeCollabMessageTurn({
    id: "read-turn",
    items: [
      {
        type: "contextCompaction",
        id: "read-item",
      },
    ],
    status: "completed",
    completedAt: 12,
    durationMs: 2000,
  });

  const merged = mergeThreadSnapshots(
    makeThread({ turns: [restoredTurn] }),
    makeThread({ turns: [readTurn] }),
  );

  assert.deepEqual(merged.turns, [readTurn, restoredTurn]);
});

test("mergeThreadSnapshots preserves duplicate dynamic tool calls", () => {
  const item = {
    type: "dynamicToolCall",
    id: "restored-item",
    namespace: "functions",
    tool: "read",
    arguments: { path: "/tmp/file" },
    status: "completed",
    contentItems: [{ text: "same output" }],
    success: true,
    durationMs: 10,
  };
  const restoredTurn = makeCollabMessageTurn({
    id: "restored-turn",
    items: [item],
  });
  const readTurn = makeCollabMessageTurn({
    id: "read-turn",
    items: [
      {
        ...item,
        id: "read-item",
      },
    ],
    status: "completed",
    completedAt: 12,
    durationMs: 2000,
  });

  const merged = mergeThreadSnapshots(
    makeThread({ turns: [restoredTurn] }),
    makeThread({ turns: [readTurn] }),
  );

  assert.deepEqual(merged.turns, [readTurn, restoredTurn]);
});
