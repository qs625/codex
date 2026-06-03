import test from "node:test";
import assert from "node:assert/strict";

import {
  buildConversationCells,
  buildConversationEntries,
  buildConversationState,
} from "./conversation";
import { formatClockTime } from "./thread";
import type { Thread } from "../types";

function makeThread(items: Thread["turns"][number]["items"]): Thread {
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
    skills: [],
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

test("separates command, event subscriptions, event notifications, and multi-agent items into different tool cells", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "commandExecution",
        id: "cmd-1",
        command: "ls",
        cwd: "/tmp/project",
        status: "completed",
        aggregatedOutput: null,
        exitCode: 0,
        durationMs: 10,
      },
      {
        type: "eventDrivenToolCall",
        id: "builtin-1",
        tool: "process_exit_subscribe",
        arguments: { session_id: 42, label: "watch build" },
        status: "completed",
        output: { ok: true },
      },
      {
        type: "eventDrivenTool",
        id: "builtin-2",
        tool: "process_exit_subscribe",
        title: "Process exited",
        text: "watch build • session 42 completed",
      },
      {
        type: "collabAgentToolCall",
        id: "agent-1",
        tool: "spawnAgent",
        status: "completed",
        senderThreadId: "thread-1",
        senderPath: "/root",
        receiverThreadIds: ["thread-2"],
        receiverPaths: ["/root/worker"],
        prompt: null,
        model: null,
        reasoningEffort: null,
        agentsStates: {},
      },
    ]),
  );

  assert.deepEqual(
    entries.map((entry) => [entry.toolCategory, entry.toolName, entry.text]),
    [
      ["command", "ls", "tmp/project • exit 0"],
      [
        "eventDrivenSubscription",
        "process_exit_subscribe",
        "process_exit_subscribe • label watch build",
      ],
      [
        "eventDrivenEvent",
        "Process exited",
        "watch build • session 42 completed",
      ],
      ["multiAgent", "spawn agent", "/root -> /root/worker"],
    ],
  );

  const cells = buildConversationCells(entries);
  assert.equal(cells.length, 4);
});

test("uses event-driven item timestamps instead of the parent turn timestamp", () => {
  const thread = makeThread([
    {
      type: "eventDrivenToolCall",
      id: "builtin-1",
      tool: "process_exit_subscribe",
      arguments: { session_id: 42 },
      status: "inProgress",
      output: null,
      startedAtMs: 2_000,
    },
    {
      type: "eventDrivenToolCall",
      id: "builtin-2",
      tool: "process_exit_subscribe",
      arguments: { session_id: 43 },
      status: "completed",
      output: { ok: true },
      startedAtMs: 3_000,
      completedAtMs: 4_000,
    },
  ]);
  thread.turns[0].startedAt = 10;
  thread.turns[0].completedAt = 20;

  const entries = buildConversationEntries(thread);

  assert.deepEqual(
    entries.map((entry) => entry.timestamp),
    [formatClockTime(2), formatClockTime(4)],
  );
});

test("uses meaningful multi-agent titles for received work and child completion", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "collabAgentMessage",
        id: "msg-1",
        operation: "spawnAgent",
        senderThreadId: "thread-1",
        senderPath: "/root",
        recipientThreadId: "thread-2",
        recipientPath: "/root/worker",
        otherRecipientPaths: [],
        content: "do the work",
        triggerTurn: true,
      },
      {
        type: "collabAgentStatusUpdate",
        id: "status-1",
        senderThreadId: "thread-2",
        senderPath: "/root/worker",
        recipientThreadId: "thread-1",
        recipientPath: "/root",
        status: {
          path: "/root/worker",
          status: "completed",
          message: "done",
        },
      },
    ]),
  );

  assert.deepEqual(
    entries.map((entry) => entry.toolName),
    ["received from /root", "/root/worker subagent completion"],
  );
});

test("summarizes wait_agent with receiver path and timeout", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "collabAgentToolCall",
        id: "wait-1",
        tool: "wait",
        status: "completed",
        senderThreadId: "thread-1",
        senderPath: "/root",
        receiverThreadIds: ["thread-2"],
        receiverPaths: ["/root/worker"],
        timeoutMs: 30000,
        prompt: null,
        model: null,
        reasoningEffort: null,
        agentsStates: {},
      },
    ]),
  );

  assert.deepEqual(
    entries.map((entry) => [entry.toolName, entry.text, entry.toolDetails]),
    [[
      "wait for agent",
      "wait on /root/worker for 30s",
      "Tool\nwait_agent\n\nSender\n/root\n\nReceivers\n/root/worker\n\nTimeout\n30s",
    ]],
  );
});

test("tolerates non-string multi-agent history fields without crashing", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "collabAgentMessage",
        id: "msg-1",
        operation: "childCompletion",
        senderThreadId: "thread-2",
        senderPath: { path: "/root/worker" } as never,
        recipientThreadId: "thread-1",
        recipientPath: null as never,
        otherRecipientPaths: [{ path: "/root/other" } as never],
        content: { text: "done" } as never,
        triggerTurn: true,
      },
      {
        type: "collabAgentStatusUpdate",
        id: "status-1",
        senderThreadId: "thread-2",
        senderPath: { path: "/root/worker" } as never,
        recipientThreadId: "thread-1",
        recipientPath: "/root",
        status: {
          path: { path: "/root/worker" } as never,
          status: "completed",
          message: { text: "done" } as never,
        },
      },
    ]),
  );

  assert.deepEqual(
    entries.map((entry) => ({
      toolName: entry.toolName,
      text: entry.text,
      toolDetails: entry.toolDetails,
    })),
    [
      {
        toolName: "unknown subagent completion",
        text: "Received child completion from unknown.",
        toolDetails:
          "Operation\nchildCompletion\n\nFrom\nunknown\n\nTo\nunknown\n\nMessage\n…\n\nTrigger Turn\ntrue\n\nOther Recipients\nunknown",
      },
      {
        toolName: "unknown subagent completion",
        text: "unknown • completed",
        toolDetails: "From\nunknown\n\nTo\n/root\n\nStatus\ncompleted",
      },
    ],
  );
});

test("renders child completion envelopes in event-driven tools as multi-agent tools", () => {
  const content =
    '<subagent_notification>\n{"agent_path":"/root/worker","status":{"completed":"done"}}\n</subagent_notification>';
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "eventDrivenTool",
        id: "event-1",
        tool: "spawn_agent",
        title: "Subagent completed",
        text: JSON.stringify({
          author: "/root/worker",
          recipient: "/root",
          other_recipients: [],
          content,
          operation: "childCompletion",
          trigger_turn: true,
          sender_thread_id: "thread-2",
          recipient_thread_id: "thread-1",
          status: {
            completed: "done",
          },
        }),
      },
    ]),
  );

  assert.deepEqual(
    entries.map((entry) => ({
      kind: entry.kind,
      role: entry.role,
      text: entry.text,
      toolName: entry.toolName,
      toolDetails: entry.toolDetails,
      toolCategory: entry.toolCategory,
    })),
    [
      {
        kind: "tool",
        role: "system",
        text: "Received child completion from /root/worker.",
        toolName: "/root/worker subagent completion",
        toolDetails:
          "Operation\nchildCompletion\n\nFrom\n/root/worker\n\nTo\n/root\n\nMessage\ndone\n\nTrigger Turn\ntrue",
        toolCategory: "multiAgent",
      },
    ],
  );
});

test("renders child completion envelopes in agent messages as multi-agent tools", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "agentMessage",
        id: "msg-1",
        text: JSON.stringify({
          author: "/root/worker",
          recipient: "/root",
          other_recipients: [],
          content: "done",
          operation: "childCompletion",
          trigger_turn: true,
          sender_thread_id: "thread-2",
          recipient_thread_id: "thread-1",
        }),
        phase: null,
        memoryCitation: null,
      },
    ]),
  );

  assert.deepEqual(
    entries.map((entry) => [entry.kind, entry.text, entry.toolName]),
    [
      [
        "tool",
        "Received child completion from /root/worker.",
        "/root/worker subagent completion",
      ],
    ],
  );
});

test("keeps ordinary child completion JSON in agent messages as text", () => {
  const text = JSON.stringify({
    operation: "childCompletion",
    content: "this is ordinary model output",
  });
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "agentMessage",
        id: "msg-1",
        text,
        phase: null,
        memoryCitation: null,
      },
    ]),
  );

  assert.deepEqual(
    entries.map((entry) => ({
      kind: entry.kind,
      role: entry.role,
      text: entry.text,
      toolCategory: entry.toolCategory,
    })),
    [
      {
        kind: "message",
        role: "agent",
        text,
        toolCategory: undefined,
      },
    ],
  );
});

test("keeps ordinary child completion JSON in event-driven tools as event text", () => {
  const text = JSON.stringify({
    operation: "childCompletion",
    content: "not a collab envelope",
  });
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "eventDrivenTool",
        id: "event-1",
        tool: "schedule_subscribe",
        title: "Schedule fired",
        text,
      },
    ]),
  );

  assert.deepEqual(
    entries.map((entry) => ({
      kind: entry.kind,
      text: entry.text,
      toolName: entry.toolName,
      toolCategory: entry.toolCategory,
    })),
    [
      {
        kind: "tool",
        text,
        toolName: "Schedule fired",
        toolCategory: "eventDrivenEvent",
      },
    ],
  );
});

test("renders context compaction as a context tool entry", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "contextCompaction",
        id: "compact-1",
      },
    ]),
  );

  assert.equal(entries.length, 1);
  assert.deepEqual(
    {
      id: entries[0]?.id,
      kind: entries[0]?.kind,
      text: entries[0]?.text,
      toolName: entries[0]?.toolName,
      toolStatus: entries[0]?.toolStatus,
      toolCategory: entries[0]?.toolCategory,
    },
    {
      id: "compact-1",
      kind: "tool",
      text: "Conversation history compacted.",
      toolName: "compact context",
      toolStatus: "completed",
      toolCategory: "context",
    },
  );
});

test("reuses unchanged conversation cells when only the tail item updates", () => {
  const leadingMessage = {
    type: "userMessage" as const,
    id: "msg-1",
    content: [{ type: "text", text: "first" }],
  };
  const trailingMessage = {
    type: "agentMessage" as const,
    id: "msg-2",
    text: "second",
    phase: null,
    memoryCitation: null,
  };
  const initialThread = makeThread([leadingMessage, trailingMessage]);
  const initialState = buildConversationState(initialThread);

  const updatedThread = makeThread([
    leadingMessage,
    {
      ...trailingMessage,
      text: "second updated",
    },
  ]);
  const updatedState = buildConversationState(updatedThread, initialState);

  assert.equal(updatedState.entries[0], initialState.entries[0]);
  assert.notEqual(updatedState.entries[1], initialState.entries[1]);
  assert.equal(updatedState.cells[0], initialState.cells[0]);
  assert.notEqual(updatedState.cells.at(-1), initialState.cells.at(-1));
});
