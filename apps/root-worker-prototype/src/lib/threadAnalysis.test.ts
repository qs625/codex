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

test("returns empty monitor sections when no thread is selected", () => {
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
      ["filesystem", "Filesystem", "No file watches.", []],
      ["process", "Processes", "No process listeners.", []],
      ["schedule", "Schedules", "No scheduled listeners.", []],
    ],
  );
});

test("hides subscriptions after their event is observed", () => {
  const analysis = buildThreadAnalysis(
    makeThread([
      {
        type: "eventDrivenToolCall",
        id: "fs-1",
        tool: "fs_subscribe",
        arguments: {
          path: "/tmp/out.log",
          recursive: true,
          label: "build log",
        },
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
        id: "fs-event-1",
        tool: "fs_subscribe",
        title: "File watch triggered",
        text: "[File subscription (build log)] File changed: /tmp/out.log",
      },
      {
        type: "eventDrivenTool",
        id: "process-event-1",
        tool: "process_exit_subscribe",
        title: "Process exited",
        text: "session 42 exited with code 0",
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

  assert.equal(analysis.monitors.totalCount, 0);
  assert.equal(analysis.monitors.eventCount, 3);
  assert.deepEqual(
    analysis.monitors.sections.map((section) => ({
      kind: section.kind,
      monitors: section.monitors,
    })),
    [
      {
        kind: "filesystem",
        monitors: [],
      },
      {
        kind: "process",
        monitors: [],
      },
      {
        kind: "schedule",
        monitors: [],
      },
    ],
  );
});

test("keeps subscriptions in listening state before an event is observed", () => {
  const analysis = buildThreadAnalysis(
    makeThread([
      {
        type: "eventDrivenToolCall",
        id: "fs-1",
        tool: "fs_subscribe",
        arguments: { path: "/tmp/out.log" },
        status: "completed",
        output: null,
      },
    ]),
    0,
  );

  assert.deepEqual(analysis.monitors.sections[0]?.monitors, [
    {
      id: "fs-1",
      subscriptionId: null,
      kind: "filesystem",
      label: "/tmp/out.log",
      detail: "/tmp/out.log",
      status: "Listening",
      eventCount: 0,
      latestEvent: null,
    },
  ]);
});

test("attributes same-kind events to matching subscriptions only", () => {
  const analysis = buildThreadAnalysis(
    makeThread([
      {
        type: "eventDrivenToolCall",
        id: "fs-1",
        tool: "fs_subscribe",
        arguments: { path: "/tmp/build.log", label: "build log" },
        status: "completed",
        output: null,
      },
      {
        type: "eventDrivenToolCall",
        id: "fs-2",
        tool: "fs_subscribe",
        arguments: { path: "/tmp/test.log", label: "test log" },
        status: "completed",
        output: null,
      },
      {
        type: "eventDrivenTool",
        id: "fs-event-1",
        tool: "fs_subscribe",
        title: "File watch triggered",
        text: "[File subscription (test log)] File changed: /tmp/test.log",
      },
    ]),
    0,
  );

  assert.deepEqual(analysis.monitors.sections[0]?.monitors, [
    {
      id: "fs-1",
      subscriptionId: null,
      kind: "filesystem",
      label: "build log",
      detail: "/tmp/build.log",
      status: "Listening",
      eventCount: 0,
      latestEvent: null,
    },
  ]);
});

test("removes monitors after matching unsubscribe calls", () => {
  const analysis = buildThreadAnalysis(
    makeThread([
      {
        type: "eventDrivenToolCall",
        id: "fs-1",
        tool: "fs_subscribe",
        arguments: { path: "/tmp/build.log", label: "build log" },
        status: "completed",
        output: { subscription_id: "sub-fs" },
      },
      {
        type: "eventDrivenToolCall",
        id: "fs-2",
        tool: "fs_subscribe",
        arguments: { path: "/tmp/test.log", label: "test log" },
        status: "completed",
        output: { subscription_id: "sub-test" },
      },
      {
        type: "eventDrivenToolCall",
        id: "fs-unsubscribe-1",
        tool: "fs_unsubscribe",
        arguments: { subscription_id: "sub-fs" },
        status: "completed",
        output: { unsubscribed: true, subscription_id: "sub-fs" },
      },
    ]),
    0,
  );

  assert.deepEqual(analysis.monitors.sections[0]?.monitors, [
    {
      id: "fs-2",
      subscriptionId: "sub-test",
      kind: "filesystem",
      label: "test log",
      detail: "/tmp/test.log",
      status: "Listening",
      eventCount: 0,
      latestEvent: null,
    },
  ]);
  assert.equal(analysis.monitors.totalCount, 1);
});

test("maps in-progress subscription calls to subscribing status", () => {
  const analysis = buildThreadAnalysis(
    makeThread([
      {
        type: "eventDrivenToolCall",
        id: "process-1",
        tool: "process_exit_subscribe",
        arguments: { session_id: 42 },
        status: "inProgress",
        output: null,
      },
    ]),
    0,
  );

  assert.deepEqual(analysis.monitors.sections[1]?.monitors, [
    {
      id: "process-1",
      subscriptionId: null,
      kind: "process",
      label: "Session 42",
      detail: "session 42",
      status: "Subscribing",
      eventCount: 0,
      latestEvent: null,
    },
  ]);
});
