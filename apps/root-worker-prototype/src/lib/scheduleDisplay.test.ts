import test from "node:test";
import assert from "node:assert/strict";

import {
  buildScheduleOccurrences,
  formatScheduleArgument,
  formatScheduleRule,
} from "./scheduleDisplay";

test("formatScheduleArgument preserves raw strings and object fallbacks", () => {
  assert.equal(formatScheduleArgument(" every 5m "), "every 5m");
  assert.equal(formatScheduleArgument(null), null);
  assert.equal(
    formatScheduleArgument({ label: "custom" }),
    '{"label":"custom"}',
  );
  assert.equal(
    formatScheduleArgument({ kind: "unknown_kind" }),
    "unknown_kind",
  );
});

test("formatScheduleArgument and formatScheduleRule project schedule variants", () => {
  assert.equal(
    formatScheduleArgument({ kind: "every_interval", interval_ms: 90_000 }),
    "every_interval 90s",
  );
  assert.equal(
    formatScheduleRule({ kind: "every_interval", interval_ms: 86_400_000 }),
    "Every 1 day",
  );
  assert.equal(
    formatScheduleArgument({ kind: "once_after", delay_ms: 3_600_000 }),
    "once_after 1h",
  );
  assert.equal(
    formatScheduleRule({ kind: "once_after", delay_ms: 1_500 }),
    "Once after 1500 milliseconds",
  );
  assert.equal(
    formatScheduleArgument({
      kind: "every_week_at",
      weekdays: ["mon", "wednesday"],
      time: "09:30",
      timezone: "UTC",
    }),
    "every_week_at mon,wednesday 09:30 UTC",
  );
  assert.equal(
    formatScheduleRule({
      kind: "every_day_at",
      time: "17:45",
      timezone: "Asia/Shanghai",
    }),
    "Daily 17:45 Asia/Shanghai",
  );
  assert.equal(
    formatScheduleArgument({
      kind: "once_at",
      run_at: "2026-01-01T00:00:00.000Z",
    }),
    "once_at 2026-01-01T00:00:00.000Z",
  );
  assert.equal(formatScheduleRule({ kind: "once_at" }), "Once");
});

test("buildScheduleOccurrences advances interval schedules from stale next fire times", () => {
  const occurrences = buildScheduleOccurrences(
    { kind: "every_interval", interval_ms: 60_000 },
    {
      now: "2026-01-01T00:05:30.000Z",
      nextFireAt: "2026-01-01T00:03:00.000Z",
      limit: 3,
      horizonDays: 1,
    },
  );

  assert.deepEqual(occurrences, [
    { startsAt: "2026-01-01T00:06:00.000Z" },
    { startsAt: "2026-01-01T00:07:00.000Z" },
    { startsAt: "2026-01-01T00:08:00.000Z" },
  ]);
});

test("buildScheduleOccurrences prefers nextFireAt for once_after schedules", () => {
  const occurrences = buildScheduleOccurrences(
    { kind: "once_after", delay_ms: 3_600_000 },
    {
      now: "2026-01-01T00:00:00.000Z",
      nextFireAt: "2026-01-01T00:10:00.000Z",
      horizonDays: 1,
    },
  );

  assert.deepEqual(occurrences, [{ startsAt: "2026-01-01T00:10:00.000Z" }]);
});

test("buildScheduleOccurrences filters one-shot dates outside the active window", () => {
  assert.deepEqual(
    buildScheduleOccurrences(
      { kind: "once_at", run_at: "2026-01-01T00:00:00.000Z" },
      { now: "2026-01-01T00:00:00.000Z", horizonDays: 1 },
    ),
    [],
  );
  assert.deepEqual(
    buildScheduleOccurrences(
      { kind: "once_at", run_at: "2026-01-03T00:00:01.000Z" },
      { now: "2026-01-01T00:00:00.000Z", horizonDays: 2 },
    ),
    [],
  );
});

test("buildScheduleOccurrences projects daily schedules in the requested timezone", () => {
  const occurrences = buildScheduleOccurrences(
    { kind: "every_day_at", time: "09:30", timezone: "UTC" },
    {
      now: "2026-01-01T09:29:00.000Z",
      limit: 2,
      horizonDays: 2,
    },
  );

  assert.deepEqual(occurrences, [
    { startsAt: "2026-01-01T09:30:00.000Z" },
    { startsAt: "2026-01-02T09:30:00.000Z" },
  ]);
});

test("buildScheduleOccurrences projects weekly schedules with aliases and limit", () => {
  const occurrences = buildScheduleOccurrences(
    {
      kind: "every_week_at",
      weekdays: ["tue", "friday"],
      time: "12:00",
      timezone: "UTC",
    },
    {
      now: "2026-01-05T00:00:00.000Z",
      limit: 2,
      horizonDays: 7,
    },
  );

  assert.deepEqual(occurrences, [
    { startsAt: "2026-01-06T12:00:00.000Z" },
    { startsAt: "2026-01-09T12:00:00.000Z" },
  ]);
});

test("buildScheduleOccurrences rejects invalid schedules without falling back", () => {
  assert.deepEqual(buildScheduleOccurrences("every day"), []);
  assert.deepEqual(
    buildScheduleOccurrences(
      { kind: "every_interval", interval_ms: 0 },
      { now: "2026-01-01T00:00:00.000Z" },
    ),
    [],
  );
  assert.deepEqual(
    buildScheduleOccurrences(
      { kind: "every_day_at", time: "25:00", timezone: "UTC" },
      { now: "2026-01-01T00:00:00.000Z" },
    ),
    [],
  );
});
