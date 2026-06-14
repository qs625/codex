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

test("keeps consecutive ordinary tools grouped in one visible cell", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "commandExecution",
        id: "cmd-1",
        command: "pwd",
        cwd: "/tmp/project",
        status: "completed",
        aggregatedOutput: null,
        exitCode: 0,
        durationMs: 10,
      },
      {
        type: "commandExecution",
        id: "cmd-2",
        command: "ls",
        cwd: "/tmp/project",
        status: "completed",
        aggregatedOutput: null,
        exitCode: 0,
        durationMs: 10,
      },
    ]),
  );

  const cells = buildConversationCells(entries);

  assert.deepEqual(
    cells.map((cell) => ({
      id: cell.id,
      kind: cell.kind,
      entries: cell.entries.map((entry) => entry.toolName),
    })),
    [
      {
        id: "cmd-1",
        kind: "tool",
        entries: ["pwd", "ls"],
      },
    ],
  );
});

test("includes command session parameters in command details", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "commandExecution",
        id: "cmd-1",
        command: "cargo test",
        cwd: "/tmp/project",
        status: "running",
        initialWaitMs: 3000,
        notifyOn: "output",
        aggregatedOutput: null,
        exitCode: null,
        durationMs: null,
      },
    ]),
  );

  assert.equal(entries[0]?.toolDetails?.includes("Initial Wait\n3000 ms"), true);
  assert.equal(entries[0]?.toolDetails?.includes("Notify On\noutput"), true);
});

test("renders command notifications as standalone event entries", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "commandExecutionNotification",
        id: "cmd-1:notification:output:1",
        commandItemId: "cmd-1",
        kind: "output",
        message: "Command output notification received.",
        output: "changed",
        exitCode: null,
        createdAtMs: 1,
      },
      {
        type: "commandExecutionNotification",
        id: "cmd-1:notification:exit",
        commandItemId: "cmd-1",
        kind: "exit",
        message: "Command exit notification received.",
        output: null,
        exitCode: 0,
        createdAtMs: 2,
      },
    ]),
  );

  assert.deepEqual(
    entries.map((entry) => [entry.kind, entry.text]),
    [
      [
        "event",
        "Command output notification received for cmd-1: changed",
      ],
      ["event", "Command exit notification received for cmd-1: exit 0."],
    ],
  );
});

test("renders command wait and stdin actions as standalone event entries", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "commandWait",
        id: "wait-1",
        commandId: "7",
        status: "completed",
        notification: "exit",
        exitCode: 0,
        wallTimeSeconds: 1.25,
        createdAtMs: 2_000,
      },
      {
        type: "commandWriteStdin",
        id: "stdin-1",
        commandId: "7",
        bytesWritten: 4,
        containsNewline: true,
        createdAtMs: 3_000,
      },
    ]),
  );

  assert.deepEqual(
    entries.map((entry) => [entry.kind, entry.text, entry.timestamp]),
    [
      [
        "event",
        "Waited for command 7 after exit notification: completed, exit 0 in 1.250s.",
        formatClockTime(2),
      ],
      [
        "event",
        "Wrote 4 bytes to command 7 including newline.",
        formatClockTime(3),
      ],
    ],
  );

  const cells = buildConversationCells(entries);
  assert.deepEqual(
    cells.map((cell) => ({
      id: cell.id,
      kind: cell.kind,
      entries: cell.entries.map((entry) => entry.id),
    })),
    [
      {
        id: "wait-1",
        kind: "event",
        entries: ["wait-1"],
      },
      {
        id: "stdin-1",
        kind: "event",
        entries: ["stdin-1"],
      },
    ],
  );
});

test("groups consecutive agent messages without crossing semantic boundaries", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "agentMessage",
        id: "agent-1",
        text: "first agent message",
        phase: null,
        memoryCitation: null,
      },
      {
        type: "agentMessage",
        id: "agent-2",
        text: "second agent message",
        phase: null,
        memoryCitation: null,
      },
      {
        type: "userMessage",
        id: "user-1",
        content: [{ type: "text", text: "user boundary" }],
      },
      {
        type: "agentMessage",
        id: "agent-3",
        text: "agent after user",
        phase: null,
        memoryCitation: null,
      },
      {
        type: "commandExecution",
        id: "cmd-1",
        command: "pwd",
        cwd: "/tmp/project",
        status: "completed",
        aggregatedOutput: null,
        exitCode: 0,
        durationMs: 10,
      },
      {
        type: "agentMessage",
        id: "agent-4",
        text: "agent after tool",
        phase: null,
        memoryCitation: null,
      },
      {
        type: "collabAgentMessage",
        id: "child-1",
        operation: "childCompletion",
        senderThreadId: "thread-2",
        senderPath: "/root/worker",
        recipientThreadId: "thread-1",
        recipientPath: "/root",
        otherRecipientPaths: [],
        content: "child done",
        triggerTurn: true,
      },
      {
        type: "agentMessage",
        id: "agent-5",
        text: "agent after child completion",
        phase: null,
        memoryCitation: null,
      },
    ]),
  );

  const cells = buildConversationCells(entries);

  assert.deepEqual(
    cells.map((cell) => ({
      id: cell.id,
      kind: cell.kind,
      entries: cell.entries.map((entry) => entry.id),
    })),
    [
      {
        id: "agent-1",
        kind: "message",
        entries: ["agent-1", "agent-2"],
      },
      {
        id: "user-1",
        kind: "message",
        entries: ["user-1"],
      },
      {
        id: "agent-3",
        kind: "message",
        entries: ["agent-3"],
      },
      {
        id: "cmd-1",
        kind: "tool",
        entries: ["cmd-1"],
      },
      {
        id: "agent-4",
        kind: "message",
        entries: ["agent-4"],
      },
      {
        id: "child-1",
        kind: "tool",
        entries: ["child-1"],
      },
      {
        id: "agent-5",
        kind: "message",
        entries: ["agent-5"],
      },
    ],
  );
});

test("keeps ordinary multi-agent tool entries grouped in one visible cell", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "collabAgentToolCall",
        id: "spawn-1",
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
      {
        type: "collabAgentToolCall",
        id: "send-1",
        tool: "sendInput",
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

  const cells = buildConversationCells(entries);

  assert.deepEqual(
    cells.map((cell) => ({
      id: cell.id,
      kind: cell.kind,
      entries: cell.entries.map((entry) => [
        entry.toolCategory,
        entry.toolName,
      ]),
    })),
    [
      {
        id: "spawn-1",
        kind: "tool",
        entries: [
          ["multiAgent", "spawn agent"],
          ["multiAgent", "send message"],
        ],
      },
    ],
  );
});

test("keeps typed sendMessage collab messages visible", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "collabAgentMessage",
        id: "send-1",
        operation: "sendMessage",
        senderThreadId: "thread-2",
        senderPath: "/root/worker",
        recipientThreadId: "thread-1",
        recipientPath: "/root",
        otherRecipientPaths: [],
        content: "same backend message",
        triggerTurn: true,
      },
    ]),
  );

  assert.deepEqual(
    entries.map((entry) => [entry.toolCategory, entry.toolName, entry.text]),
    [
      [
        "multiAgent",
        "received from /root/worker",
        "Received message from /root/worker.",
      ],
    ],
  );
});

test("keeps child completions and subagent notifications as visible cells", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "collabAgentMessage",
        id: "child-1",
        operation: "childCompletion",
        senderThreadId: "thread-2",
        senderPath: "/root/worker",
        recipientThreadId: "thread-1",
        recipientPath: "/root",
        otherRecipientPaths: [],
        content: "done",
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

  const cells = buildConversationCells(entries);

  assert.deepEqual(
    cells.map((cell) => ({
      id: cell.id,
      kind: cell.kind,
      entries: cell.entries.map((entry) => [
        entry.toolCategory,
        entry.toolName,
      ]),
    })),
    [
      {
        id: "child-1",
        kind: "tool",
        entries: [["childCompletion", "/root/worker subagent completion"]],
      },
      {
        id: "status-1",
        kind: "tool",
        entries: [["subagentNotification", "/root/worker subagent completion"]],
      },
    ],
  );
});

test("keeps child completion order inside the parent turn", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "userMessage",
        id: "user-1",
        content: [{ type: "text", text: "wait for worker" }],
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
          message: "worker finished",
        },
      },
      {
        type: "agentMessage",
        id: "agent-1",
        text: "The worker is done.",
        phase: null,
        memoryCitation: null,
      },
    ]),
  );

  assert.deepEqual(
    entries.map((entry) => [entry.id, entry.kind, entry.text]),
    [
      ["user-1", "message", "wait for worker"],
      ["status-1", "tool", "/root/worker • completed • worker finished"],
      ["agent-1", "message", "The worker is done."],
    ],
  );
});

test("uses short previews for long child completion details", () => {
  const longMessage = [
    "first line",
    "second line",
    "third line",
    "fourth line",
    "fifth line",
    "sixth line",
    "seventh line",
    "eighth line",
    "ninth line",
    "tenth line",
    "eleventh line",
    "twelfth line",
    "thirteenth line",
    "fourteenth line",
  ].join("\n");
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "collabAgentMessage",
        id: "child-1",
        operation: "childCompletion",
        senderThreadId: "thread-2",
        senderPath: "/root/worker",
        recipientThreadId: "thread-1",
        recipientPath: "/root",
        otherRecipientPaths: [],
        content: longMessage,
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
          message: longMessage,
        },
      },
    ]),
  );

  assert.equal(entries[0]?.text.includes("\n"), false);
  assert.equal(entries[0]?.text.endsWith("…"), true);
  assert.equal(entries[0]?.text.includes("fourteenth line"), false);
  assert.equal(entries[0]?.toolDetails?.includes(longMessage), true);
  assert.equal(entries[1]?.text.includes("\n"), false);
  assert.equal(entries[1]?.text.endsWith("…"), true);
  assert.equal(entries[1]?.text.includes("fourteenth line"), false);
  assert.equal(entries[1]?.toolDetails?.includes(longMessage), true);
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

test("previews captured process exit output without putting it in the collapsed event text", () => {
  const capturedOutput = Array.from(
    { length: 16 },
    (_, index) => `captured line ${index + 1}`,
  ).join("\n");
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "eventDrivenTool",
        id: "event-1",
        tool: "process_exit_subscribe",
        title: "Process exit subscription (build)",
        text: `Session 42 exited with code 0\nCaptured output:\n${capturedOutput}`,
      },
    ]),
  );

  assert.equal(entries.length, 1);
  assert.equal(entries[0]?.text, "Session 42 exited with code 0");
  assert.equal(entries[0]?.text.includes("captured line 1"), false);
  assert.equal(
    entries[0]?.toolDetails,
    [
      "Tool\nprocess_exit_subscribe",
      "Event\nProcess exit subscription (build)",
      "Details\nSession 42 exited with code 0",
      [
        "Captured output",
        "captured line 1",
        "captured line 2",
        "captured line 3",
        "captured line 4",
        "captured line 5",
        "captured line 6",
        "captured line 7",
        "captured line 8",
        "captured line 9",
        "captured line 10",
        "captured line 11",
        "captured line 12",
        "… omitted additional captured output",
      ].join("\n"),
    ].join("\n\n"),
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
    [
      [
        "wait for agent",
        "wait on /root/worker for 30s",
        "Tool\nwait_agent\n\nSender\n/root\n\nReceivers\n/root/worker\n\nTimeout\n30s",
      ],
    ],
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

test("does not render child completion envelopes in event-driven tools as multi-agent tools", () => {
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

  assert.equal(entries.length, 1);
  assert.equal(entries[0]?.kind, "tool");
  assert.equal(entries[0]?.toolCategory, "eventDrivenEvent");
  assert.notEqual(entries[0]?.toolName, "/root/worker subagent completion");
  assert.ok(!entries[0]?.text.startsWith("Received child completion"));
});

test("does not render child completion envelopes in agent messages as multi-agent tools", () => {
  const text = JSON.stringify({
    author: "/root/worker",
    recipient: "/root",
    other_recipients: [],
    content: "done",
    operation: "childCompletion",
    trigger_turn: true,
    sender_thread_id: "thread-2",
    recipient_thread_id: "thread-1",
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
    entries.map((entry) => [entry.kind, entry.text, entry.toolName]),
    [["message", text, undefined]],
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

test("renders context compaction as a compact boundary entry", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "contextCompaction",
        id: "compact-1",
        replacementHistory: null,
      },
    ]),
  );

  assert.equal(entries.length, 1);
  assert.deepEqual(
    {
      id: entries[0]?.id,
      kind: entries[0]?.kind,
      text: entries[0]?.text,
      replacementHistoryStatus: entries[0]?.replacementHistoryStatus,
      replacementHistoryEntries: entries[0]?.replacementHistoryEntries,
    },
    {
      id: "compact-1",
      kind: "compact",
      text: "Previous conversation was archived; compacted model context continues below.",
      replacementHistoryStatus: "missing",
      replacementHistoryEntries: null,
    },
  );
});

test("renders compact replacement history using readable entries", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "contextCompaction",
        id: "compact-1",
        replacementHistory: [
          {
            type: "message",
            role: "user",
            content: [{ type: "input_text", text: "recent request" }],
          },
          {
            type: "function_call",
            namespace: "functions",
            name: "exec_command",
            arguments: '{"cmd":"pwd"}',
            call_id: "call-1",
          },
          {
            type: "new_special_item",
            payload: { ok: true },
          },
        ],
      },
    ]),
  );

  const replacementEntries = entries[0]?.replacementHistoryEntries ?? [];

  assert.equal(entries[0]?.replacementHistoryStatus, "available");
  assert.equal(entries[0]?.replacementHistoryCount, 3);
  assert.deepEqual(
    replacementEntries.map((entry) => ({
      kind: entry.kind,
      author: entry.author,
      text: entry.text,
      toolName: entry.toolName,
      toolDetails: entry.toolDetails,
    })),
    [
      {
        kind: "message",
        author: "You",
        text: "recent request",
        toolName: undefined,
        toolDetails: undefined,
      },
      {
        kind: "tool",
        author: "root",
        text: "Function call call-1",
        toolName: "functions/exec_command",
        toolDetails:
          'Type\nfunction_call\n\nCall ID\ncall-1\n\nName\nexec_command\n\nNamespace\nfunctions\n\nArguments\n{\n  "cmd": "pwd"\n}',
      },
      {
        kind: "tool",
        author: "root",
        text: "Unsupported replacement history item. Raw data is preserved below.",
        toolName: "unsupported new special item",
        toolDetails:
          'Raw item\n{\n  "type": "new_special_item",\n  "payload": {\n    "ok": true\n  }\n}',
      },
    ],
  );
});

test("renders common replacement response item variants without dropping them", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "contextCompaction",
        id: "compact-1",
        replacementHistory: [
          { type: "reasoning", summary: [{ text: "reasoned" }] },
          {
            type: "function_call_output",
            call_id: "call-1",
            output: "done",
          },
          {
            type: "custom_tool_call",
            call_id: "custom-1",
            name: "patch",
            input: "edit",
          },
          {
            type: "custom_tool_call_output",
            call_id: "custom-1",
            name: "patch",
            output: "edited",
          },
          {
            type: "local_shell_call",
            call_id: "shell-1",
            status: "completed",
            action: { type: "exec", command: "pwd" },
          },
          {
            type: "tool_search_call",
            call_id: "search-1",
            execution: "search",
            arguments: { q: "docs" },
          },
          {
            type: "tool_search_output",
            call_id: "search-1",
            status: "completed",
            execution: "search",
            tools: [],
          },
          {
            type: "web_search_call",
            action: { type: "search", query: "weather" },
          },
          {
            type: "image_generation_call",
            id: "image-1",
            status: "completed",
            result: "image",
          },
          {
            type: "compaction",
            encrypted_content: "encrypted",
          },
          {
            type: "context_compaction",
          },
        ],
      },
    ]),
  );

  const replacementEntries = entries[0]?.replacementHistoryEntries ?? [];

  assert.deepEqual(
    replacementEntries.map((entry) => ({
      kind: entry.kind,
      text: entry.text,
      toolName: entry.toolName,
      isReplacementHistory: entry.isReplacementHistory,
    })),
    [
      {
        kind: "event",
        text: "reasoned",
        toolName: undefined,
        isReplacementHistory: true,
      },
      {
        kind: "tool",
        text: "Function output call-1",
        toolName: "function output",
        isReplacementHistory: true,
      },
      {
        kind: "tool",
        text: "Custom tool call custom-1",
        toolName: "patch",
        isReplacementHistory: true,
      },
      {
        kind: "tool",
        text: "Custom tool output custom-1",
        toolName: "patch",
        isReplacementHistory: true,
      },
      {
        kind: "tool",
        text: "Local shell call shell-1",
        toolName: "local shell",
        isReplacementHistory: true,
      },
      {
        kind: "tool",
        text: "Replacement history 6: tool search call",
        toolName: "tool search call",
        isReplacementHistory: true,
      },
      {
        kind: "tool",
        text: "Replacement history 7: tool search output",
        toolName: "tool search output",
        isReplacementHistory: true,
      },
      {
        kind: "tool",
        text: "Web search for weather",
        toolName: "web search call",
        isReplacementHistory: true,
      },
      {
        kind: "tool",
        text: "Replacement history 9: image generation call",
        toolName: "image generation call",
        isReplacementHistory: true,
      },
      {
        kind: "event",
        text: "Compaction summary item included in compacted model context.",
        toolName: undefined,
        isReplacementHistory: true,
      },
      {
        kind: "compact",
        text: "Nested context compaction item included in compacted model context.",
        toolName: undefined,
        isReplacementHistory: true,
      },
    ],
  );
});

test("places replacement history into the main conversation after archived history", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "userMessage",
        id: "old-user",
        content: [{ type: "text", text: "old request" }],
      },
      {
        type: "contextCompaction",
        id: "compact-1",
        replacementHistory: [
          {
            type: "message",
            role: "user",
            content: [{ type: "input_text", text: "recent request" }],
          },
          {
            type: "function_call",
            name: "shell",
            arguments: '{"cmd":"pwd"}',
            call_id: "call-1",
          },
        ],
      },
      {
        type: "agentMessage",
        id: "after-compact",
        text: "continued",
        phase: null,
        memoryCitation: null,
      },
    ]),
  );

  const cells = buildConversationCells(entries);

  assert.deepEqual(
    cells.map((cell) => ({
      id: cell.id,
      kind: cell.kind,
      entries: cell.entries.map((entry) => ({
        id: entry.id,
        kind: entry.kind,
        text: entry.text,
        archivedEntryCount: entry.archivedEntryCount,
        isReplacementHistory: entry.isReplacementHistory,
      })),
    })),
    [
      {
        id: "compact-1:archive",
        kind: "archive",
        entries: [
          {
            id: "compact-1:archive",
            kind: "archive",
            text: "Previous conversation is no longer the active model context.",
            archivedEntryCount: 1,
            isReplacementHistory: undefined,
          },
        ],
      },
      {
        id: "compact-1",
        kind: "compact",
        entries: [
          {
            id: "compact-1",
            kind: "compact",
            text: "Previous conversation was archived; compacted model context continues below.",
            archivedEntryCount: undefined,
            isReplacementHistory: undefined,
          },
        ],
      },
      {
        id: "compact-1:replacement:0",
        kind: "message",
        entries: [
          {
            id: "compact-1:replacement:0",
            kind: "message",
            text: "recent request",
            archivedEntryCount: undefined,
            isReplacementHistory: true,
          },
        ],
      },
      {
        id: "compact-1:replacement:1",
        kind: "tool",
        entries: [
          {
            id: "compact-1:replacement:1",
            kind: "tool",
            text: "Function call call-1",
            archivedEntryCount: undefined,
            isReplacementHistory: true,
          },
        ],
      },
      {
        id: "after-compact",
        kind: "message",
        entries: [
          {
            id: "after-compact",
            kind: "message",
            text: "continued",
            archivedEntryCount: undefined,
            isReplacementHistory: undefined,
          },
        ],
      },
    ],
  );
});

test("preserves compact replacement raw child completion when live status update exists", () => {
  const childCompletionEnvelope = JSON.stringify({
    author: "/root/worker",
    recipient: "/root",
    other_recipients: [],
    content: "done",
    operation: "childCompletion",
    trigger_turn: true,
    sender_thread_id: "thread-child",
    recipient_thread_id: "thread-1",
  });
  const state = buildConversationState(
    makeThread([
      {
        type: "userMessage",
        id: "old-user",
        content: [{ type: "text", text: "old request" }],
      },
      {
        type: "contextCompaction",
        id: "compact-1",
        replacementHistory: [
          {
            type: "message",
            role: "assistant",
            content: [{ type: "output_text", text: childCompletionEnvelope }],
          },
        ],
      },
      {
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
      },
    ]),
  );

  assert.deepEqual(
    state.cells.map((cell) => cell.kind),
    ["archive", "compact", "message", "tool"],
  );
  assert.equal(state.cells[0]?.entries[0]?.archivedEntryCount, 1);
  assert.deepEqual(
    state.cells
      .flatMap((cell) => cell.entries)
      .filter((entry) => entry.kind === "message" && entry.role === "agent")
      .map((entry) => entry.text),
    [childCompletionEnvelope],
  );
  assert.deepEqual(
    state.cells
      .flatMap((cell) => cell.entries)
      .filter(
        (entry) =>
          entry.toolCategory === "childCompletion" ||
          entry.toolCategory === "subagentNotification",
      )
      .map((entry) => [entry.toolName, entry.text]),
    [
      [
        "/root/worker subagent completion",
        "/root/worker • completed • done",
      ],
    ],
  );
});

test("does not render compact replacement raw child completion before typed status", () => {
  const childCompletionEnvelope = JSON.stringify({
    author: "/root/worker",
    recipient: "/root",
    other_recipients: [],
    content: "first done",
    operation: "childCompletion",
    trigger_turn: true,
    sender_thread_id: "thread-child",
    recipient_thread_id: "thread-1",
  });
  const state = buildConversationState(
    makeThread([
      {
        type: "userMessage",
        id: "old-user",
        content: [{ type: "text", text: "old request" }],
      },
      {
        type: "contextCompaction",
        id: "compact-1",
        replacementHistory: [
          {
            type: "message",
            role: "assistant",
            content: [{ type: "output_text", text: childCompletionEnvelope }],
          },
        ],
      },
      {
        type: "collabAgentStatusUpdate",
        id: "later-completion",
        senderThreadId: "thread-child",
        senderPath: "/root/worker",
        recipientThreadId: "thread-1",
        recipientPath: "/root",
        status: {
          path: "/root/worker",
          status: "completed",
          message: "second done",
        },
      },
    ]),
  );

  assert.deepEqual(
    state.cells
      .flatMap((cell) => cell.entries)
      .filter(
        (entry) =>
          entry.toolCategory === "childCompletion" ||
          entry.toolCategory === "subagentNotification",
      )
      .map((entry) => [entry.toolName, entry.text]),
    [
      ["/root/worker subagent completion", "/root/worker • completed • second done"],
    ],
  );
});

test("archives earlier compact boundaries when a later compact replaces context again", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "userMessage",
        id: "old-user",
        content: [{ type: "text", text: "old request" }],
      },
      {
        type: "contextCompaction",
        id: "compact-1",
        replacementHistory: [
          {
            type: "message",
            role: "user",
            content: [{ type: "input_text", text: "first replacement" }],
          },
        ],
      },
      {
        type: "agentMessage",
        id: "after-compact-1",
        text: "between compacts",
        phase: null,
        memoryCitation: null,
      },
      {
        type: "contextCompaction",
        id: "compact-2",
        replacementHistory: [
          {
            type: "message",
            role: "user",
            content: [{ type: "input_text", text: "second replacement" }],
          },
        ],
      },
    ]),
  );

  const cells = buildConversationCells(entries);
  const archiveEntry = cells[0]?.entries[0];

  assert.deepEqual(
    cells.map((cell) => ({
      id: cell.id,
      kind: cell.kind,
      text: cell.entries[0]?.text,
    })),
    [
      {
        id: "compact-2:archive",
        kind: "archive",
        text: "Previous conversation is no longer the active model context.",
      },
      {
        id: "compact-2",
        kind: "compact",
        text: "Previous conversation was archived; compacted model context continues below.",
      },
      {
        id: "compact-2:replacement:0",
        kind: "message",
        text: "second replacement",
      },
    ],
  );
  assert.equal(archiveEntry?.archivedEntryCount, 4);
  assert.deepEqual(
    archiveEntry?.archivedCells?.map((cell) => [cell.id, cell.kind]),
    [
      ["compact-1:archive", "archive"],
      ["compact-1", "compact"],
      ["compact-1:replacement:0", "message"],
      ["after-compact-1", "message"],
    ],
  );
});

test("distinguishes empty compact replacement history from missing history", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "contextCompaction",
        id: "compact-1",
        replacementHistory: [],
      },
    ]),
  );

  assert.deepEqual(
    {
      status: entries[0]?.replacementHistoryStatus,
      count: entries[0]?.replacementHistoryCount,
      replacementEntries: entries[0]?.replacementHistoryEntries,
    },
    {
      status: "empty",
      count: 0,
      replacementEntries: [],
    },
  );
});

test("builds visible entries for empty reasoning and unsupported typed items", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "reasoning",
        id: "reasoning-empty",
        summary: [],
        content: [],
      },
      {
        type: "builtinToolCall",
        id: "builtin-unsupported",
        tool: "todo_write",
        arguments: { items: [] },
        status: "completed",
        output: null,
      },
    ]),
  );

  assert.deepEqual(
    entries.map((entry) => ({
      id: entry.id,
      kind: entry.kind,
      text: entry.text,
    })),
    [
      {
        id: "reasoning-empty",
        kind: "event",
        text: "Reasoning item received.",
      },
      {
        id: "builtin-unsupported",
        kind: "event",
        text: "Unsupported thread item: builtinToolCall",
      },
    ],
  );
});

test("preserves marker-like agent messages as visible message entries", () => {
  const markerText =
    '<event_driven_tool>{"tool":"process_exit_subscribe","title":"Process exited","text":"done"}</event_driven_tool>';
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "agentMessage",
        id: "marker-like-agent-message",
        text: markerText,
        phase: null,
        memoryCitation: null,
      },
    ]),
  );

  assert.equal(entries[0]?.id, "marker-like-agent-message");
  assert.equal(entries[0]?.text, markerText);
});

test("keeps repeated standalone notifications as distinct cell entries", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "collabAgentStatusUpdate",
        id: "status-1",
        senderThreadId: "thread-child",
        senderPath: "/root/worker",
        recipientThreadId: "thread-1",
        recipientPath: "/root",
        status: {
          path: "/root/worker",
          status: "completed",
          message: "done",
        },
      },
      {
        type: "collabAgentStatusUpdate",
        id: "status-2",
        senderThreadId: "thread-child",
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
  const cells = buildConversationCells(entries);

  assert.deepEqual(cells.map((cell) => cell.entries.map((entry) => entry.id)), [
    ["status-1"],
    ["status-2"],
  ]);
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
