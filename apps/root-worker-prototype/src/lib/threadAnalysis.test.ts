import test from "node:test";
import assert from "node:assert/strict";

import { buildThreadAnalysis } from "./threadAnalysis";
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
    status: { type: "complete" },
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

test("returns command and schedule monitor sections when no thread is selected", () => {
  const analysis = buildThreadAnalysis(null, 4);

  assert.equal(analysis.contextUsage.totalSkills, 4);
  assert.equal(analysis.monitors.totalCount, 0);
  assert.equal(analysis.monitors.eventCount, 0);
  assert.deepEqual(analysis.changedFiles, []);
  assert.deepEqual(
    analysis.monitors.sections.map((section) => [
      section.kind,
      section.title,
      section.emptyLabel,
      section.monitors,
    ]),
    [
      ["command", "Live Commands", "No live commands.", []],
      ["schedule", "Schedules", "No scheduled listeners.", []],
    ],
  );
});

test("dedupes changed files and keeps the latest change kind", () => {
  const analysis = buildThreadAnalysis(
    makeThread([
      {
        type: "fileChange",
        id: "change-1",
        status: "completed",
        changes: [
          { path: "/tmp/src/App.tsx", kind: "modified" },
          { path: "/tmp/package.json", kind: "added" },
        ],
      },
      {
        type: "fileChange",
        id: "change-2",
        status: "completed",
        changes: [{ path: "/tmp/src/App.tsx", kind: "deleted" }],
      },
    ]),
    0,
  );

  assert.deepEqual(analysis.changedFiles, [
    {
      path: "/tmp/src/App.tsx",
      displayPath: "src/App.tsx",
      kind: "deleted",
      updateCount: 2,
    },
    {
      path: "/tmp/package.json",
      displayPath: "package.json",
      kind: "added",
      updateCount: 1,
    },
  ]);
});

test("ignores incomplete file change items", () => {
  const analysis = buildThreadAnalysis(
    makeThread([
      {
        type: "fileChange",
        id: "change-1",
        status: "running",
        changes: [{ path: "/tmp/src/App.tsx", kind: "modified" }],
      },
      {
        type: "fileChange",
        id: "change-2",
        status: "failed",
        changes: [{ path: "/tmp/src/App.tsx", kind: "deleted" }],
      },
      {
        type: "fileChange",
        id: "change-3",
        status: "completed",
        changes: [{ path: "/tmp/package.json", kind: "added" }],
      },
    ]),
    0,
  );

  assert.deepEqual(analysis.changedFiles, [
    {
      path: "/tmp/package.json",
      displayPath: "package.json",
      kind: "added",
      updateCount: 1,
    },
  ]);
});

test("keeps running command active and records latest output line", () => {
  const analysis = buildThreadAnalysis(
    makeThread([
      {
        type: "commandExecution",
        id: "command-1",
        command: "tail -f /tmp/build.log",
        cwd: "/repo",
        status: "running",
        aggregatedOutput: "starting\nchanged:/tmp/build.log\n",
        exitCode: null,
        durationMs: null,
      },
    ]),
    0,
  );

  assert.equal(analysis.monitors.totalCount, 1);
  assert.equal(analysis.monitors.eventCount, 1);
  assert.deepEqual(analysis.monitors.sections[0]?.monitors, [
    {
      id: "command-1",
      subscriptionId: "command-1",
      kind: "command",
      label: "tail -f /tmp/build.log",
      detail: "/repo",
      status: "Running",
      eventCount: 1,
      latestEvent: "changed:/tmp/build.log",
    },
  ]);
});

test("keeps in-progress command statuses active", () => {
  const analysis = buildThreadAnalysis(
    makeThread([
      {
        type: "commandExecution",
        id: "command-1",
        command: "rtk cargo test",
        cwd: "/repo",
        status: "inProgress",
        aggregatedOutput: null,
        exitCode: null,
        durationMs: null,
      },
      {
        type: "commandExecution",
        id: "command-2",
        command: "rtk pnpm test",
        cwd: "/repo",
        status: "in_progress",
        aggregatedOutput: null,
        exitCode: null,
        durationMs: null,
      },
    ]),
    0,
  );

  assert.equal(analysis.monitors.totalCount, 2);
  assert.equal(analysis.monitors.sections[0]?.monitors[0]?.status, "Running");
  assert.equal(analysis.monitors.sections[0]?.monitors[1]?.status, "Running");
});

test("omits failed completed commands from live command index", () => {
  const analysis = buildThreadAnalysis(
    makeThread([
      {
        type: "commandExecution",
        id: "command-1",
        command: "cargo test -p codex-tui",
        cwd: "/repo",
        status: "completed",
        aggregatedOutput: "test failed\n",
        exitCode: 101,
        durationMs: 1200,
      },
    ]),
    0,
  );

  assert.equal(analysis.monitors.totalCount, 0);
  assert.deepEqual(analysis.monitors.sections[0]?.monitors, []);
});

test("uses command notification as latest live command event", () => {
  const analysis = buildThreadAnalysis(
    makeThread([
      {
        type: "commandExecution",
        id: "command-1",
        command: "tail -f /tmp/build.log",
        cwd: "/repo",
        status: "running",
        aggregatedOutput: "older output\n",
        exitCode: null,
        durationMs: null,
      },
      {
        type: "commandExecutionNotification",
        id: "command-1:notification:output:1",
        commandItemId: "command-1",
        kind: "output",
        message: "Command output notification received.",
        output: "fresh notification",
        exitCode: null,
        createdAtMs: 1,
      },
    ]),
    0,
  );

  assert.equal(
    analysis.monitors.sections[0]?.monitors[0]?.latestEvent,
    "fresh notification",
  );
});

test("omits successful completed commands from live command index", () => {
  const analysis = buildThreadAnalysis(
    makeThread([
      {
        type: "commandExecution",
        id: "command-1",
        command: "cargo test -p codex-tui",
        cwd: "/repo",
        status: "completed",
        aggregatedOutput: "ok\n",
        exitCode: 0,
        durationMs: 1200,
      },
    ]),
    0,
  );

  assert.equal(analysis.monitors.totalCount, 0);
  assert.deepEqual(analysis.monitors.sections[0]?.monitors, []);
});

test("does not restore legacy fs or process monitors as active", () => {
  const analysis = buildThreadAnalysis(
    makeThread([
      {
        type: "eventDrivenToolCall",
        id: "fs-1",
        tool: "fs_subscribe",
        arguments: { path: "/tmp/out.log" },
        status: "completed",
        output: { subscription_id: "sub-fs" },
      },
      {
        type: "eventDrivenToolCall",
        id: "process-1",
        tool: "process_exit_subscribe",
        arguments: { session_id: 42 },
        status: "completed",
        output: { subscription_id: "sub-process" },
      },
    ]),
    0,
  );

  assert.equal(analysis.monitors.totalCount, 0);
  assert.deepEqual(
    analysis.monitors.sections.flatMap((section) => section.monitors),
    [],
  );
});

test("keeps schedule subscriptions separate from command sessions", () => {
  const analysis = buildThreadAnalysis(
    makeThread([
      {
        type: "eventDrivenToolCall",
        id: "schedule-1",
        tool: "schedule_subscribe",
        arguments: {
          schedule: { once_after: { seconds: 60 } },
          label: "standup ping",
        },
        status: "completed",
        output: { schedule_summary: "once after 60s" },
      },
      {
        type: "eventDrivenTool",
        id: "schedule-event-1",
        tool: "schedule_subscribe",
        title: "Schedule triggered",
        text: "[Schedule subscription (standup ping)] Trigger fired",
      },
    ]),
    0,
  );

  assert.equal(analysis.monitors.totalCount, 1);
  assert.equal(analysis.monitors.eventCount, 1);
  assert.deepEqual(analysis.monitors.sections[1]?.monitors, [
    {
      id: "schedule-1",
      subscriptionId: null,
      kind: "schedule",
      label: "standup ping",
      detail: "once after 60s",
      status: "Listening",
      eventCount: 1,
      latestEvent: "[Schedule subscription (standup ping)] Trigger fired",
    },
  ]);
});
