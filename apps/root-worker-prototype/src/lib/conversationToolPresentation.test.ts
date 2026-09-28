import test from "node:test";
import assert from "node:assert/strict";

import {
  buildCommandLookup,
  commandExecutionNotificationOutput,
  extractEventDrivenSummaryDetails,
  formatCommandExecutionDetails,
  formatCommandExecutionNotificationDetails,
  formatEventCommandCallDetails,
  formatStructuredToolDetails,
  statusForCommandExecutionNotification,
  summarizeCommandExecution,
  summarizeCommandExecutionNotification,
  summarizeCommandWait,
  summarizeCommandWriteStdin,
  summarizeEventCommandCall,
  summarizeEventCommandEvent,
  toolCategoryForName,
} from "./conversationToolPresentation";
import type { Thread, ThreadItem } from "../types";

type CommandExecutionItem = Extract<ThreadItem, { type: "commandExecution" }>;
type CommandExecutionNotificationItem = Extract<
  ThreadItem,
  { type: "commandExecutionNotification" }
>;

function makeThread(items: ThreadItem[]): Thread {
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
    lifecycleStatus: { type: "active", activeFlags: [] },
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

function commandExecution(
  overrides: Partial<CommandExecutionItem> = {},
): CommandExecutionItem {
  return {
    type: "commandExecution",
    id: "cmd-1",
    command: "npm test",
    cwd: "/work/project",
    status: "inProgress",
    aggregatedOutput: null,
    exitCode: null,
    durationMs: 1234,
    initialWaitMs: 1000,
    notifyOn: "output",
    ...overrides,
  };
}

function commandNotification(
  overrides: Partial<CommandExecutionNotificationItem> = {},
): CommandExecutionNotificationItem {
  return {
    type: "commandExecutionNotification",
    id: "notice-1",
    commandItemId: "cmd-1",
    kind: "exit",
    message: "done",
    output: "ok",
    exitCode: 0,
    createdAtMs: 2000,
    ...overrides,
  };
}

test("projects command execution summaries and details without changing labels", () => {
  const running = commandExecution();
  const completed = commandExecution({
    status: "completed",
    exitCode: 2,
    durationMs: 2500,
  });

  assert.equal(summarizeCommandExecution(running), "work/project • running");
  assert.equal(summarizeCommandExecution(completed), "work/project • exit 2");
  assert.equal(
    formatCommandExecutionDetails(completed),
    [
      "Command\nnpm test",
      "Cwd\n/work/project",
      "Status\ncompleted",
      "Initial Wait\n1000 ms",
      "Notify On\noutput",
      "Duration\n2500 ms",
      "Exit Code\n2",
    ].join("\n\n"),
  );
});

test("projects command notifications through lookup, status, details, and output", () => {
  const lookup = buildCommandLookup(makeThread([commandExecution()]));
  const outputNotice = commandNotification({
    kind: "output",
    output: "line 1\nline 2",
    exitCode: null,
  });
  const failedExit = commandNotification({ exitCode: 1 });

  assert.equal(
    summarizeCommandExecutionNotification(outputNotice, lookup),
    "Command notification • output • npm test",
  );
  assert.equal(
    summarizeCommandExecutionNotification(failedExit, lookup),
    "Command notification • exit 1 • npm test",
  );
  assert.equal(
    statusForCommandExecutionNotification(outputNotice),
    "completed",
  );
  assert.equal(statusForCommandExecutionNotification(failedExit), "failed");
  assert.equal(
    formatCommandExecutionNotificationDetails(failedExit, "npm test"),
    [
      "Kind\nexit",
      "Command\nnpm test",
      "Command ID\ncmd-1",
      "Exit Code\n1",
      "Message\ndone",
    ].join("\n\n"),
  );
  assert.deepEqual(commandExecutionNotificationOutput(outputNotice), {
    label: "Command output",
    text: "line 1\nline 2",
    isEmpty: false,
    terminalEmulated: true,
  });
});

test("projects command wait and stdin event summaries", () => {
  assert.equal(
    summarizeCommandWait({
      type: "commandWait",
      id: "wait-1",
      commandId: "7",
      status: "completed",
      notification: "exit",
      exitCode: 0,
      wallTimeSeconds: 1.25,
      waitTimeoutMs: 300_000,
      createdAtMs: 2000,
    }),
    "Waited for command 7 with timeout 5m after exit notification: completed, exit 0 in 1.25s.",
  );
  assert.equal(
    summarizeCommandWriteStdin({
      type: "commandWriteStdin",
      id: "stdin-1",
      commandId: "7",
      bytesWritten: 4,
      containsNewline: true,
      createdAtMs: 3000,
    }),
    "Wrote 4 bytes to command 7 including newline.",
  );
});

test("projects event command call details and event summaries", () => {
  const call = {
    type: "eventCommandCall" as const,
    id: "monitor-1",
    subscriptionId: "sub-1",
    command: "cargo test -p app-server",
    cwd: "/tmp/project",
    label: "app-server tests",
    status: "completed",
    output: { ok: true },
  };

  assert.equal(
    summarizeEventCommandCall(call),
    "app-server tests • cargo test -p app-server",
  );
  assert.equal(
    formatEventCommandCallDetails(call),
    [
      "Command\ncargo test -p app-server",
      "Directory\n/tmp/project",
      "Label\napp-server tests",
      "Subscription\nsub-1",
      'Output\n{\n  "ok": true\n}',
    ].join("\n\n"),
  );
  assert.equal(
    summarizeEventCommandEvent({
      type: "eventCommandEvent",
      id: "event-1",
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
    }),
    "app-server tests: running 1 test",
  );
});

test("projects structured tool details and event-driven summary metadata", () => {
  assert.equal(
    formatStructuredToolDetails({ path: "/tmp" }, { ok: true }),
    ['Arguments\n{\n  "path": "/tmp"\n}', 'Output\n{\n  "ok": true\n}'].join(
      "\n\n",
    ),
  );
  assert.equal(
    extractEventDrivenSummaryDetails("process_exit_subscribe", {
      session_id: 42,
    }),
    "session 42",
  );
  assert.equal(
    extractEventDrivenSummaryDetails("fs_subscribe", {
      label: "repo",
      path: "/work/project",
    }),
    "repo • /work/project",
  );
  assert.equal(
    extractEventDrivenSummaryDetails("schedule_subscribe", {
      label: "daily digest",
      schedule: "every_interval 6h",
    }),
    "daily digest • every_interval 6h",
  );
});

test("classifies built-in tool categories without changing fallback behavior", () => {
  assert.equal(toolCategoryForName("read_agent"), "multiAgent");
  assert.equal(
    toolCategoryForName("process_exit_subscribe"),
    "eventDrivenSubscription",
  );
  assert.equal(toolCategoryForName("unknown_tool", "namespace"), "external");
});
