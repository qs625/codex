import test from "node:test";
import assert from "node:assert/strict";

import {
  buildConversationEntries,
  buildConversationState,
} from "./conversation";
import { buildConversationCells } from "./conversationCompact";
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

function makeThreadWithTurns(turns: Thread["turns"]): Thread {
  return {
    ...makeThread([]),
    turns,
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

test("builds a conversation event for typed goal updates", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "threadGoalUpdate",
        id: "goal-1",
        action: "created",
        source: "modelTool",
        previousStatus: null,
        goal: {
          threadId: "thread-1",
          objective: "Ship goal ThreadItem display",
          status: "active",
          tokenBudget: 50_000,
          tokensUsed: 1_250,
          timeUsedSeconds: 75,
          createdAt: 1,
          updatedAt: 2,
        },
      },
    ]),
  );

  assert.deepEqual(entries, [
    {
      id: "goal-1",
      kind: "event",
      author: "root",
      role: "system",
      text: "Goal created: Ship goal ThreadItem display",
      timestamp: formatClockTime(1),
      attachments: [],
      toolName: "Goal",
      toolStatus: "active",
      toolDetails:
        "Objective\nShip goal ThreadItem display\n\nStatus\nActive\n\nSource\nModel tool\n\nToken Usage\n1,250 / 50,000\n\nTime Used\n1m 15s",
      toolCategory: "goal",
      turnId: "turn-1",
    },
  ]);

  assert.deepEqual(buildConversationCells(entries), [
    {
      id: "goal-1",
      kind: "event",
      entries,
    },
  ]);
});

test("builds a workflow progress tool entry from typed thread items", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "workflowRunProgress",
        id: "workflow-1",
        event: {
          runId: "wf_123",
          workflowId: "feature-dev",
          status: "running",
          runnerStatus: "runner_active",
          kind: "started",
          message: "workflow runner is executing TypeScript entry",
          updatedAt: 1,
        },
      },
    ]),
  );

  assert.deepEqual(entries, [
    {
      id: "workflow-1",
      kind: "tool",
      author: "root",
      role: "system",
      text: "workflow runner is executing TypeScript entry",
      timestamp: formatClockTime(1),
      attachments: [],
      toolName: "Workflow · feature-dev",
      toolStatus: "running",
      toolDetails:
        "Workflow\nfeature-dev\n\nRun\nwf_123\n\nProgress\nWorkflow started\n\nRunner Status\nrunner_active\n\nMessage\nworkflow runner is executing TypeScript entry\n\nRun Status\nrunning\n\nGraph\nNo graph details in this update.",
      toolCategory: "workflow",
      turnId: "turn-1",
    },
  ]);

  assert.deepEqual(buildConversationCells(entries), [
    {
      id: "workflow-1",
      kind: "tool",
      entries,
    },
  ]);
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
        type: "commandExecution",
        id: "cmd-1",
        command: "npm test",
        cwd: "/tmp/project",
        status: "completed",
        aggregatedOutput: null,
        exitCode: 0,
        durationMs: 10,
      },
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
      ["tool", "tmp/project • exit 0"],
      [
        "event",
        "Command output notification received for npm test: changed",
      ],
      ["event", "Command exit notification received for npm test: exit 0."],
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
        waitTimeoutMs: 300_000,
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
        "Waited for command 7 with timeout 5m after exit notification: completed, exit 0 in 1.25s.",
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

test("renders event command subscriptions and output events", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "eventCommandCall",
        id: "monitor-1",
        subscriptionId: "sub-1",
        command: "cargo test -p app-server",
        cwd: "/tmp/project",
        label: "app-server tests",
        status: "completed",
        output: { ok: true },
      },
      {
        type: "eventCommandEvent",
        id: "monitor-event-1",
        subscriptionId: "sub-1",
        kind: "output",
        label: "app-server tests",
        command: "cargo test -p app-server",
        cwd: "/tmp/project",
        line: "running 1 test",
        sequence: 1,
        exitCode: null,
        signal: null,
        message: null,
        truncated: false,
        createdAt: 4,
      },
    ]),
  );

  assert.deepEqual(
    entries.map((entry) => ({
      id: entry.id,
      kind: entry.kind,
      text: entry.text,
      toolName: entry.toolName,
      toolCategory: entry.toolCategory,
      timestamp: entry.timestamp,
    })),
    [
      {
        id: "monitor-1",
        kind: "tool",
        text: "app-server tests • cargo test -p app-server",
        toolName: "app-server tests",
        toolCategory: "eventDrivenSubscription",
        timestamp: formatClockTime(1),
      },
      {
        id: "monitor-event-1",
        kind: "event",
        text: "app-server tests: running 1 test",
        toolName: undefined,
        toolCategory: undefined,
        timestamp: formatClockTime(4),
      },
    ],
  );
});

test("renders event command exit signals in event summaries", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "eventCommandEvent",
        id: "monitor-event-2",
        subscriptionId: "sub-1",
        kind: "exited",
        label: "app-server tests",
        command: "cargo test -p app-server",
        cwd: "/tmp/project",
        line: null,
        sequence: 2,
        exitCode: null,
        signal: "SIGTERM",
        message: null,
        truncated: false,
        createdAt: 5,
      },
    ]),
  );

  assert.deepEqual(
    entries.map((entry) => ({
      id: entry.id,
      kind: entry.kind,
      text: entry.text,
      timestamp: entry.timestamp,
    })),
    [
      {
        id: "monitor-event-2",
        kind: "event",
        text: "app-server tests: signal SIGTERM.",
        timestamp: formatClockTime(5),
      },
    ],
  );
});

test("renders injected init context as a conversation context entry", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "injectedContext",
        id: "ctx-1",
        title: "Init Context",
        preview: "Workspace • Instructions",
        sections: [
          { label: "Workspace", text: "/tmp/project" },
          { label: "Instructions", text: "全程使用中文" },
        ],
      },
    ]),
  );

  assert.deepEqual(
    entries.map((entry) => ({
      id: entry.id,
      kind: entry.kind,
      role: entry.role,
      text: entry.text,
      toolName: entry.toolName,
      toolStatus: entry.toolStatus,
      toolCategory: entry.toolCategory,
      toolDetails: entry.toolDetails,
    })),
    [
      {
        id: "ctx-1",
        kind: "tool",
        role: "system",
        text: "Workspace • Instructions",
        toolName: "Init Context",
        toolStatus: "completed",
        toolCategory: "context",
        toolDetails: "Workspace\n/tmp/project\n\nInstructions\n全程使用中文",
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
          ["multiAgent", "followup task"],
        ],
      },
    ],
  );
});

test("shows legacy sendMessage collab messages as follow-up messages", () => {
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
        "Received follow-up from /root/worker.",
      ],
    ],
  );
  assert.match(entries[0]?.toolDetails ?? "", /Operation\nfollowupTask/);
});

test("shows typed list_agents collab tool calls", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "collabAgentToolCall",
        id: "list-agents-1",
        tool: "listAgents",
        status: "completed",
        senderThreadId: "thread-1",
        senderPath: "/root",
        receiverThreadIds: ["/root/worker"],
        receiverPaths: ["/root/worker"],
        timeoutMs: null,
        prompt: "/root",
        model: null,
        reasoningEffort: null,
        agentsStates: {
          "/root/worker": {
            path: "/root/worker",
            status: "completed",
            message: "done",
          },
        },
      },
    ]),
  );

  assert.deepEqual(
    entries.map((entry) => [entry.toolCategory, entry.toolName, entry.text]),
    [["multiAgent", "list agents", "listed 1 agents"]],
  );
  assert.match(entries[0]?.toolDetails ?? "", /Tool\nlist_agents/);
  assert.match(entries[0]?.toolDetails ?? "", /Agent States\n\/root\/worker • completed • done/);
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

test("does not render context compaction as a visible conversation entry", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "contextCompaction",
        id: "compact-1",
        replacementHistory: [
          {
            type: "message",
            role: "assistant",
            content: [{ type: "output_text", text: "compact final output" }],
          },
        ],
      },
    ]),
  );

  assert.deepEqual(entries, []);
});

test("hides compact turn entries while preserving later visible items", () => {
  const entries = buildConversationEntries(
    makeThreadWithTurns([
      {
        id: "turn-1",
        items: [
          {
            type: "userMessage",
            id: "old-user",
            content: [{ type: "text", text: "old request" }],
          },
        ],
        itemsView: "full",
        status: "completed",
        error: null,
        startedAt: 1,
        completedAt: 1,
        durationMs: 0,
      },
      {
        id: "turn-2",
        items: [
          {
            type: "agentMessage",
            id: "compact-summary",
            text: "Summarizing previous context.",
            phase: null,
            memoryCitation: null,
          },
          {
            type: "contextCompaction",
            id: "compact-1",
            replacementHistory: [
              {
                type: "message",
                role: "assistant",
                content: [{ type: "output_text", text: "compact final output" }],
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
        ],
        itemsView: "full",
        status: "completed",
        error: null,
        startedAt: 2,
        completedAt: 2,
        durationMs: 0,
      },
    ]),
  );

  assert.deepEqual(
    entries.map((entry) => [entry.id, entry.kind, entry.text]),
    [
      ["after-compact", "message", "continued"],
    ],
  );
});

test("multiple compactions keep only entries after the latest hidden compact boundary", () => {
  const entries = buildConversationEntries(
    makeThreadWithTurns([
      {
        id: "turn-1",
        items: [
          {
            type: "userMessage",
            id: "old-user",
            content: [{ type: "text", text: "old request" }],
          },
        ],
        itemsView: "full",
        status: "completed",
        error: null,
        startedAt: 1,
        completedAt: 1,
        durationMs: 0,
      },
      {
        id: "turn-2",
        items: [
          {
            type: "contextCompaction",
            id: "compact-1",
            replacementHistory: [
              {
                type: "message",
                role: "assistant",
                content: [{ type: "output_text", text: "first compact output" }],
              },
            ],
          },
          {
            type: "agentMessage",
            id: "after-first-compact",
            text: "continued after first compact",
            phase: null,
            memoryCitation: null,
          },
        ],
        itemsView: "full",
        status: "completed",
        error: null,
        startedAt: 2,
        completedAt: 2,
        durationMs: 0,
      },
      {
        id: "turn-3",
        items: [
          {
            type: "contextCompaction",
            id: "compact-2",
            replacementHistory: [
              {
                type: "message",
                role: "assistant",
                content: [{ type: "output_text", text: "second compact output" }],
              },
            ],
          },
          {
            type: "agentMessage",
            id: "after-second-compact",
            text: "continued after second compact",
            phase: null,
            memoryCitation: null,
          },
        ],
        itemsView: "full",
        status: "completed",
        error: null,
        startedAt: 3,
        completedAt: 3,
        durationMs: 0,
      },
    ]),
  );

  assert.deepEqual(
    entries.map((entry) => [entry.id, entry.kind, entry.text]),
    [
      [
        "after-second-compact",
        "message",
        "continued after second compact",
      ],
    ],
  );
});

test("builds visible entries for empty reasoning and builtin schedule tools", () => {
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
        id: "builtin-schedule",
        tool: "schedule_subscribe",
        arguments: {
          label: "daily digest",
          schedule: {
            kind: "every_interval",
            interval_ms: 21_600_000,
          },
        },
        status: "completed",
        output: { subscription_id: "sub-1" },
      },
    ]),
  );

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
        id: "reasoning-empty",
        kind: "event",
        text: "Reasoning item received.",
        toolName: undefined,
        toolCategory: undefined,
      },
      {
        id: "builtin-schedule",
        kind: "tool",
        text: "schedule_subscribe • daily digest • every_interval 6h",
        toolName: "schedule_subscribe",
        toolCategory: "eventDrivenSubscription",
      },
    ],
  );
});

test("builds visible entries for poll_event builtin tools", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "builtinToolCall",
        id: "builtin-poll",
        tool: "poll_event",
        arguments: {},
        status: "completed",
        output: {
          timedOut: false,
          sourceHint: "mailbox_message",
          waitedMs: 14,
          initialTimeoutMs: 50,
          currentTimeoutMs: 50,
          hardCapTimeoutMs: 1000,
        },
      },
    ]),
  );

  assert.deepEqual(
    entries.map((entry) => ({
      id: entry.id,
      kind: entry.kind,
      text: entry.text,
      toolName: entry.toolName,
    })),
    [
      {
        id: "builtin-poll",
        kind: "tool",
        text: "poll_event • mailbox_message",
        toolName: "poll_event",
      },
    ],
  );
});

test("summarizes structured weekly and once-at schedule arguments", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "builtinToolCall",
        id: "builtin-weekly-schedule",
        tool: "schedule_subscribe",
        arguments: {
          label: "weekly sync",
          schedule: {
            kind: "every_week_at",
            weekdays: ["mon", "wed"],
            time: "09:00",
            timezone: "Asia/Shanghai",
          },
        },
        status: "completed",
        output: { subscription_id: "sub-weekly" },
      },
      {
        type: "builtinToolCall",
        id: "builtin-once-schedule",
        tool: "schedule_subscribe",
        arguments: {
          schedule: {
            kind: "once_at",
            run_at: "2026-07-14T09:00:00Z",
          },
        },
        status: "completed",
        output: { subscription_id: "sub-once" },
      },
    ]),
  );

  assert.deepEqual(
    entries.map((entry) => entry.text),
    [
      "schedule_subscribe • weekly sync • every_week_at mon,wed 09:00 Asia/Shanghai",
      "schedule_subscribe • once_at 2026-07-14T09:00:00Z",
    ],
  );
});

test("builds failed poll_event builtin entries without pretending they woke", () => {
  const entries = buildConversationEntries(
    makeThread([
      {
        type: "builtinToolCall",
        id: "builtin-poll-failed",
        tool: "poll_event",
        arguments: {},
        status: "failed",
        output: {
          error: "thread wait backend unavailable",
        },
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
        id: "builtin-poll-failed",
        kind: "tool",
        text: "poll_event • failed: thread wait backend unavailable",
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
