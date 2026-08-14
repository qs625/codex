import test from "node:test";
import assert from "node:assert/strict";

import { buildThreadAnalysis } from "./threadAnalysis";
import type { Thread, ThreadLifecycleStatus } from "../types";

function makeThread(
  items: Thread["turns"][number]["items"],
  lifecycleStatus: ThreadLifecycleStatus = {
    type: "final",
    result: { type: "completed" },
  },
): Thread {
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
    lifecycleStatus,
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

test("builds schedule monitors from active subscription current state items", () => {
  const thread = {
    ...makeThread([]),
    turns: [] as Thread["turns"],
    activeSubscriptionItems: [
      {
        type: "builtinToolCall",
        id: "active-subscription:sub-schedule",
        tool: "schedule_subscribe",
        arguments: {
          label: "daily digest",
          schedule: { kind: "every_interval", interval_ms: 21_600_000 },
        },
        status: "completed",
        output: {
          subscription_id: "sub-schedule",
          schedule_summary: "every 21600000 ms",
        },
      },
    ],
  } satisfies Thread;

  const analysis = buildThreadAnalysis(thread, 4);

  assert.deepEqual(
    thread.turns.flatMap((turn) => turn.items.map((item) => item.id)),
    [],
  );
  assert.deepEqual(
    analysis.monitors.sections.find((section) => section.kind === "schedule")
      ?.monitors,
    [
      {
        id: "active-subscription:sub-schedule",
        subscriptionId: "sub-schedule",
        kind: "schedule",
        label: "daily digest",
        detail: "every_interval 6h",
        status: "Listening",
        eventCount: 0,
        latestEvent: null,
      },
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
    makeThread(
      [
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
      ],
      { type: "active", activeFlags: ["running"] },
    ),
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
    makeThread(
      [
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
      ],
      { type: "waiting", reason: "command" },
    ),
    0,
  );

  assert.equal(analysis.monitors.totalCount, 2);
  assert.equal(analysis.monitors.sections[0]?.monitors[0]?.status, "Running");
  assert.equal(analysis.monitors.sections[0]?.monitors[1]?.status, "Running");
});

test("keeps live commands visible while thread waits on a child", () => {
  const analysis = buildThreadAnalysis(
    makeThread(
      [
        {
          type: "commandExecution",
          id: "command-1",
          command: "rtk tail -f /tmp/build.log",
          cwd: "/repo",
          status: "running",
          aggregatedOutput: null,
          exitCode: null,
          durationMs: null,
        },
      ],
      { type: "waiting", reason: "child" },
    ),
    0,
  );

  assert.equal(analysis.monitors.totalCount, 1);
  assert.equal(
    analysis.monitors.sections[0]?.monitors[0]?.label,
    "rtk tail -f /tmp/build.log",
  );
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
    makeThread(
      [
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
      ],
      { type: "waiting", reason: "command" },
    ),
    0,
  );

  assert.equal(
    analysis.monitors.sections[0]?.monitors[0]?.latestEvent,
    "fresh notification",
  );
});

test("ignores stale running command residue after reload when thread is complete", () => {
  const analysis = buildThreadAnalysis(
    makeThread([
      {
        type: "commandExecution",
        id: "command-1",
        command: "rtk cargo test -p app-server",
        cwd: "/repo",
        status: "running",
        aggregatedOutput: "Compiling app-server\n",
        exitCode: null,
        durationMs: null,
      },
    ]),
    0,
  );

  assert.equal(analysis.monitors.totalCount, 0);
  assert.deepEqual(analysis.monitors.sections[0]?.monitors, []);
});

test("ignores stale running command residue when thread is only waiting on approval", () => {
  const analysis = buildThreadAnalysis(
    makeThread(
      [
        {
          type: "commandExecution",
          id: "command-1",
          command: "rtk cargo test -p app-server",
          cwd: "/repo",
          status: "running",
          aggregatedOutput: "Awaiting approval\n",
          exitCode: null,
          durationMs: null,
        },
      ],
      { type: "active", activeFlags: ["waitingOnApproval"] },
    ),
    0,
  );

  assert.equal(analysis.monitors.totalCount, 0);
  assert.deepEqual(analysis.monitors.sections[0]?.monitors, []);
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
        type: "builtinToolCall",
        id: "schedule-1",
        tool: "schedule_subscribe",
        arguments: {
          schedule: { kind: "every_interval", interval_ms: 21_600_000 },
          label: "standup ping",
        },
        status: "completed",
        output: {
          subscription_id: "sub-schedule",
          schedule_summary: "every 21600000 ms",
          next_fire_at: "2026-07-13T09:35:37.570867+00:00",
        },
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
    null,
    {
      now: "2026-07-13T08:00:00Z",
      agendaTimeZone: "UTC",
      agendaLimit: 3,
      agendaHorizonDays: 1,
    },
  );

  assert.equal(analysis.monitors.totalCount, 1);
  assert.equal(analysis.monitors.eventCount, 1);
  assert.deepEqual(analysis.monitors.scheduleAgenda, [
    {
      dateKey: "2026-07-13",
      dateLabel: "Today",
      items: [
        {
          id: "schedule-1:2026-07-13T09:35:37.570Z",
          subscriptionId: "sub-schedule",
          label: "standup ping",
          rule: "Every 6 hours",
          startsAt: "2026-07-13T09:35:37.570Z",
          timeLabel: "09:35",
        },
        {
          id: "schedule-1:2026-07-13T15:35:37.570Z",
          subscriptionId: "sub-schedule",
          label: "standup ping",
          rule: "Every 6 hours",
          startsAt: "2026-07-13T15:35:37.570Z",
          timeLabel: "15:35",
        },
        {
          id: "schedule-1:2026-07-13T21:35:37.570Z",
          subscriptionId: "sub-schedule",
          label: "standup ping",
          rule: "Every 6 hours",
          startsAt: "2026-07-13T21:35:37.570Z",
          timeLabel: "21:35",
        },
      ],
    },
  ]);
  assert.deepEqual(analysis.monitors.sections[1]?.monitors, [
    {
      id: "schedule-1",
      subscriptionId: "sub-schedule",
      kind: "schedule",
      label: "standup ping",
      detail: "every_interval 6h",
      status: "Listening",
      eventCount: 1,
      latestEvent: "[Schedule subscription (standup ping)] Trigger fired",
    },
  ]);
});

test("falls back to schedule output summary when arguments are unavailable", () => {
  const analysis = buildThreadAnalysis(
    makeThread([
      {
        type: "builtinToolCall",
        id: "schedule-1",
        tool: "schedule_subscribe",
        arguments: {
          label: "legacy schedule",
        },
        status: "completed",
        output: {
          subscription_id: "sub-schedule",
          schedule_summary: "every 21600000 ms",
        },
      },
    ]),
    0,
  );

  assert.equal(
    analysis.monitors.sections[1]?.monitors[0]?.detail,
    "every 21600000 ms",
  );
  assert.deepEqual(analysis.monitors.scheduleAgenda, []);
});

test("skips stale interval occurrences without iterative catch-up", () => {
  const analysis = buildThreadAnalysis(
    makeThread([
      {
        type: "builtinToolCall",
        id: "schedule-1",
        tool: "schedule_subscribe",
        arguments: {
          schedule: { kind: "every_interval", interval_ms: 21_600_000 },
          label: "cleanup",
        },
        status: "completed",
        output: {
          subscription_id: "sub-schedule",
          next_fire_at: "2026-07-01T00:00:00Z",
        },
      },
    ]),
    0,
    null,
    {
      now: "2026-07-13T08:00:00Z",
      agendaTimeZone: "UTC",
      agendaLimit: 2,
      agendaHorizonDays: 1,
    },
  );

  assert.deepEqual(
    analysis.monitors.scheduleAgenda.flatMap((group) =>
      group.items.map((item) => item.startsAt),
    ),
    ["2026-07-13T12:00:00.000Z", "2026-07-13T18:00:00.000Z"],
  );
});

test("builds agenda entries for one-shot daily and weekly schedules", () => {
  const analysis = buildThreadAnalysis(
    makeThread([
      {
        type: "builtinToolCall",
        id: "once",
        tool: "schedule_subscribe",
        arguments: {
          schedule: { kind: "once_at", run_at: "2026-07-13T08:30:00Z" },
          label: "deploy",
        },
        status: "completed",
        output: { subscription_id: "once-sub" },
      },
      {
        type: "builtinToolCall",
        id: "daily",
        tool: "schedule_subscribe",
        arguments: {
          schedule: { kind: "every_day_at", time: "09:00:15", timezone: "UTC" },
          label: "daily digest",
        },
        status: "completed",
        output: { subscription_id: "daily-sub" },
      },
      {
        type: "builtinToolCall",
        id: "weekly",
        tool: "schedule_subscribe",
        arguments: {
          schedule: {
            kind: "every_week_at",
            weekdays: ["mon", "wed"],
            time: "10:00:30",
            timezone: "UTC",
          },
          label: "weekly sync",
        },
        status: "completed",
        output: { subscription_id: "weekly-sub" },
      },
      {
        type: "builtinToolCall",
        id: "past-once",
        tool: "schedule_subscribe",
        arguments: {
          schedule: { kind: "once_at", run_at: "2026-07-13T07:00:00Z" },
          label: "past deploy",
        },
        status: "completed",
        output: { subscription_id: "past-once-sub" },
      },
    ]),
    0,
    null,
    {
      now: "2026-07-13T08:00:00Z",
      agendaTimeZone: "UTC",
      agendaLimit: 5,
      agendaHorizonDays: 3,
    },
  );

  assert.deepEqual(
    analysis.monitors.scheduleAgenda.flatMap((group) =>
      group.items.map(
        (item) =>
          `${group.dateKey} ${item.timeLabel} ${item.label} ${item.rule}`,
      ),
    ),
    [
      "2026-07-13 08:30 deploy Once",
      "2026-07-13 09:00 daily digest Daily 09:00:15 UTC",
      "2026-07-13 10:00 weekly sync Weekly mon,wed 10:00:30 UTC",
      "2026-07-14 09:00 daily digest Daily 09:00:15 UTC",
      "2026-07-15 09:00 daily digest Daily 09:00:15 UTC",
    ],
  );
  assert.deepEqual(
    analysis.monitors.scheduleAgenda
      .slice(0, 2)
      .map((group) => group.dateLabel),
    ["Today", "Tomorrow"],
  );
  assert.deepEqual(
    analysis.monitors.scheduleAgenda.flatMap((group) =>
      group.items.map((item) => item.startsAt),
    ),
    [
      "2026-07-13T08:30:00.000Z",
      "2026-07-13T09:00:15.000Z",
      "2026-07-13T10:00:30.000Z",
      "2026-07-14T09:00:15.000Z",
      "2026-07-15T09:00:15.000Z",
    ],
  );
});

test("removes builtin schedule subscriptions after unsubscribe", () => {
  const analysis = buildThreadAnalysis(
    makeThread([
      {
        type: "builtinToolCall",
        id: "schedule-1",
        tool: "schedule_subscribe",
        arguments: {
          schedule: { kind: "every_interval", interval_ms: 21_600_000 },
          label: "cargo clean",
        },
        status: "completed",
        output: {
          subscription_id: "sub-schedule",
          schedule_summary: "every 21600000 ms",
        },
      },
      {
        type: "builtinToolCall",
        id: "schedule-unsub-1",
        tool: "schedule_unsubscribe",
        arguments: {
          subscription_id: "sub-schedule",
        },
        status: "completed",
        output: {
          subscription_id: "sub-schedule",
          unsubscribed: true,
        },
      },
    ]),
    0,
  );

  assert.equal(analysis.monitors.totalCount, 0);
  assert.deepEqual(analysis.monitors.sections[1]?.monitors, []);
  assert.deepEqual(analysis.monitors.scheduleAgenda, []);
});

test("keeps schedules active while unsubscribe has not succeeded", () => {
  const analysis = buildThreadAnalysis(
    makeThread([
      {
        type: "builtinToolCall",
        id: "schedule-1",
        tool: "schedule_subscribe",
        arguments: {
          schedule: { kind: "every_interval", interval_ms: 21_600_000 },
          label: "cargo clean",
        },
        status: "completed",
        output: {
          subscription_id: "sub-schedule",
          schedule_summary: "every 21600000 ms",
        },
      },
      {
        type: "builtinToolCall",
        id: "schedule-unsub-started",
        tool: "schedule_unsubscribe",
        arguments: { subscription_id: "sub-schedule" },
        status: "inProgress",
        output: null,
      },
      {
        type: "builtinToolCall",
        id: "schedule-unsub-failed",
        tool: "schedule_unsubscribe",
        arguments: { subscription_id: "sub-schedule" },
        status: "failed",
        output: { subscription_id: "sub-schedule", error: "not removed" },
      },
      {
        type: "builtinToolCall",
        id: "schedule-unsub-false",
        tool: "schedule_unsubscribe",
        arguments: { subscription_id: "sub-schedule" },
        status: "completed",
        output: { subscription_id: "sub-schedule", unsubscribed: false },
      },
      {
        type: "builtinToolCall",
        id: "schedule-failed",
        tool: "schedule_subscribe",
        arguments: {
          schedule: { kind: "once_after", delay_ms: 0 },
          label: "bad schedule",
        },
        status: "failed",
        output: { error: "delay_ms must be greater than zero" },
      },
    ]),
    0,
  );

  assert.equal(analysis.monitors.totalCount, 1);
  assert.equal(analysis.monitors.sections[1]?.monitors[0]?.subscriptionId, "sub-schedule");
  assert.equal(analysis.monitors.sections[1]?.monitors[0]?.label, "cargo clean");
});
