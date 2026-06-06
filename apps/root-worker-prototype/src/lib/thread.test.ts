import test from "node:test";
import assert from "node:assert/strict";

import {
  appendAgentDelta,
  applyPendingThreadUpdates,
  buildCurrentThreadTodoItems,
  formatUpdatedLabel,
  getPresenceLabel,
  getParentThreadId,
  getThreadPath,
  isThreadThinking,
  mergeThreadSnapshot,
  queuePendingThreadUpdate,
  threadStatusClass,
  treeThreadStatusClass,
  treeThreadStatusLabel,
  updateThreadItem,
  updateThreadTurn,
  upsertThread,
} from "./thread";
import type { Thread, ThreadItem, TreeNode, Turn } from "../types";

function makeThread(): Thread {
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
    status: { type: "idle" },
    path: null,
    cwd: "/tmp",
    cliVersion: "test",
    source: "cli",
    threadSource: null,
    agentNickname: null,
    agentRole: null,
    gitInfo: null,
    name: null,
    skills: [],
    turns: [],
    threadUsage: {
      tokenUsage: {
        total: {
          totalTokens: 1200,
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
      },
      contextUsage: {
        totalBytes: 1234,
        budgetUsedPercent: 12,
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
      },
    },
    tokenUsage: {
      total: {
        totalTokens: 1200,
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
    },
    contextUsage: {
      totalBytes: 1234,
      budgetUsedPercent: 12,
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
    },
  };
}

function makeTreeNode(
  thread: Thread | null,
  children: TreeNode[] = [],
): TreeNode {
  return {
    key: thread?.id ?? "placeholder",
    label: thread?.name ?? thread?.id ?? "placeholder",
    path: thread?.path ?? "/root",
    thread,
    threadId: thread?.id ?? "placeholder",
    isPlaceholder: thread === null,
    children,
  };
}

function eventDrivenToolMarker(
  trigger = {
    tool: "process_exit_subscribe",
    title: "Process exited",
    text: "Session 42 exited with code 0",
  },
) {
  return `<event_driven_tool>${JSON.stringify(trigger)}</event_driven_tool>`;
}

function makeRawProcessExitItem(
  overrides: Partial<Extract<ThreadItem, { type: "agentMessage" }>> = {},
): ThreadItem {
  return {
    type: "agentMessage",
    id: "raw-item",
    text: eventDrivenToolMarker(),
    phase: null,
    memoryCitation: null,
    ...overrides,
  };
}

test("mergeThreadSnapshot preserves usage fields when thread/read omits them", () => {
  const existing = makeThread();
  const next = {
    ...makeThread(),
    preview: "fresh preview",
    turns: [
      {
        id: "turn-1",
        items: [],
        itemsView: "full" as const,
        status: "completed" as const,
        error: null,
        startedAt: 1,
        completedAt: 2,
        durationMs: 1,
      },
    ],
  };
  delete next.tokenUsage;
  delete next.contextUsage;
  delete next.threadUsage;

  const merged = mergeThreadSnapshot(existing, next);

  assert.equal(merged.preview, "fresh preview");
  assert.equal(merged.turns.length, 1);
  assert.equal(merged.threadUsage?.tokenUsage?.total.totalTokens, 1200);
  assert.equal(merged.threadUsage?.contextUsage?.budgetUsedPercent, 12);
  assert.equal(merged.tokenUsage?.total.totalTokens, 1200);
  assert.equal(merged.contextUsage?.budgetUsedPercent, 12);
});

test("mergeThreadSnapshot normalizes raw process exit marker messages in history", () => {
  const turn: Turn = {
    id: "turn-1",
    items: [makeRawProcessExitItem()],
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 1,
    completedAt: 2,
    durationMs: 1,
  };

  const merged = mergeThreadSnapshot(null, {
    ...makeThread(),
    turns: [turn],
  });

  assert.deepEqual(merged.turns[0]?.items, [
    {
      type: "eventDrivenTool",
      id: "raw-item",
      tool: "process_exit_subscribe",
      title: "Process exited",
      text: "Session 42 exited with code 0",
    },
  ]);
});

test("mergeThreadSnapshot drops restored raw process exit when read has structured event item", () => {
  const existing: Thread = {
    ...makeThread(),
    turns: [
      {
        id: "raw-turn",
        items: [makeRawProcessExitItem()],
        itemsView: "notLoaded" as const,
        status: "completed" as const,
        error: null,
        startedAt: 1,
        completedAt: 2,
        durationMs: 1,
      },
    ],
  };
  const readTurn: Turn = {
    id: "read-turn",
    items: [
      {
        type: "eventDrivenTool",
        id: "event-item",
        tool: "process_exit_subscribe",
        title: "Process exited",
        text: "Session 42 exited with code 0",
      },
    ],
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 1,
    completedAt: 2,
    durationMs: 1,
  };

  const merged = mergeThreadSnapshot(existing, {
    ...makeThread(),
    turns: [readTurn],
  });

  assert.deepEqual(merged.turns, [readTurn]);
});

test("updateThreadItem creates a running turn when item notifications arrive first", () => {
  const thread = makeThread();

  const updated = updateThreadItem(
    thread,
    "turn-1",
    {
      type: "eventDrivenToolCall",
      id: "item-1",
      tool: "process_exit_subscribe",
      arguments: { session_id: 42 },
      status: "inProgress",
      output: null,
    },
    { startedAtMs: 2_000 },
  );

  assert.deepEqual(updated.turns, [
    {
      id: "turn-1",
      items: [
        {
          type: "eventDrivenToolCall",
          id: "item-1",
          tool: "process_exit_subscribe",
          arguments: { session_id: 42 },
          status: "inProgress",
          output: null,
          startedAtMs: 2_000,
        },
      ],
      itemsView: "full",
      status: "running",
      error: null,
      startedAt: 2,
      completedAt: null,
      durationMs: null,
    },
  ]);
});

test("updateThreadItem preserves started time when completion updates the same item", () => {
  const thread = updateThreadItem(
    makeThread(),
    "turn-1",
    {
      type: "eventDrivenToolCall",
      id: "item-1",
      tool: "process_exit_subscribe",
      arguments: { session_id: 42 },
      status: "inProgress",
      output: null,
    },
    { startedAtMs: 2_000 },
  );

  const updated = updateThreadItem(
    thread,
    "turn-1",
    {
      type: "eventDrivenToolCall",
      id: "item-1",
      tool: "process_exit_subscribe",
      arguments: { session_id: 42 },
      status: "completed",
      output: { subscription_id: "sub-1" },
    },
    { completedAtMs: 3_000 },
  );

  assert.deepEqual(updated.turns[0]?.items[0], {
    type: "eventDrivenToolCall",
    id: "item-1",
    tool: "process_exit_subscribe",
    arguments: { session_id: 42 },
    status: "completed",
    output: { subscription_id: "sub-1" },
    startedAtMs: 2_000,
    completedAtMs: 3_000,
  });
});

test("updateThreadTurn preserves item timestamps when a completed turn snapshot arrives", () => {
  const thread = updateThreadItem(
    makeThread(),
    "turn-1",
    {
      type: "eventDrivenToolCall",
      id: "item-1",
      tool: "process_exit_subscribe",
      arguments: { session_id: 42 },
      status: "completed",
      output: { subscription_id: "sub-1" },
    },
    { startedAtMs: 2_000, completedAtMs: 3_000 },
  );

  const updated = updateThreadTurn(thread, {
    id: "turn-1",
    items: [
      {
        type: "eventDrivenToolCall",
        id: "item-1",
        tool: "process_exit_subscribe",
        arguments: { session_id: 42 },
        status: "completed",
        output: { subscription_id: "sub-1" },
      },
    ],
    itemsView: "full",
    status: "completed",
    error: null,
    startedAt: 2,
    completedAt: 4,
    durationMs: 2_000,
  });

  assert.deepEqual(updated.turns[0]?.items[0], {
    type: "eventDrivenToolCall",
    id: "item-1",
    tool: "process_exit_subscribe",
    arguments: { session_id: 42 },
    status: "completed",
    output: { subscription_id: "sub-1" },
    startedAtMs: 2_000,
    completedAtMs: 3_000,
  });
});

test("updateThreadTurn normalizes raw process exit marker messages in new turns", () => {
  const updated = updateThreadTurn(makeThread(), {
    id: "turn-1",
    items: [makeRawProcessExitItem()],
    itemsView: "full",
    status: "completed",
    error: null,
    startedAt: 1,
    completedAt: 2,
    durationMs: 1,
  });

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "eventDrivenTool",
      id: "raw-item",
      tool: "process_exit_subscribe",
      title: "Process exited",
      text: "Session 42 exited with code 0",
    },
  ]);
});

test("mergeThreadSnapshot hydrates restored usage fields from thread/read", () => {
  const existing = {
    ...makeThread(),
    threadUsage: undefined,
    tokenUsage: undefined,
    contextUsage: undefined,
  };
  const merged = mergeThreadSnapshot(existing, makeThread());

  assert.equal(merged.threadUsage?.tokenUsage?.total.totalTokens, 1200);
  assert.equal(merged.threadUsage?.contextUsage?.budgetUsedPercent, 12);
  assert.equal(merged.tokenUsage?.total.totalTokens, 1200);
  assert.equal(merged.contextUsage?.budgetUsedPercent, 12);
});

test("mergeThreadSnapshot preserves restored usage when a later snapshot sends null fields", () => {
  const existing = makeThread();
  const next = {
    ...makeThread(),
    tokenUsage: null,
    contextUsage: null,
    threadUsage: {
      tokenUsage: null,
      contextUsage: null,
    },
  };

  const merged = mergeThreadSnapshot(existing, next);

  assert.equal(merged.threadUsage?.tokenUsage?.total.totalTokens, 1200);
  assert.equal(merged.threadUsage?.contextUsage?.budgetUsedPercent, 12);
  assert.equal(merged.tokenUsage?.total.totalTokens, 1200);
  assert.equal(merged.contextUsage?.budgetUsedPercent, 12);
});

test("mergeThreadSnapshot normalizes duplicate items when existing is null", () => {
  const thread = {
    ...makeThread(),
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "agentMessage" as const,
            id: "item-1",
            text: "same response",
            phase: null,
            memoryCitation: null,
          },
          {
            type: "agentMessage" as const,
            id: "item-2",
            text: "same response",
            phase: null,
            memoryCitation: null,
          },
        ],
        itemsView: "full" as const,
        status: "completed" as const,
        error: null,
        startedAt: 10,
        completedAt: 12,
        durationMs: 2000,
      },
    ],
  };

  const merged = mergeThreadSnapshot(null, thread);

  assert.deepEqual(merged.turns, [
    {
      ...thread.turns[0],
      items: [thread.turns[0].items[1]],
    },
  ]);
});

test("mergeThreadSnapshot preserves prefix-compatible agent messages in the same snapshot", () => {
  const thread = {
    ...makeThread(),
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "agentMessage" as const,
            id: "item-1",
            text: "same",
            phase: null,
            memoryCitation: null,
          },
          {
            type: "agentMessage" as const,
            id: "item-2",
            text: "same response",
            phase: null,
            memoryCitation: null,
          },
        ],
        itemsView: "full" as const,
        status: "completed" as const,
        error: null,
        startedAt: 10,
        completedAt: 12,
        durationMs: 2000,
      },
    ],
  };

  const merged = mergeThreadSnapshot(null, thread);

  assert.deepEqual(merged.turns, thread.turns);
});

test("upsertThread normalizes the first inserted snapshot", () => {
  const thread = {
    ...makeThread(),
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "agentMessage" as const,
            id: "item-1",
            text: "same response",
            phase: null,
            memoryCitation: null,
          },
          {
            type: "agentMessage" as const,
            id: "item-2",
            text: "same response",
            phase: null,
            memoryCitation: null,
          },
        ],
        itemsView: "full" as const,
        status: "completed" as const,
        error: null,
        startedAt: 10,
        completedAt: 12,
        durationMs: 2000,
      },
    ],
  };

  const threads = upsertThread([], thread);

  assert.deepEqual(threads, [
    {
      ...thread,
      turns: [
        {
          ...thread.turns[0],
          items: [thread.turns[0].items[1]],
        },
      ],
    },
  ]);
});

test("mergeThreadSnapshot normalizes duplicate live-derived turns within next snapshot", () => {
  const liveTurn = {
    id: "live-turn",
    items: [
      {
        type: "agentMessage" as const,
        id: "live-item",
        text: "same response",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "notLoaded" as const,
    status: "completed" as const,
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  };
  const readTurn = {
    ...liveTurn,
    id: "read-turn",
    items: [
      {
        ...liveTurn.items[0],
        id: "read-item",
      },
    ],
    itemsView: "full" as const,
  };

  const merged = mergeThreadSnapshot(null, {
    ...makeThread(),
    turns: [liveTurn, readTurn],
  });

  assert.deepEqual(merged.turns, [readTurn]);
});

test("mergeThreadSnapshot preserves completed full agent turns with matching content in one snapshot", () => {
  const firstTurn = {
    id: "first-turn",
    items: [
      {
        type: "agentMessage" as const,
        id: "first-item",
        text: "same response",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  };
  const secondTurn = {
    ...firstTurn,
    id: "second-turn",
    items: [
      {
        ...firstTurn.items[0],
        id: "second-item",
      },
    ],
  };

  const merged = mergeThreadSnapshot(null, {
    ...makeThread(),
    turns: [firstTurn, secondTurn],
  });

  assert.deepEqual(merged.turns, [firstTurn, secondTurn]);
});

test("mergeThreadSnapshot preserves duplicate dynamic tool calls in one snapshot", () => {
  const item = {
    type: "dynamicToolCall" as const,
    id: "item-1",
    namespace: "functions",
    tool: "read",
    arguments: { path: "/tmp/file" },
    status: "completed" as const,
    contentItems: [{ text: "same output" }],
    success: true,
    durationMs: 10,
  };
  const firstTurn = {
    id: "first-turn",
    items: [item],
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  };
  const secondTurn = {
    ...firstTurn,
    id: "second-turn",
    items: [
      {
        ...item,
        id: "item-2",
      },
    ],
  };

  const merged = mergeThreadSnapshot(null, {
    ...makeThread(),
    turns: [firstTurn, secondTurn],
  });

  assert.deepEqual(merged.turns, [firstTurn, secondTurn]);
});

test("mergeThreadSnapshot preserves duplicate context compaction markers in one snapshot", () => {
  const firstTurn = {
    id: "first-turn",
    items: [
      {
        type: "contextCompaction" as const,
        id: "first-item",
      },
    ],
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  };
  const secondTurn = {
    ...firstTurn,
    id: "second-turn",
    items: [
      {
        ...firstTurn.items[0],
        id: "second-item",
      },
    ],
  };

  const merged = mergeThreadSnapshot(null, {
    ...makeThread(),
    turns: [firstTurn, secondTurn],
  });

  assert.deepEqual(merged.turns, [firstTurn, secondTurn]);
});

test("mergeThreadSnapshot preserves an in-flight turn missing from a stale snapshot", () => {
  const existing = {
    ...makeThread(),
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "collabAgentMessage" as const,
            id: "item-1",
            operation: "send_message",
            senderThreadId: "thread-2",
            senderPath: "/root/worker",
            recipientThreadId: "thread-1",
            recipientPath: "/root",
            otherRecipientPaths: [],
            content: "new backend message",
            triggerTurn: true,
          },
        ],
        itemsView: "full" as const,
        status: "running" as const,
        error: null,
        startedAt: 1,
        completedAt: null,
        durationMs: null,
      },
    ],
  };

  const merged = mergeThreadSnapshot(existing, {
    ...makeThread(),
    turns: [],
  });

  assert.deepEqual(merged.turns, existing.turns);
});

test("mergeThreadSnapshot drops duplicate in-flight items already present in the read snapshot", () => {
  const restoredTurn = {
    id: "restored-turn",
    items: [
      {
        type: "collabAgentMessage" as const,
        id: "restored-item",
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
    itemsView: "full" as const,
    status: "running" as const,
    error: null,
    startedAt: 10,
    completedAt: null,
    durationMs: null,
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
    status: "completed" as const,
    completedAt: 12,
    durationMs: 2000,
  };

  const merged = mergeThreadSnapshot(
    {
      ...makeThread(),
      turns: [restoredTurn],
    },
    {
      ...makeThread(),
      turns: [readTurn],
    },
  );

  assert.deepEqual(merged.turns, [readTurn]);
});

test("mergeThreadSnapshot drops duplicate completed live agent turns already present in the read snapshot", () => {
  const liveDelta = appendAgentDelta(
    makeThread(),
    "live-turn",
    "live-item",
    "same response",
  );
  const completedLive = updateThreadTurn(liveDelta, {
    id: "live-turn",
    items: [],
    itemsView: "notLoaded",
    status: "completed",
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  });
  const readTurn = {
    id: "read-turn",
    items: [
      {
        type: "agentMessage" as const,
        id: "read-item",
        text: "same response",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  };

  const merged = mergeThreadSnapshot(completedLive, {
    ...makeThread(),
    turns: [readTurn],
  });

  assert.deepEqual(merged.turns, [readTurn]);
});

test("mergeThreadSnapshot preserves completed full agent turns with matching content", () => {
  const liveTurn = {
    id: "live-turn",
    items: [
      {
        type: "agentMessage" as const,
        id: "live-item",
        text: "same response",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  };
  const readTurn = {
    ...liveTurn,
    id: "read-turn",
    items: [
      {
        ...liveTurn.items[0],
        id: "read-item",
      },
    ],
  };

  const merged = mergeThreadSnapshot(
    {
      ...makeThread(),
      turns: [liveTurn],
    },
    {
      ...makeThread(),
      turns: [readTurn],
    },
  );

  assert.deepEqual(merged.turns, [readTurn, liveTurn]);
});

test("mergeThreadSnapshot only matches one existing item per semantic read item", () => {
  const restoredTurn = {
    id: "restored-turn",
    items: [
      {
        type: "agentMessage" as const,
        id: "restored-item-1",
        text: "same response",
        phase: null,
        memoryCitation: null,
      },
      {
        type: "agentMessage" as const,
        id: "restored-item-2",
        text: "same response",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "full" as const,
    status: "running" as const,
    error: null,
    startedAt: 10,
    completedAt: null,
    durationMs: null,
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
    status: "completed" as const,
    completedAt: 12,
    durationMs: 2000,
  };

  const merged = mergeThreadSnapshot(
    {
      ...makeThread(),
      turns: [restoredTurn],
    },
    {
      ...makeThread(),
      turns: [readTurn],
    },
  );

  assert.deepEqual(merged.turns, [
    readTurn,
    {
      ...restoredTurn,
      items: [restoredTurn.items[1]],
    },
  ]);
});

test("mergeThreadSnapshot preserves distinct in-flight items with matching content", () => {
  const readTurn = {
    id: "read-turn",
    items: [
      {
        type: "collabAgentMessage" as const,
        id: "read-item",
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
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  };
  const liveTurn = {
    ...readTurn,
    id: "live-turn",
    items: [
      {
        ...readTurn.items[0],
        id: "live-item",
      },
    ],
    status: "running" as const,
    startedAt: 20,
    completedAt: null,
    durationMs: null,
  };

  const merged = mergeThreadSnapshot(
    {
      ...makeThread(),
      turns: [liveTurn],
    },
    {
      ...makeThread(),
      turns: [readTurn],
    },
  );

  assert.deepEqual(merged.turns, [readTurn, liveTurn]);
});

test("updateThreadTurn drops duplicate in-flight items when a completed turn arrives with new ids", () => {
  const runningTurn = {
    id: "running-turn",
    items: [
      {
        type: "agentMessage" as const,
        id: "running-item",
        text: "same response",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "full" as const,
    status: "running" as const,
    error: null,
    startedAt: 10,
    completedAt: null,
    durationMs: null,
  };
  const completedTurn = {
    ...runningTurn,
    id: "completed-turn",
    items: [
      {
        ...runningTurn.items[0],
        id: "completed-item",
      },
    ],
    status: "completed" as const,
    completedAt: 12,
    durationMs: 2000,
  };

  const updated = updateThreadTurn(
    {
      ...makeThread(),
      turns: [runningTurn],
    },
    completedTurn,
  );

  assert.deepEqual(updated.turns, [completedTurn]);
});

test("updateThreadTurn drops duplicate placeholder delta turns when a completed turn arrives with new ids", () => {
  const placeholderThread = appendAgentDelta(
    makeThread(),
    "placeholder-turn",
    "placeholder-item",
    "same response",
  );
  const completedTurn = {
    id: "completed-turn",
    items: [
      {
        type: "agentMessage" as const,
        id: "completed-item",
        text: "same response",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  };

  const updated = updateThreadTurn(placeholderThread, completedTurn);

  assert.deepEqual(updated.turns, [completedTurn]);
});

test("updateThreadTurn preserves distinct running items when appending a same-content turn from another time", () => {
  const completedTurn = {
    id: "completed-turn",
    items: [
      {
        type: "agentMessage" as const,
        id: "completed-item",
        text: "same response",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  };
  const runningTurn = {
    ...completedTurn,
    id: "running-turn",
    items: [
      {
        ...completedTurn.items[0],
        id: "running-item",
      },
    ],
    status: "running" as const,
    startedAt: 20,
    completedAt: null,
    durationMs: null,
  };

  const updated = updateThreadTurn(
    {
      ...makeThread(),
      turns: [runningTurn],
    },
    completedTurn,
  );

  assert.deepEqual(updated.turns, [runningTurn, completedTurn]);
});

test("appendAgentDelta preserves later same-content assistant turns", () => {
  const completedTurn = {
    id: "completed-turn",
    items: [
      {
        type: "agentMessage" as const,
        id: "completed-item",
        text: "same response",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  };
  const thread = {
    ...makeThread(),
    turns: [completedTurn],
  };

  const updated = appendAgentDelta(
    thread,
    "later-turn",
    "later-item",
    "same response",
  );

  assert.deepEqual(updated.turns, [
    completedTurn,
    {
      id: "later-turn",
      items: [
        {
          type: "agentMessage",
          id: "later-item",
          text: "same response",
          phase: null,
          memoryCitation: null,
        },
      ],
      itemsView: "full",
      status: "running",
      error: null,
      startedAt: null,
      completedAt: null,
      durationMs: null,
    },
  ]);
});

test("mergeThreadSnapshot keeps the more complete in-flight agent message text", () => {
  const existing = {
    ...makeThread(),
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "agentMessage" as const,
            id: "item-1",
            text: "hello world",
            phase: null,
            memoryCitation: null,
          },
        ],
        itemsView: "full" as const,
        status: "running" as const,
        error: null,
        startedAt: 1,
        completedAt: null,
        durationMs: null,
      },
    ],
  };

  const merged = mergeThreadSnapshot(existing, {
    ...makeThread(),
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "agentMessage" as const,
            id: "item-1",
            text: "hello",
            phase: null,
            memoryCitation: null,
          },
        ],
        itemsView: "full" as const,
        status: "running" as const,
        error: null,
        startedAt: 1,
        completedAt: null,
        durationMs: null,
      },
    ],
  });

  assert.equal(merged.turns[0]?.items[0]?.type, "agentMessage");
  if (merged.turns[0]?.items[0]?.type !== "agentMessage") {
    assert.fail("expected an agent message item");
  }
  assert.equal(merged.turns[0].items[0].text, "hello world");
});

test("updateThreadItem merges same-turn event-driven items with different ids", () => {
  const thread = {
    ...makeThread(),
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "eventDrivenTool" as const,
            id: "snapshot-item",
            tool: "fs_subscribe",
            title: "File watch triggered",
            text: "build.log changed",
            completedAtMs: 100,
          },
        ],
        itemsView: "full" as const,
        status: "running" as const,
        error: null,
        startedAt: 1,
        completedAt: null,
        durationMs: null,
      },
    ],
  };

  const updated = updateThreadItem(thread, "turn-1", {
    type: "eventDrivenTool",
    id: "live-item",
    tool: "fs_subscribe",
    title: "File watch triggered",
    text: "build.log changed",
    completedAtMs: 200,
  });

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "eventDrivenTool",
      id: "live-item",
      tool: "fs_subscribe",
      title: "File watch triggered",
      text: "build.log changed",
      completedAtMs: 200,
    },
  ]);
});

test("updateThreadItem merges same-turn agent delta placeholder with completed item", () => {
  const thread = appendAgentDelta(
    makeThread(),
    "turn-1",
    "delta-item",
    "same response",
  );

  const updated = updateThreadItem(
    thread,
    "turn-1",
    {
      type: "agentMessage",
      id: "completed-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
    },
    { completedAtMs: 2_000 },
  );

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "agentMessage",
      id: "completed-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
      completedAtMs: 2_000,
    },
  ]);
});

test("updateThreadItem keeps the more complete same-turn agent text", () => {
  const thread = appendAgentDelta(makeThread(), "turn-1", "delta-item", "same");

  const updated = updateThreadItem(
    thread,
    "turn-1",
    {
      type: "agentMessage",
      id: "completed-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
    },
    { completedAtMs: 2_000 },
  );

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "agentMessage",
      id: "completed-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
      completedAtMs: 2_000,
    },
  ]);
});

test("updateThreadItem does not overwrite an existing agent message with a prefix-compatible item", () => {
  const thread = updateThreadItem(makeThread(), "turn-1", {
    type: "agentMessage",
    id: "first-item",
    text: "same",
    phase: null,
    memoryCitation: null,
  });

  const updated = updateThreadItem(
    thread,
    "turn-1",
    {
      type: "agentMessage",
      id: "second-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
    },
    { completedAtMs: 3_000 },
  );

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "agentMessage",
      id: "first-item",
      text: "same",
      phase: null,
      memoryCitation: null,
    },
    {
      type: "agentMessage",
      id: "second-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
      completedAtMs: 3_000,
    },
  ]);
});

test("updateThreadItem preserves same-text agent messages unless the existing item is a delta placeholder", () => {
  const thread = updateThreadItem(makeThread(), "turn-1", {
    type: "agentMessage",
    id: "first-item",
    text: "same response",
    phase: null,
    memoryCitation: null,
  });

  const updated = updateThreadItem(thread, "turn-1", {
    type: "agentMessage",
    id: "second-item",
    text: "same response",
    phase: null,
    memoryCitation: null,
  });

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "agentMessage",
      id: "first-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
    },
    {
      type: "agentMessage",
      id: "second-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
    },
  ]);
});

test("updateThreadItem merges a completed agent message into the latest matching delta placeholder", () => {
  const threadWithFirst = appendAgentDelta(
    makeThread(),
    "turn-1",
    "first-delta-item",
    "same response",
  );
  const thread = appendAgentDelta(
    threadWithFirst,
    "turn-1",
    "second-delta-item",
    "same",
  );

  const updated = updateThreadItem(
    thread,
    "turn-1",
    {
      type: "agentMessage",
      id: "completed-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
    },
    { completedAtMs: 3_000 },
  );

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "agentMessage",
      id: "first-delta-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
    },
    {
      type: "agentMessage",
      id: "completed-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
      completedAtMs: 3_000,
    },
  ]);
});

test("updateThreadItem preserves distinct completed agent messages with matching text", () => {
  const thread = updateThreadItem(
    makeThread(),
    "turn-1",
    {
      type: "agentMessage",
      id: "first-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
    },
    { completedAtMs: 2_000 },
  );

  const updated = updateThreadItem(
    thread,
    "turn-1",
    {
      type: "agentMessage",
      id: "second-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
    },
    { completedAtMs: 3_000 },
  );

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "agentMessage",
      id: "first-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
      completedAtMs: 2_000,
    },
    {
      type: "agentMessage",
      id: "second-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
      completedAtMs: 3_000,
    },
  ]);
});

test("updateThreadItem merges same-turn collab agent messages with different ids", () => {
  const thread = updateThreadItem(makeThread(), "turn-1", {
    type: "collabAgentMessage",
    id: "live-item",
    operation: "send_message",
    senderThreadId: "thread-2",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    otherRecipientPaths: [],
    content: "same backend message",
    triggerTurn: true,
  });

  const updated = updateThreadItem(thread, "turn-1", {
    type: "collabAgentMessage",
    id: "completed-item",
    operation: "send_message",
    senderThreadId: "thread-2",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    otherRecipientPaths: [],
    content: "same backend message",
    triggerTurn: true,
  });

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "collabAgentMessage",
      id: "completed-item",
      operation: "send_message",
      senderThreadId: "thread-2",
      senderPath: "/root/worker",
      recipientThreadId: "thread-1",
      recipientPath: "/root",
      otherRecipientPaths: [],
      content: "same backend message",
      triggerTurn: true,
    },
  ]);
});

test("updateThreadItem preserves separate context compaction markers", () => {
  const thread = updateThreadItem(makeThread(), "turn-1", {
    type: "contextCompaction",
    id: "first-item",
  });

  const updated = updateThreadItem(thread, "turn-1", {
    type: "contextCompaction",
    id: "second-item",
  });

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "contextCompaction",
      id: "first-item",
    },
    {
      type: "contextCompaction",
      id: "second-item",
    },
  ]);
});

test("updateThreadItem preserves repeated dynamic tool calls with matching content", () => {
  const thread = updateThreadItem(makeThread(), "turn-1", {
    type: "dynamicToolCall",
    id: "first-item",
    namespace: "functions",
    tool: "read",
    arguments: { path: "/tmp/file" },
    status: "completed",
    contentItems: [{ text: "same output" }],
    success: true,
    durationMs: 10,
  });

  const updated = updateThreadItem(thread, "turn-1", {
    type: "dynamicToolCall",
    id: "second-item",
    namespace: "functions",
    tool: "read",
    arguments: { path: "/tmp/file" },
    status: "completed",
    contentItems: [{ text: "same output" }],
    success: true,
    durationMs: 10,
  });

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "dynamicToolCall",
      id: "first-item",
      namespace: "functions",
      tool: "read",
      arguments: { path: "/tmp/file" },
      status: "completed",
      contentItems: [{ text: "same output" }],
      success: true,
      durationMs: 10,
    },
    {
      type: "dynamicToolCall",
      id: "second-item",
      namespace: "functions",
      tool: "read",
      arguments: { path: "/tmp/file" },
      status: "completed",
      contentItems: [{ text: "same output" }],
      success: true,
      durationMs: 10,
    },
  ]);
});

test("pending thread updates replay when the thread snapshot arrives", () => {
  const pendingUpdates = new Map<string, Array<(thread: Thread) => Thread>>();
  queuePendingThreadUpdate(pendingUpdates, "thread-1", (thread) =>
    updateThreadItem(thread, "turn-1", {
      type: "collabAgentStatusUpdate",
      id: "subagent-complete",
      senderThreadId: "thread-child",
      senderPath: "/root/worker",
      recipientThreadId: "thread-1",
      recipientPath: "/root",
      status: {
        status: "completed",
        message: "done",
      },
    }),
  );

  const updated = applyPendingThreadUpdates(makeThread(), pendingUpdates);

  assert.equal(pendingUpdates.size, 0);
  assert.deepEqual(updated.turns, [
    {
      id: "turn-1",
      items: [
        {
          type: "collabAgentStatusUpdate",
          id: "subagent-complete",
          senderThreadId: "thread-child",
          senderPath: "/root/worker",
          recipientThreadId: "thread-1",
          recipientPath: "/root",
          status: {
            status: "completed",
            message: "done",
          },
        },
      ],
      itemsView: "full",
      status: "running",
      error: null,
      startedAt: null,
      completedAt: null,
      durationMs: null,
    },
  ]);
});

for (const terminalStatus of [
  "completed",
  "errored",
  "shutdown",
  "notFound",
]) {
  test(`updateThreadItem preserves repeated terminal collab status updates for ${terminalStatus}`, () => {
    const thread = updateThreadItem(makeThread(), "turn-1", {
      type: "collabAgentStatusUpdate",
      id: "first-completion",
      senderThreadId: "thread-child",
      senderPath: "/root/worker",
      recipientThreadId: "thread-1",
      recipientPath: "/root",
      status: {
        status: terminalStatus,
        message: "done",
      },
    });

    const updated = updateThreadItem(thread, "turn-1", {
      type: "collabAgentStatusUpdate",
      id: "second-completion",
      senderThreadId: "thread-child",
      senderPath: "/root/worker",
      recipientThreadId: "thread-1",
      recipientPath: "/root",
      status: {
        status: terminalStatus,
        message: "done",
      },
    });

    assert.deepEqual(updated.turns[0]?.items, [
      {
        type: "collabAgentStatusUpdate",
        id: "first-completion",
        senderThreadId: "thread-child",
        senderPath: "/root/worker",
        recipientThreadId: "thread-1",
        recipientPath: "/root",
        status: {
          status: terminalStatus,
          message: "done",
        },
      },
      {
        type: "collabAgentStatusUpdate",
        id: "second-completion",
        senderThreadId: "thread-child",
        senderPath: "/root/worker",
        recipientThreadId: "thread-1",
        recipientPath: "/root",
        status: {
          status: terminalStatus,
          message: "done",
        },
      },
    ]);
  });
}

test("pending thread updates do not duplicate semantic items already in the snapshot", () => {
  const pendingUpdates = new Map<string, Array<(thread: Thread) => Thread>>();
  queuePendingThreadUpdate(pendingUpdates, "thread-1", (thread) =>
    updateThreadItem(thread, "turn-1", {
      type: "collabAgentMessage",
      id: "pending-item",
      operation: "send_message",
      senderThreadId: "thread-child",
      senderPath: "/root/worker",
      recipientThreadId: "thread-1",
      recipientPath: "/root",
      otherRecipientPaths: [],
      content: "same backend message",
      triggerTurn: true,
    }),
  );
  const snapshot = {
    ...makeThread(),
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "collabAgentMessage" as const,
            id: "snapshot-item",
            operation: "send_message",
            senderThreadId: "thread-child",
            senderPath: "/root/worker",
            recipientThreadId: "thread-1",
            recipientPath: "/root",
            otherRecipientPaths: [],
            content: "same backend message",
            triggerTurn: true,
          },
        ],
        itemsView: "full" as const,
        status: "running" as const,
        error: null,
        startedAt: 1,
        completedAt: null,
        durationMs: null,
      },
    ],
  };

  const updated = applyPendingThreadUpdates(snapshot, pendingUpdates);

  assert.equal(pendingUpdates.size, 0);
  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "collabAgentMessage",
      id: "pending-item",
      operation: "send_message",
      senderThreadId: "thread-child",
      senderPath: "/root/worker",
      recipientThreadId: "thread-1",
      recipientPath: "/root",
      otherRecipientPaths: [],
      content: "same backend message",
      triggerTurn: true,
    },
  ]);
});

test("pending agent deltas do not duplicate completed assistant blocks already in the snapshot", () => {
  const pendingUpdates = new Map<string, Array<(thread: Thread) => Thread>>();
  queuePendingThreadUpdate(pendingUpdates, "thread-1", (thread) =>
    appendAgentDelta(thread, "pending-turn", "pending-item", "same response"),
  );
  const snapshot = {
    ...makeThread(),
    turns: [
      {
        id: "snapshot-turn",
        items: [
          {
            type: "agentMessage" as const,
            id: "snapshot-item",
            text: "same response",
            phase: null,
            memoryCitation: null,
          },
        ],
        itemsView: "full" as const,
        status: "completed" as const,
        error: null,
        startedAt: 10,
        completedAt: 12,
        durationMs: 2000,
      },
    ],
  };

  const updated = applyPendingThreadUpdates(snapshot, pendingUpdates);

  assert.equal(pendingUpdates.size, 0);
  assert.deepEqual(updated.turns, snapshot.turns);
});

test("pending agent deltas do not duplicate structured process exit events already in the snapshot", () => {
  const pendingUpdates = new Map<string, Array<(thread: Thread) => Thread>>();
  queuePendingThreadUpdate(pendingUpdates, "thread-1", (thread) =>
    appendAgentDelta(
      thread,
      "pending-turn",
      "pending-item",
      eventDrivenToolMarker(),
    ),
  );
  const snapshot = {
    ...makeThread(),
    turns: [
      {
        id: "snapshot-turn",
        items: [
          {
            type: "eventDrivenTool" as const,
            id: "snapshot-item",
            tool: "process_exit_subscribe",
            title: "Process exited",
            text: "Session 42 exited with code 0",
          },
        ],
        itemsView: "full" as const,
        status: "completed" as const,
        error: null,
        startedAt: 10,
        completedAt: 12,
        durationMs: 2000,
      },
    ],
  };

  const updated = applyPendingThreadUpdates(snapshot, pendingUpdates);

  assert.equal(pendingUpdates.size, 0);
  assert.deepEqual(updated.turns, snapshot.turns);
});

test("threadStatusClass treats active thread status as doing", () => {
  assert.equal(
    threadStatusClass({
      type: "active",
      activeFlags: [],
    }),
    "doing",
  );
});

test("treeThreadStatusClass keeps self active thread green", () => {
  const thread = {
    ...makeThread(),
    status: {
      type: "active" as const,
      activeFlags: [],
    },
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "agentMessage" as const,
            id: "item-1",
            text: "working",
            phase: null,
            memoryCitation: null,
          },
        ],
        itemsView: "full" as const,
        status: "running" as const,
        error: null,
        startedAt: 1,
        completedAt: null,
        durationMs: null,
      },
    ],
  };

  assert.equal(treeThreadStatusClass(makeTreeNode(thread)), "doing");
});

test("treeThreadStatusClass shows subagent waiting separately", () => {
  const thread = {
    ...makeThread(),
    status: {
      type: "active" as const,
      activeFlags: [],
    },
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "collabAgentToolCall" as const,
            id: "item-1",
            tool: "Wait",
            status: "InProgress",
            senderThreadId: "thread-1",
            senderPath: "/root",
            receiverThreadIds: ["thread-2"],
            receiverPaths: ["/root/worker"],
            prompt: null,
            model: null,
            reasoningEffort: null,
            agentsStates: {},
          },
        ],
        itemsView: "full" as const,
        status: "inProgress" as const,
        error: null,
        startedAt: 1,
        completedAt: null,
        durationMs: null,
      },
    ],
  };

  assert.equal(treeThreadStatusClass(makeTreeNode(thread)), "waiting-subagent");
  assert.equal(
    treeThreadStatusLabel("waiting-subagent"),
    "Waiting on subagent",
  );
});

test("treeThreadStatusClass shows event tool waiting separately", () => {
  const thread = {
    ...makeThread(),
    status: {
      type: "active" as const,
      activeFlags: [],
    },
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "eventCommandCall" as const,
            id: "item-1",
            subscriptionId: "sub-1",
            command: "tail -f /tmp/build.log",
            cwd: "/tmp",
            label: "build log",
            status: "completed",
            output: { subscription_id: "sub-1" },
          },
        ],
        itemsView: "full" as const,
        status: "completed" as const,
        error: null,
        startedAt: 1,
        completedAt: 2,
        durationMs: 1000,
      },
    ],
  };

  assert.equal(
    treeThreadStatusClass(makeTreeNode(thread)),
    "waiting-eventtool",
  );
});

test("treeThreadStatusClass ignores process exit restore failures", () => {
  const thread = {
    ...makeThread(),
    status: {
      type: "active" as const,
      activeFlags: [],
    },
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "eventDrivenToolCall" as const,
            id: "item-1",
            tool: "process_exit_subscribe",
            arguments: { session_id: 42, label: "build process" },
            status: "completed",
            output: { subscription_id: "sub-1" },
          },
          {
            type: "eventDrivenTool" as const,
            id: "item-2",
            tool: "process_exit_subscribe",
            title: "Process exit restore failed",
            text: "[Process exit subscription restore (build process)] Could not restore session 42 after restart because the original exec session is no longer available.",
          },
        ],
        itemsView: "full" as const,
        status: "completed" as const,
        error: null,
        startedAt: 1,
        completedAt: 2,
        durationMs: 1000,
      },
    ],
  };

  assert.equal(treeThreadStatusClass(makeTreeNode(thread)), "doing");
});

test("treeThreadStatusClass ignores event tool subscriptions after unsubscribe", () => {
  const thread = {
    ...makeThread(),
    status: {
      type: "active" as const,
      activeFlags: [],
    },
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "eventDrivenToolCall" as const,
            id: "item-1",
            tool: "process_exit_subscribe",
            arguments: { session_id: 42 },
            status: "completed",
            output: { subscription_id: "sub-1" },
          },
          {
            type: "eventDrivenToolCall" as const,
            id: "item-2",
            tool: "process_exit_unsubscribe",
            arguments: { subscription_id: "sub-1" },
            status: "completed",
            output: { ok: true },
          },
        ],
        itemsView: "full" as const,
        status: "completed" as const,
        error: null,
        startedAt: 1,
        completedAt: 2,
        durationMs: 1000,
      },
    ],
  };

  assert.equal(treeThreadStatusClass(makeTreeNode(thread)), "doing");
});

test("treeThreadStatusClass rolls descendant active into parent waiting", () => {
  const parent = {
    ...makeThread(),
    id: "parent",
  };
  const child = {
    ...makeThread(),
    id: "child",
    status: {
      type: "active" as const,
      activeFlags: [],
    },
  };

  assert.equal(
    treeThreadStatusClass(makeTreeNode(parent, [makeTreeNode(child)])),
    "waiting-subagent",
  );
});

test("treeThreadStatusClass prioritizes active descendants over event tool waits", () => {
  const parent = {
    ...makeThread(),
    id: "parent",
  };
  const eventToolChild = {
    ...makeThread(),
    id: "event-tool-child",
    status: {
      type: "active" as const,
      activeFlags: [],
    },
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "eventDrivenToolCall" as const,
            id: "item-1",
            tool: "process_exit_subscribe",
            arguments: { session_id: 42 },
            status: "completed",
            output: { subscription_id: "sub-1" },
          },
        ],
        itemsView: "full" as const,
        status: "completed" as const,
        error: null,
        startedAt: 1,
        completedAt: 2,
        durationMs: 1000,
      },
    ],
  };
  const activeChild = {
    ...makeThread(),
    id: "active-child",
    status: {
      type: "active" as const,
      activeFlags: [],
    },
  };

  assert.equal(
    treeThreadStatusClass(
      makeTreeNode(parent, [
        makeTreeNode(eventToolChild),
        makeTreeNode(activeChild),
      ]),
    ),
    "waiting-subagent",
  );
});

test("treeThreadStatusClass rolls descendant system errors into parent blocked", () => {
  const parent = {
    ...makeThread(),
    id: "parent",
  };
  const child = {
    ...makeThread(),
    id: "child",
    status: {
      type: "systemError" as const,
    },
  };

  assert.equal(
    treeThreadStatusClass(makeTreeNode(parent, [makeTreeNode(child)])),
    "blocked",
  );
});

test("treeThreadStatusClass leaves inactive trees unchanged", () => {
  const parent = {
    ...makeThread(),
    id: "parent",
  };
  const child = {
    ...makeThread(),
    id: "child",
  };

  assert.equal(
    treeThreadStatusClass(makeTreeNode(parent, [makeTreeNode(child)])),
    "todo",
  );
});

test("thread path helpers read snake_case thread spawn metadata", () => {
  const thread = {
    ...makeThread(),
    source: {
      subAgent: {
        thread_spawn: {
          parent_thread_id: "parent-1",
          depth: 1,
          agent_path: "/root/worker",
          agent_nickname: "worker",
          agent_role: "Worker Agent",
        },
      },
    },
  } satisfies Thread;

  assert.equal(getThreadPath(thread), "/root/worker");
  assert.equal(getParentThreadId(thread), "parent-1");
});

test("thread path helpers read camelCase thread spawn metadata", () => {
  const thread = {
    ...makeThread(),
    source: {
      subAgent: {
        threadSpawn: {
          parentThreadId: "parent-2",
          depth: 1,
          agentPath: "/root/reviewer",
          agentNickname: "reviewer",
          agentRole: "Reviewer",
        },
      },
    } as unknown as Thread["source"],
  };

  assert.equal(getThreadPath(thread), "/root/reviewer");
  assert.equal(getParentThreadId(thread), "parent-2");
});

test("buildCurrentThreadTodoItems only returns direct child threads", () => {
  const directChildUpdatedAt = Math.floor(Date.now() / 1000);
  const parent = {
    ...makeThread(),
    id: "parent",
    updatedAt: 1,
  } satisfies Thread;
  const directChild = {
    ...makeThread(),
    id: "child",
    updatedAt: directChildUpdatedAt,
    name: "Direct Child",
    source: {
      subAgent: {
        thread_spawn: {
          parent_thread_id: "parent",
          depth: 1,
          agent_path: "/root/child",
          agent_nickname: "child",
          agent_role: "worker",
        },
      },
    },
  } satisfies Thread;
  const sibling = {
    ...makeThread(),
    id: "sibling",
    updatedAt: 5,
    name: "Sibling",
    source: {
      subAgent: {
        thread_spawn: {
          parent_thread_id: "other-parent",
          depth: 1,
          agent_path: "/root/sibling",
          agent_nickname: "sibling",
          agent_role: "worker",
        },
      },
    },
  } satisfies Thread;
  const grandchild = {
    ...makeThread(),
    id: "grandchild",
    updatedAt: 6,
    name: "Grandchild",
    source: {
      subAgent: {
        thread_spawn: {
          parent_thread_id: "child",
          depth: 2,
          agent_path: "/root/child/grandchild",
          agent_nickname: "grandchild",
          agent_role: "worker",
        },
      },
    },
  } satisfies Thread;

  assert.deepEqual(
    buildCurrentThreadTodoItems(
      [parent, directChild, sibling, grandchild],
      "parent",
      "all",
    ),
    [
      {
        id: "child",
        title: "Direct Child",
        ownerPath: "/root/child",
        status: "todo",
        statusLabel: "Todo",
        updatedLabel: formatUpdatedLabel(directChildUpdatedAt),
        summary: "",
        threadId: "child",
      },
    ],
  );
});

test("getPresenceLabel surfaces active thread flags", () => {
  assert.equal(
    getPresenceLabel({
      type: "active",
      activeFlags: ["waitingOnApproval"],
    }),
    "Waiting on Approval",
  );
  assert.equal(
    getPresenceLabel({
      type: "active",
      activeFlags: ["waitingOnUserInput"],
    }),
    "Waiting on Input",
  );
  assert.equal(
    getPresenceLabel({
      type: "active",
      activeFlags: [],
    }),
    "Active",
  );
});

test("isThreadThinking stays false while a turn only injects init context", () => {
  const thread = {
    ...makeThread(),
    status: {
      type: "active" as const,
      activeFlags: [],
    },
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "userMessage" as const,
            id: "msg-1",
            content: [{ type: "text", text: "hello" }],
          },
          {
            type: "injectedContext" as const,
            id: "ctx-1",
            title: "environment",
            preview: "workspace context",
            sections: [],
          },
        ],
        itemsView: "full" as const,
        status: "inProgress" as const,
        error: null,
        startedAt: 1,
        completedAt: null,
        durationMs: null,
      },
    ],
  };

  assert.equal(
    isThreadThinking(thread, {
      isLoadingThread: false,
      isSending: false,
    }),
    false,
  );
});

test("isThreadThinking stays false when thread is only active for subscriptions", () => {
  const thread = {
    ...makeThread(),
    status: {
      type: "active" as const,
      activeFlags: [],
    },
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "eventDrivenToolCall" as const,
            id: "sub-1",
            tool: "schedule_subscribe",
            arguments: { delay_ms: 60000 },
            status: "completed",
            output: { ok: true },
          },
        ],
        itemsView: "full" as const,
        status: "completed" as const,
        error: null,
        startedAt: 1,
        completedAt: 2,
        durationMs: 1000,
      },
    ],
  };

  assert.equal(
    isThreadThinking(thread, {
      isLoadingThread: false,
      isSending: false,
    }),
    false,
  );
});

test("isThreadThinking turns true once non-context output begins", () => {
  const thread = {
    ...makeThread(),
    status: {
      type: "active" as const,
      activeFlags: [],
    },
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "userMessage" as const,
            id: "msg-1",
            content: [{ type: "text", text: "hello" }],
          },
          {
            type: "commandExecution" as const,
            id: "cmd-1",
            command: "rtk git status --short",
            cwd: "/tmp",
            status: "running",
            aggregatedOutput: null,
            exitCode: null,
            durationMs: null,
          },
        ],
        itemsView: "full" as const,
        status: "inProgress" as const,
        error: null,
        startedAt: 1,
        completedAt: null,
        durationMs: null,
      },
    ],
  };

  assert.equal(
    isThreadThinking(thread, {
      isLoadingThread: false,
      isSending: false,
    }),
    true,
  );
});
