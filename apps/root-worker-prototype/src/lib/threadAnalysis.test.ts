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

test("returns EventCommand and schedule monitor sections when no thread is selected", () => {
  const analysis = buildThreadAnalysis(null, 4);

  assert.equal(analysis.contextUsage.totalSkills, 4);
  assert.equal(analysis.monitors.totalCount, 0);
  assert.equal(analysis.monitors.eventCount, 0);
  assert.deepEqual(
    analysis.monitors.sections.map((section) => [
      section.kind,
      section.title,
      section.emptyLabel,
      section.monitors,
    ]),
    [
      ["eventCommand", "Event commands", "No command monitors.", []],
      ["schedule", "Schedules", "No scheduled listeners.", []],
    ],
  );
});

test("keeps EventCommand active and records output events", () => {
  const analysis = buildThreadAnalysis(
    makeThread([
      {
        type: "eventCommandCall",
        id: "event-command-1",
        subscriptionId: "sub-command",
        command: "tail -f /tmp/build.log",
        cwd: "/repo",
        label: "build log",
        status: "completed",
        output: { subscription_id: "sub-command" },
      },
      {
        type: "eventCommandEvent",
        id: "event-command-event-1",
        subscriptionId: "sub-command",
        kind: "output",
        label: "build log",
        command: "tail -f /tmp/build.log",
        cwd: "/repo",
        line: "changed:/tmp/build.log",
        sequence: 1,
        exitCode: null,
        signal: null,
        message: null,
        truncated: false,
        createdAt: 1,
      },
    ]),
    0,
  );

  assert.equal(analysis.monitors.totalCount, 1);
  assert.equal(analysis.monitors.eventCount, 0);
  assert.deepEqual(analysis.monitors.sections[0]?.monitors, [
    {
      id: "event-command-1",
      subscriptionId: "sub-command",
      kind: "eventCommand",
      label: "build log",
      detail: "tail -f /tmp/build.log (/repo)",
      status: "Listening",
      eventCount: 1,
      latestEvent: "changed:/tmp/build.log",
    },
  ]);
});

test("attaches early EventCommand output before subscription id is recorded", () => {
  const analysis = buildThreadAnalysis(
    makeThread([
      {
        type: "eventCommandCall",
        id: "event-command-1",
        subscriptionId: "",
        command: "tail -f /tmp/build.log",
        cwd: "/repo",
        label: "build log",
        status: "inProgress",
        output: null,
      },
      {
        type: "eventCommandEvent",
        id: "event-command-event-1",
        subscriptionId: "sub-command",
        kind: "output",
        label: "build log",
        command: "tail -f /tmp/build.log",
        cwd: "/repo",
        line: "changed:/tmp/build.log",
        sequence: 1,
        exitCode: null,
        signal: null,
        message: null,
        truncated: false,
        createdAt: 1,
      },
    ]),
    0,
  );

  assert.equal(analysis.monitors.totalCount, 1);
  assert.deepEqual(analysis.monitors.sections[0]?.monitors, [
    {
      id: "event-command-1",
      subscriptionId: "sub-command",
      kind: "eventCommand",
      label: "build log",
      detail: "tail -f /tmp/build.log (/repo)",
      status: "Subscribing",
      eventCount: 1,
      latestEvent: "changed:/tmp/build.log",
    },
  ]);
});

test("removes EventCommand monitor after early terminal event", () => {
  const analysis = buildThreadAnalysis(
    makeThread([
      {
        type: "eventCommandCall",
        id: "event-command-1",
        subscriptionId: "",
        command: "cargo test -p codex-tui",
        cwd: "/repo",
        label: "tui tests",
        status: "inProgress",
        output: null,
      },
      {
        type: "eventCommandEvent",
        id: "event-command-event-1",
        subscriptionId: "sub-command",
        kind: "exited",
        label: "tui tests",
        command: "cargo test -p codex-tui",
        cwd: "/repo",
        line: null,
        sequence: null,
        exitCode: 0,
        signal: null,
        message: "EventCommand exited with status exit status: 0",
        truncated: false,
        createdAt: 1,
      },
    ]),
    0,
  );

  assert.equal(analysis.monitors.totalCount, 0);
  assert.deepEqual(analysis.monitors.sections[0]?.monitors, []);
});

test("removes EventCommand monitor after terminal event", () => {
  const analysis = buildThreadAnalysis(
    makeThread([
      {
        type: "eventCommandCall",
        id: "event-command-1",
        subscriptionId: "sub-command",
        command: "cargo test -p codex-tui",
        cwd: "/repo",
        label: "tui tests",
        status: "completed",
        output: { subscription_id: "sub-command" },
      },
      {
        type: "eventCommandEvent",
        id: "event-command-event-1",
        subscriptionId: "sub-command",
        kind: "exited",
        label: "tui tests",
        command: "cargo test -p codex-tui",
        cwd: "/repo",
        line: null,
        sequence: null,
        exitCode: 0,
        signal: null,
        message: "EventCommand exited with status exit status: 0",
        truncated: false,
        createdAt: 1,
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

test("keeps schedule subscriptions separate from EventCommand monitors", () => {
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
