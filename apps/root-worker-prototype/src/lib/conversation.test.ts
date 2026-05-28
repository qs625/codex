import test from "node:test";
import assert from "node:assert/strict";

import { buildConversationCells, buildConversationEntries } from "./conversation";
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
    status: "running",
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

test("separates command, builtin, and multi-agent items into different tool cells", () => {
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
        type: "builtinToolCall",
        id: "builtin-1",
        tool: "view_image",
        arguments: { path: "/tmp/image.png" },
        status: "completed",
        output: { ok: true },
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
    entries.map((entry) => [entry.toolCategory, entry.toolName]),
    [
      ["command", "ls"],
      ["builtin", "view_image"],
      ["multiAgent", "spawn agent"],
    ],
  );

  const cells = buildConversationCells(entries);
  assert.equal(cells.length, 3);
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
