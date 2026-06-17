import test from "node:test";
import assert from "node:assert/strict";

import { buildConversationEntries } from "./conversation";
import {
  appendAgentDelta,
  applyInitializedThreadUpdate,
  applyPendingThreadUpdates,
  buildCurrentThreadTodoItems,
  formatUpdatedLabel,
  getPresenceLabel,
  getParentThreadId,
  getThreadPresenceLabel,
  getThreadItemNotificationSyntheticTurnStatus,
  getThreadItemNotificationTargetThreadIds,
  getThreadPath,
  isThreadThinking,
  mergeThreadSnapshot,
  queuePendingThreadUpdate,
  threadDisplayStatusClass,
  threadStatusClass,
  treeThreadStatusClass,
  treeThreadStatusLabel,
  updateThreadItem,
  updateThreadTurn,
  updateThreadTurnLifecycle,
  upsertThread,
  upsertThreadMetadataPreservingTurns,
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

test("command wait item notifications create visible conversation entries", () => {
  const updated = updateThreadItem(
    makeThread(),
    "turn-1",
    {
      type: "commandWait",
      id: "wait-1",
      commandId: "7",
      status: "completed",
      notification: "exit",
      exitCode: 0,
      wallTimeSeconds: 0.25,
      waitTimeoutMs: 1_000,
      createdAtMs: 2_000,
    },
    { completedAtMs: 2_000 },
  );

  const entries = buildConversationEntries(updated);

  assert.deepEqual(
    entries.map((entry) => [entry.id, entry.kind, entry.text]),
    [
      [
        "wait-1",
        "event",
        "Waited for command 7 with timeout 1s after exit notification: completed, exit 0 in 250ms.",
      ],
    ],
  );
});

test("injected init context item notifications create visible conversation entries", () => {
  const updated = updateThreadItem(
    makeThread(),
    "turn-1",
    {
      type: "injectedContext",
      id: "ctx-1",
      title: "Init Context",
      preview: "Workspace • Instructions",
      sections: [{ label: "Workspace", text: "/tmp/project" }],
    },
    { completedAtMs: 2_000 },
  );

  const entries = buildConversationEntries(updated);

  assert.deepEqual(
    entries.map((entry) => ({
      id: entry.id,
      kind: entry.kind,
      text: entry.text,
      toolName: entry.toolName,
      toolCategory: entry.toolCategory,
    })),
    [
      {
        id: "ctx-1",
        kind: "tool",
        text: "Workspace • Instructions",
        toolName: "Init Context",
        toolCategory: "context",
      },
    ],
  );
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

test("mergeThreadSnapshot preserves same-content items with different ids", () => {
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

  assert.deepEqual(merged.turns, thread.turns);
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

  assert.deepEqual(threads, [thread]);
});

test("mergeThreadSnapshot preserves live-derived turns with different item ids within next snapshot", () => {
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

  assert.deepEqual(merged.turns, [liveTurn, readTurn]);
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
            operation: "sendMessage",
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

test("mergeThreadSnapshot preserves init context when a started snapshot omits turns", () => {
  const initContextTurn: Turn = {
    id: "turn-init",
    items: [
      {
        type: "injectedContext",
        id: "ctx-1",
        title: "Init Context",
        preview: "Developer",
        sections: [
          {
            label: "Developer",
            text: "Agent type file body: always inspect the active task.",
          },
        ],
      },
    ],
    itemsView: "full",
    status: "completed",
    error: null,
    startedAt: 1,
    completedAt: 2,
    durationMs: 1000,
  };
  const existing = {
    ...makeThread(),
    turns: [initContextTurn],
  };

  const merged = mergeThreadSnapshot(existing, {
    ...makeThread(),
    turns: [],
  });

  assert.deepEqual(buildConversationEntries(merged).map((entry) => entry.id), [
    "ctx-1",
  ]);
});

test("mergeThreadSnapshot preserves same-content in-flight items with different ids", () => {
  const restoredTurn = {
    id: "restored-turn",
    items: [
      {
        type: "collabAgentMessage" as const,
        id: "restored-item",
        operation: "sendMessage",
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

  assert.deepEqual(merged.turns, [readTurn, restoredTurn]);
});

test("mergeThreadSnapshot preserves same-content completed live agent turns with different ids", () => {
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

  assert.deepEqual(merged.turns, [readTurn, completedLive.turns[0]]);
});

test("mergeThreadSnapshot preserves same-content live child completion with different ids", () => {
  const liveItem: ThreadItem = {
    type: "collabAgentStatusUpdate",
    id: "live-completion",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    status: {
      path: "/root/worker",
      status: "completed",
      message: "done",
    },
  };
  const liveThread = updateThreadItem(makeThread(), "live-turn", liveItem, {
    completedAtMs: 3_000,
    syntheticTurnStatus: "completed",
  });
  const readTurn = {
    id: "read-turn",
    items: [
      {
        ...liveItem,
        id: "read-completion",
      },
    ],
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 3,
    completedAt: 3,
    durationMs: null,
  };

  const merged = mergeThreadSnapshot(liveThread, {
    ...makeThread(),
    turns: [readTurn],
  });

  assert.deepEqual(merged.turns, [readTurn, liveThread.turns[0]]);
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

test("mergeThreadSnapshot preserves every same-content item with a distinct id", () => {
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
    restoredTurn,
  ]);
});

test("mergeThreadSnapshot preserves distinct in-flight items with matching content", () => {
  const readTurn = {
    id: "read-turn",
    items: [
      {
        type: "collabAgentMessage" as const,
        id: "read-item",
        operation: "sendMessage",
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

test("updateThreadTurn preserves in-flight items when a completed turn arrives with new ids", () => {
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

  assert.deepEqual(updated.turns, [runningTurn, completedTurn]);
});

test("updateThreadTurn preserves placeholder delta turns when a completed turn arrives with new ids", () => {
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

  assert.deepEqual(updated.turns, [placeholderThread.turns[0], completedTurn]);
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
  const existingTurn = {
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
  };
  const readTurn = {
    ...existingTurn,
    items: [
      {
        ...existingTurn.items[0],
        text: "hello",
      },
    ],
  };
  const existing = {
    ...makeThread(),
    turns: [existingTurn],
  };

  const merged = mergeThreadSnapshot(existing, {
    ...makeThread(),
    turns: [readTurn],
  });

  assert.equal(merged.turns.length, 1);
  assert.equal(merged.turns[0]?.id, "turn-1");
  assert.deepEqual(merged.turns[0]?.items, existingTurn.items);
});

test("updateThreadItem preserves same-turn event-driven items with different ids", () => {
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
      id: "snapshot-item",
      tool: "fs_subscribe",
      title: "File watch triggered",
      text: "build.log changed",
      completedAtMs: 100,
    },
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

test("updateThreadItem preserves same-turn agent delta placeholder with completed item", () => {
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
      id: "delta-item",
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
      completedAtMs: 2_000,
    },
  ]);
});

test("updateThreadItem preserves same-turn agent delta text when completed item has a new id", () => {
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
      id: "delta-item",
      text: "same",
      phase: null,
      memoryCitation: null,
    },
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

test("updateThreadItem preserves completed agent messages and matching delta placeholders", () => {
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
      id: "second-delta-item",
      text: "same",
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

test("updateThreadItem preserves same-turn collab agent messages with different ids", () => {
  const thread = updateThreadItem(makeThread(), "turn-1", {
    type: "collabAgentMessage",
    id: "live-item",
    operation: "sendMessage",
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
    operation: "sendMessage",
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
      id: "live-item",
      operation: "sendMessage",
      senderThreadId: "thread-2",
      senderPath: "/root/worker",
      recipientThreadId: "thread-1",
      recipientPath: "/root",
      otherRecipientPaths: [],
      content: "same backend message",
      triggerTurn: true,
    },
    {
      type: "collabAgentMessage",
      id: "completed-item",
      operation: "sendMessage",
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

for (const terminalStatus of ["completed", "errored", "shutdown", "notFound"]) {
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

test("collab status item notifications target the recipient thread", () => {
  const item: ThreadItem = {
    type: "collabAgentStatusUpdate",
    id: "subagent-complete",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    status: {
      path: "/root/worker",
      status: "completed",
      message: "done",
    },
  };

  assert.deepEqual(
    getThreadItemNotificationTargetThreadIds("thread-child", item),
    ["thread-1"],
  );

  const updatedRoot = updateThreadItem(makeThread(), "turn-child", item, {
    startedAtMs: 2_000,
    completedAtMs: 3_000,
    syntheticTurnStatus: "completed",
  });
  const entries = buildConversationEntries(updatedRoot);

  assert.equal(
    isThreadThinking(updatedRoot, {
      isLoadingThread: false,
      isSending: false,
    }),
    false,
  );
  assert.deepEqual(
    updatedRoot.turns.map((turn) => ({
      id: turn.id,
      status: turn.status,
      startedAt: turn.startedAt,
      completedAt: turn.completedAt,
      durationMs: turn.durationMs,
    })),
    [
      {
        id: "turn-child",
        status: "completed",
        startedAt: 2,
        completedAt: 3,
        durationMs: 1_000,
      },
    ],
  );
  assert.deepEqual(updatedRoot.turns[0]?.items, [
    {
      ...item,
      startedAtMs: 2_000,
      completedAtMs: 3_000,
    },
  ]);
  assert.deepEqual(
    entries.map((entry) => [entry.kind, entry.toolName, entry.text]),
    [
      [
        "tool",
        "/root/worker subagent completion",
        "/root/worker • completed • done",
      ],
    ],
  );
});

test("collab status completion notifications join the active parent turn", () => {
  const activeThread: Thread = {
    ...makeThread(),
    turns: [
      {
        id: "turn-parent",
        items: [
          {
            type: "userMessage",
            id: "user-1",
            content: [{ type: "text", text: "wait for worker" }],
          },
        ],
        itemsView: "full",
        status: "running",
        error: null,
        startedAt: 2,
        completedAt: null,
        durationMs: null,
      },
    ],
  };
  const item: ThreadItem = {
    type: "collabAgentStatusUpdate",
    id: "subagent-complete",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    status: {
      path: "/root/worker",
      status: "completed",
      message: "done",
    },
  };

  const updatedRoot = updateThreadItem(activeThread, "turn-child", item, {
    completedAtMs: 3_000,
    syntheticTurnStatus: "completed",
  });

  assert.deepEqual(
    updatedRoot.turns.map((turn) => ({
      id: turn.id,
      status: turn.status,
      startedAt: turn.startedAt,
      completedAt: turn.completedAt,
      durationMs: turn.durationMs,
    })),
    [
      {
        id: "turn-parent",
        status: "running",
        startedAt: 2,
        completedAt: null,
        durationMs: null,
      },
    ],
  );
  assert.deepEqual(updatedRoot.turns[0]?.items, [
    activeThread.turns[0]?.items[0],
    {
      ...item,
      completedAtMs: 3_000,
    },
  ]);
});

test("legacy child completion notifications join the active parent turn", () => {
  const activeThread: Thread = {
    ...makeThread(),
    turns: [
      {
        id: "turn-parent",
        items: [],
        itemsView: "full",
        status: "inProgress",
        error: null,
        startedAt: 2,
        completedAt: null,
        durationMs: null,
      },
    ],
  };
  const item: ThreadItem = {
    type: "collabAgentMessage",
    id: "child-completion",
    operation: "childCompletion",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    otherRecipientPaths: [],
    content: "done",
    triggerTurn: true,
  };

  const updatedRoot = updateThreadItem(activeThread, "turn-child", item, {
    completedAtMs: 3_000,
    syntheticTurnStatus: "completed",
  });

  assert.equal(updatedRoot.turns.length, 1);
  assert.deepEqual(updatedRoot.turns[0]?.items, [
    {
      ...item,
      completedAtMs: 3_000,
    },
  ]);
  assert.equal(updatedRoot.turns[0]?.status, "inProgress");
});

test("collab status item completion updates an existing synthetic recipient turn", () => {
  const item: ThreadItem = {
    type: "collabAgentStatusUpdate",
    id: "subagent-complete",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    status: {
      path: "/root/worker",
      status: "completed",
      message: "done",
    },
  };
  const runningRoot = updateThreadItem(makeThread(), "turn-child", item, {
    startedAtMs: 2_000,
  });

  const completedRoot = updateThreadItem(runningRoot, "turn-child", item, {
    completedAtMs: 3_000,
    syntheticTurnStatus: "completed",
  });

  assert.equal(
    isThreadThinking(completedRoot, {
      isLoadingThread: false,
      isSending: false,
    }),
    false,
  );
  assert.deepEqual(
    completedRoot.turns.map((turn) => ({
      id: turn.id,
      status: turn.status,
      startedAt: turn.startedAt,
      completedAt: turn.completedAt,
      durationMs: turn.durationMs,
    })),
    [
      {
        id: "turn-child",
        status: "completed",
        startedAt: 2,
        completedAt: 3,
        durationMs: 1_000,
      },
    ],
  );
  assert.deepEqual(completedRoot.turns[0]?.items, [
    {
      ...item,
      startedAtMs: 2_000,
      completedAtMs: 3_000,
    },
  ]);
});

test("child completion synthetic turn moves before later parent messages", () => {
  const parentThread: Thread = {
    ...makeThread(),
    turns: [
      {
        id: "turn-parent",
        items: [
          {
            type: "userMessage",
            id: "user-1",
            content: [{ type: "text", text: "wait for worker" }],
          },
        ],
        itemsView: "full",
        status: "completed",
        error: null,
        startedAt: 1,
        completedAt: 2,
        durationMs: 1_000,
      },
    ],
  };
  const childCompletion: ThreadItem = {
    type: "collabAgentStatusUpdate",
    id: "subagent-complete",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    status: {
      path: "/root/worker",
      status: "completed",
      message: "done",
    },
  };
  const withSyntheticChildCompletion = updateThreadItem(
    parentThread,
    "turn-child",
    childCompletion,
    {
      completedAtMs: 3_000,
      syntheticTurnStatus: "completed",
    },
  );

  const updated = updateThreadItem(
    withSyntheticChildCompletion,
    "turn-parent",
    {
      type: "agentMessage",
      id: "agent-after-child",
      text: "continuing after worker",
      phase: null,
      memoryCitation: null,
    },
    { completedAtMs: 4_000 },
  );

  assert.equal(updated.turns.length, 1);
  assert.deepEqual(
    buildConversationEntries(updated).map((entry) => entry.text),
    [
      "wait for worker",
      "/root/worker • completed • done",
      "continuing after worker",
    ],
  );
});

test("turn lifecycle updates preserve live child completion items", () => {
  const childCompletion: ThreadItem = {
    type: "collabAgentStatusUpdate",
    id: "subagent-complete",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    status: {
      path: "/root/worker",
      status: "completed",
      message: "done",
    },
  };
  const liveThread: Thread = {
    ...makeThread(),
    turns: [
      {
        id: "turn-parent",
        items: [
          {
            type: "userMessage",
            id: "user-1",
            content: [{ type: "text", text: "run worker" }],
          },
          {
            ...childCompletion,
            completedAtMs: 3_000,
          },
        ],
        itemsView: "full",
        status: "running",
        error: null,
        startedAt: 1,
        completedAt: null,
        durationMs: null,
      },
    ],
  };

  const updated = updateThreadTurnLifecycle(liveThread, {
    id: "turn-parent",
    items: [
      {
        type: "userMessage",
        id: "user-1",
        content: [{ type: "text", text: "run worker" }],
      },
    ],
    itemsView: "full",
    status: "completed",
    error: null,
    startedAt: 1,
    completedAt: 4,
    durationMs: 3_000,
  });

  assert.equal(updated.turns[0]?.status, "completed");
  assert.deepEqual(
    buildConversationEntries(updated).map((entry) => entry.text),
    ["run worker", "/root/worker • completed • done"],
  );
});

test("turn lifecycle updates create empty turns when local items are missing", () => {
  const updated = updateThreadTurnLifecycle(makeThread(), {
    id: "turn-lifecycle",
    items: [
      {
        type: "agentMessage",
        id: "agent-from-snapshot",
        text: "snapshot-only text",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "full",
    status: "running",
    error: null,
    startedAt: 5,
    completedAt: null,
    durationMs: null,
  });

  assert.deepEqual(updated.turns, [
    {
      id: "turn-lifecycle",
      items: [],
      itemsView: "full",
      status: "running",
      error: null,
      startedAt: 5,
      completedAt: null,
      durationMs: null,
    },
  ]);
});

test("metadata updates preserve repeated terminal collab status live items", () => {
  const firstCompletion: ThreadItem = {
    type: "collabAgentStatusUpdate",
    id: "first-completion",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    status: {
      path: "/root/worker",
      status: "completed",
      message: "done",
    },
  };
  const secondCompletion: ThreadItem = {
    ...firstCompletion,
    id: "second-completion",
  };
  const liveThread: Thread = {
    ...makeThread(),
    turns: [
      {
        id: "turn-live",
        items: [firstCompletion, secondCompletion],
        itemsView: "full",
        status: "completed",
        error: null,
        startedAt: 1,
        completedAt: 2,
        durationMs: 1_000,
      },
    ],
  };

  const updated = upsertThreadMetadataPreservingTurns([liveThread], {
    ...liveThread,
    preview: "metadata refreshed",
    updatedAt: 10,
    turns: [],
  });

  assert.equal(updated[0]?.preview, "metadata refreshed");
  assert.deepEqual(
    updated[0]?.turns[0]?.items.map((item) => item.id),
    ["first-completion", "second-completion"],
  );
});

test("direct collab status completion notifications create completed synthetic turns", () => {
  const item: ThreadItem = {
    type: "collabAgentStatusUpdate",
    id: "subagent-complete",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    status: {
      path: "/root/worker",
      status: "completed",
      message: "done",
    },
  };

  assert.equal(
    getThreadItemNotificationSyntheticTurnStatus("item/started", item),
    undefined,
  );
  assert.equal(
    getThreadItemNotificationSyntheticTurnStatus("item/completed", item),
    "completed",
  );

  const updatedRoot = updateThreadItem(makeThread(), "turn-root", item, {
    completedAtMs: 3_000,
    syntheticTurnStatus: getThreadItemNotificationSyntheticTurnStatus(
      "item/completed",
      item,
    ),
  });

  assert.equal(
    isThreadThinking(updatedRoot, {
      isLoadingThread: false,
      isSending: false,
    }),
    false,
  );
  assert.deepEqual(
    updatedRoot.turns.map((turn) => ({
      id: turn.id,
      status: turn.status,
      startedAt: turn.startedAt,
      completedAt: turn.completedAt,
      durationMs: turn.durationMs,
    })),
    [
      {
        id: "turn-root",
        status: "completed",
        startedAt: 3,
        completedAt: 3,
        durationMs: null,
      },
    ],
  );
});

test("terminal child status does not hide restored conversation history or keep loaded thread active", () => {
  const restoredThread: Thread = {
    ...makeThread(),
    status: {
      type: "idle",
    },
    turns: [
      {
        id: "turn-user",
        items: [
          {
            type: "userMessage",
            id: "user-1",
            content: [{ type: "text", text: "please fix this" }],
          },
          {
            type: "agentMessage",
            id: "agent-1",
            text: "I fixed it.",
            phase: null,
            memoryCitation: null,
          },
        ],
        itemsView: "full",
        status: "completed",
        error: null,
        startedAt: 1,
        completedAt: 2,
        durationMs: 1_000,
      },
    ],
  };

  const childStatus: ThreadItem = {
    type: "collabAgentStatusUpdate",
    id: "child-shutdown",
    senderThreadId: "thread-child",
    senderPath: "/root/worker/tester",
    recipientThreadId: "thread-1",
    recipientPath: "/root/worker",
    status: {
      path: "/root/worker/tester",
      status: "shutdown",
      message: "completed",
    },
  };
  const updated = updateThreadItem(restoredThread, "turn-child", childStatus, {
    completedAtMs: 3_000,
    syntheticTurnStatus: "completed",
  });

  assert.deepEqual(
    buildConversationEntries(updated).map((entry) => entry.text),
    [
      "please fix this",
      "I fixed it.",
      "/root/worker/tester • shutdown • completed",
    ],
  );
  assert.equal(getThreadPresenceLabel(updated), "Idle");
  assert.equal(threadDisplayStatusClass(updated), "todo");
});

test("initialized live child completion preserves existing assistant messages", () => {
  const initializedThread: Thread = {
    ...makeThread(),
    turns: [
      {
        id: "turn-parent",
        items: [
          {
            type: "userMessage",
            id: "user-1",
            content: [{ type: "text", text: "run worker" }],
          },
          {
            type: "agentMessage",
            id: "agent-1",
            text: "Worker is running.",
            phase: null,
            memoryCitation: null,
          },
        ],
        itemsView: "full",
        status: "completed",
        error: null,
        startedAt: 1,
        completedAt: 2,
        durationMs: 1_000,
      },
    ],
  };
  const childStatus: ThreadItem = {
    type: "collabAgentStatusUpdate",
    id: "child-complete",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    status: {
      path: "/root/worker",
      status: "completed",
      message: "done",
    },
  };

  const [updated] = applyInitializedThreadUpdate(
    [initializedThread],
    new Set(["thread-1"]),
    "thread-1",
    (thread) =>
      updateThreadItem(thread, "turn-child", childStatus, {
        completedAtMs: 3_000,
        syntheticTurnStatus: "completed",
      }),
  );

  assert.ok(updated);
  assert.deepEqual(
    buildConversationEntries(updated).map((entry) => entry.text),
    [
      "run worker",
      "Worker is running.",
      "/root/worker • completed • done",
    ],
  );
});

test("uninitialized live child completion does not create display turn cache", () => {
  const thread = makeThread();
  const childStatus: ThreadItem = {
    type: "collabAgentStatusUpdate",
    id: "child-complete",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    status: {
      path: "/root/worker",
      status: "completed",
      message: "done",
    },
  };

  const updatedThreads = applyInitializedThreadUpdate(
    [thread],
    new Set(),
    "thread-1",
    (current) =>
      updateThreadItem(current, "turn-child", childStatus, {
        completedAtMs: 3_000,
        syntheticTurnStatus: "completed",
      }),
  );

  assert.deepEqual(updatedThreads, [thread]);
  assert.deepEqual(
    buildConversationEntries(updatedThreads[0]!).map((entry) => entry.text),
    [],
  );
});

test("collab status item notifications stay on the notification thread unless sent by that thread", () => {
  const item: ThreadItem = {
    type: "collabAgentStatusUpdate",
    id: "subagent-complete",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    status: {
      path: "/root/worker",
      status: "completed",
      message: "done",
    },
  };

  assert.deepEqual(getThreadItemNotificationTargetThreadIds("thread-1", item), [
    "thread-1",
  ]);
});

test("legacy child completion message notifications target the recipient thread", () => {
  const item: ThreadItem = {
    type: "collabAgentMessage",
    id: "child-completion",
    operation: "childCompletion",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    otherRecipientPaths: [],
    content: "done",
    triggerTurn: true,
  };

  assert.deepEqual(
    getThreadItemNotificationTargetThreadIds("thread-child", item),
    ["thread-1"],
  );
});

test("pending thread updates preserve same-content items with different ids", () => {
  const pendingUpdates = new Map<string, Array<(thread: Thread) => Thread>>();
  queuePendingThreadUpdate(pendingUpdates, "thread-1", (thread) =>
    updateThreadItem(thread, "turn-1", {
      type: "collabAgentMessage",
      id: "pending-item",
      operation: "sendMessage",
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
            operation: "sendMessage",
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
      id: "snapshot-item",
      operation: "sendMessage",
      senderThreadId: "thread-child",
      senderPath: "/root/worker",
      recipientThreadId: "thread-1",
      recipientPath: "/root",
      otherRecipientPaths: [],
      content: "same backend message",
      triggerTurn: true,
    },
    {
      type: "collabAgentMessage",
      id: "pending-item",
      operation: "sendMessage",
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

test("pending agent deltas preserve same-content completed assistant blocks with different ids", () => {
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
  assert.deepEqual(updated.turns, [
    ...snapshot.turns,
    {
      id: "pending-turn",
      items: [
        {
          type: "agentMessage",
          id: "pending-item",
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

for (const operation of ["sendMessage", "send_message"]) {
  test(`mergeThreadSnapshot preserves raw ${operation} assistant envelope text`, () => {
    const turn: Turn = {
      id: "turn-1",
      items: [
        {
          type: "agentMessage",
          id: `raw-${operation}`,
          text: JSON.stringify({
            author: "/root/worker",
            recipient: "/root",
            content: "legacy message",
            operation,
          }),
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

    const merged = mergeThreadSnapshot(null, {
      ...makeThread(),
      turns: [turn],
    });

    assert.deepEqual(merged.turns[0]?.items, turn.items);
  });
}

test("pending agent deltas preserve structured process exit marker text with a distinct id", () => {
  const pendingUpdates = new Map<string, Array<(thread: Thread) => Thread>>();
  queuePendingThreadUpdate(pendingUpdates, "thread-1", (thread) =>
    appendAgentDelta(
      thread,
      "pending-turn",
      "pending-item",
      '<event_driven_tool>{"tool":"process_exit_subscribe","title":"Process exited","text":"Session 42 exited with code 0"}</event_driven_tool>',
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
  assert.deepEqual(updated.turns, [
    ...snapshot.turns,
    {
      id: "pending-turn",
      items: [
        {
          type: "agentMessage",
          id: "pending-item",
          text: '<event_driven_tool>{"tool":"process_exit_subscribe","title":"Process exited","text":"Session 42 exited with code 0"}</event_driven_tool>',
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

test("appendAgentDelta preserves split structured process exit marker text", () => {
  const partial = appendAgentDelta(
    makeThread(),
    "turn-1",
    "item-1",
    "<event",
  );
  const middle = appendAgentDelta(
    partial,
    "turn-1",
    "item-1",
    '_driven_tool>{"tool":"process_exit_subscribe",',
  );
  const later = appendAgentDelta(
    middle,
    "turn-1",
    "item-1",
    '"title":"Process exited",',
  );

  const updated = appendAgentDelta(
    later,
    "turn-1",
    "item-1",
    '"text":"Session 42 exited with code 0"}</event_driven_tool>',
  );

  assert.equal(updated.turns[0]?.items[0]?.type, "agentMessage");
  assert.equal(
    updated.turns[0]?.items[0]?.type === "agentMessage"
      ? updated.turns[0].items[0].text
      : "",
    '<event_driven_tool>{"tool":"process_exit_subscribe","title":"Process exited","text":"Session 42 exited with code 0"}</event_driven_tool>',
  );
});

for (const operation of ["sendMessage", "send_message", "childCompletion"]) {
  test(`appendAgentDelta preserves split raw ${operation} assistant envelope`, () => {
    const envelope = JSON.stringify({
      author: "/root/worker",
      recipient: "/root",
      content: "legacy message",
      operation,
    });
    const chunks = [
      envelope.slice(0, 8),
      envelope.slice(8, 34),
      envelope.slice(34, 72),
      envelope.slice(72),
    ];

    let thread = makeThread();
    for (const chunk of chunks) {
      thread = appendAgentDelta(thread, "turn-1", "item-1", chunk);
    }
    assert.equal(thread.turns[0]?.items[0]?.type, "agentMessage");
    assert.equal(
      thread.turns[0]?.items[0]?.type === "agentMessage"
        ? thread.turns[0].items[0].text
        : "",
      envelope,
    );
  });
}

test("appendAgentDelta keeps split ordinary JSON assistant text", () => {
  const partial = appendAgentDelta(
    makeThread(),
    "turn-1",
    "item-1",
    '{"foo":',
  );

  const updated = appendAgentDelta(partial, "turn-1", "item-1", '"bar"}');

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "agentMessage",
      id: "item-1",
      text: '{"foo":"bar"}',
      phase: null,
      memoryCitation: null,
    },
  ]);
});

test("appendAgentDelta releases split nonlegacy author JSON assistant text", () => {
  const partial = appendAgentDelta(
    makeThread(),
    "turn-1",
    "item-1",
    '{"author":',
  );

  const updated = appendAgentDelta(partial, "turn-1", "item-1", '"assistant"}');

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "agentMessage",
      id: "item-1",
      text: '{"author":"assistant"}',
      phase: null,
      memoryCitation: null,
    },
  ]);
});

test("updateThreadItem clears suppressed legacy stream buffers", () => {
  const partial = appendAgentDelta(
    makeThread(),
    "turn-1",
    "item-1",
    '{"author":',
  );
  const completed = updateThreadItem(
    partial,
    "turn-1",
    {
      type: "agentMessage",
      id: "item-1",
      text: "visible",
      phase: null,
      memoryCitation: null,
    },
    { completedAtMs: 1_000 },
  );

  const updated = appendAgentDelta(completed, "turn-1", "item-1", " text");

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "agentMessage",
      id: "item-1",
      text: "visible text",
      phase: null,
      memoryCitation: null,
      completedAtMs: 1_000,
    },
  ]);
});

test("updateThreadTurn clears suppressed legacy stream buffers", () => {
  const partial = appendAgentDelta(
    makeThread(),
    "turn-1",
    "item-1",
    "<event",
  );
  const completed = updateThreadTurn(partial, {
    id: "turn-1",
    items: [
      {
        type: "agentMessage",
        id: "item-1",
        text: "visible",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "full",
    status: "completed",
    error: null,
    startedAt: 1,
    completedAt: 2,
    durationMs: 1000,
  });

  const updated = appendAgentDelta(completed, "turn-1", "item-1", " text");

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "agentMessage",
      id: "item-1",
      text: "visible text",
      phase: null,
      memoryCitation: null,
    },
  ]);
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
      activeFlags: ["waitingOnSubagent"],
    },
  } satisfies Thread;

  assert.equal(treeThreadStatusClass(makeTreeNode(thread)), "waiting-subagent");
  assert.equal(
    treeThreadStatusLabel("waiting-subagent"),
    "Waiting on subagent",
  );
});

test("treeThreadStatusClass ignores item-derived subagent waits", () => {
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
          {
            type: "collabAgentToolCall" as const,
            id: "item-2",
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

  assert.equal(treeThreadStatusClass(makeTreeNode(thread)), "doing");
});

test("treeThreadStatusClass shows event tool waiting separately", () => {
  const thread = {
    ...makeThread(),
    status: {
      type: "active" as const,
      activeFlags: ["waitingOnEventTool"],
    },
  } satisfies Thread;

  assert.equal(
    treeThreadStatusClass(makeTreeNode(thread)),
    "waiting-eventtool",
  );
});

test("treeThreadStatusClass prioritizes backend event tool flags over subagent flags", () => {
  const thread = {
    ...makeThread(),
    status: {
      type: "active" as const,
      activeFlags: ["waitingOnSubagent", "waitingOnEventTool"],
    },
  } satisfies Thread;

  assert.equal(
    treeThreadStatusClass(makeTreeNode(thread)),
    "waiting-eventtool",
  );
});

test("treeThreadStatusClass ignores process exit restore failures when backend status is idle", () => {
  const thread = {
    ...makeThread(),
    status: {
      type: "idle" as const,
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

  assert.equal(treeThreadStatusClass(makeTreeNode(thread)), "todo");
});

test("treeThreadStatusClass ignores event tool subscriptions after unsubscribe when backend status is idle", () => {
  const thread = {
    ...makeThread(),
    status: {
      type: "idle" as const,
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

  assert.equal(treeThreadStatusClass(makeTreeNode(thread)), "todo");
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
      activeFlags: ["waitingOnEventTool"],
    },
  } satisfies Thread;
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

test("treeThreadStatusClass rolls descendant event tool waits into parent subagent wait", () => {
  const parent = {
    ...makeThread(),
    id: "parent",
  };
  const child = {
    ...makeThread(),
    id: "child",
    status: {
      type: "active" as const,
      activeFlags: ["waitingOnEventTool"],
    },
  } satisfies Thread;

  assert.equal(
    treeThreadStatusClass(makeTreeNode(parent, [makeTreeNode(child)])),
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
      activeFlags: ["waitingOnEventTool"],
    }),
    "Waiting on Event Tool",
  );
  assert.equal(
    getPresenceLabel({
      type: "active",
      activeFlags: ["waitingOnSubagent"],
    }),
    "Waiting on Subagent",
  );
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
  assert.equal(
    getPresenceLabel({
      type: "active",
      activeFlags: ["running"],
    }),
    "Running",
  );
});

test("isThreadThinking stays false while a turn only injects init context", () => {
  const thread = {
    ...makeThread(),
    status: {
      type: "active" as const,
      activeFlags: ["waitingOnUserInput"],
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

test("isThreadThinking ignores item-derived running state when backend status is idle", () => {
  const thread = {
    ...makeThread(),
    status: {
      type: "idle" as const,
    },
    turns: [
      {
        id: "turn-1",
        items: [
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
    false,
  );
});

test("isThreadThinking follows backend running active flag", () => {
  const thread = {
    ...makeThread(),
    status: {
      type: "active" as const,
      activeFlags: ["running"],
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
