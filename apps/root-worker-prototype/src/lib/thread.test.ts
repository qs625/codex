import test from "node:test";
import assert from "node:assert/strict";

import {
  getPresenceLabel,
  getParentThreadId,
  getThreadPath,
  isThreadThinking,
  mergeThreadSnapshot,
  threadStatusClass,
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
