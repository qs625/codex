import test from "node:test";
import assert from "node:assert/strict";

import {
  appendAgentDelta,
  buildCurrentThreadTodoItems,
  formatUpdatedLabel,
  getPresenceLabel,
  getParentThreadId,
  getThreadPath,
  isThreadThinking,
  mergeThreadSnapshot,
  threadStatusClass,
  updateThreadTurn,
} from "./thread";
import type { Thread } from "../types";

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

test("threadStatusClass treats active thread status as doing", () => {
  assert.equal(
    threadStatusClass({
      type: "active",
      activeFlags: [],
    }),
    "doing",
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
