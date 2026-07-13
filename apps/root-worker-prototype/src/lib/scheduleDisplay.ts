export type ScheduleOccurrence = {
  startsAt: string;
};

export type ScheduleOccurrenceOptions = {
  now?: Date | string | number;
  nextFireAt?: string | null;
  limit?: number;
  horizonDays?: number;
};

const DEFAULT_OCCURRENCE_LIMIT = 20;
const DEFAULT_OCCURRENCE_HORIZON_DAYS = 7;
const WEEKDAY_INDEX: Record<string, number> = {
  sun: 0,
  sunday: 0,
  mon: 1,
  monday: 1,
  tue: 2,
  tuesday: 2,
  wed: 3,
  wednesday: 3,
  thu: 4,
  thursday: 4,
  fri: 5,
  friday: 5,
  sat: 6,
  saturday: 6,
};

export function formatScheduleArgument(value: unknown) {
  const text = stringOrNull(value);
  if (text) {
    return text;
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  const record = value as Record<string, unknown>;
  const kind = stringOrNull(record.kind);
  if (!kind) {
    return safeJson(value);
  }
  switch (kind) {
    case "every_interval":
      return typeof record.interval_ms === "number"
        ? `${kind} ${formatScheduleDuration(record.interval_ms)}`
        : kind;
    case "once_after":
      return typeof record.delay_ms === "number"
        ? `${kind} ${formatScheduleDuration(record.delay_ms)}`
        : kind;
    case "every_day_at":
      return [kind, stringOrNull(record.time), stringOrNull(record.timezone)]
        .filter(Boolean)
        .join(" ");
    case "every_week_at":
      return [
        kind,
        formatScheduleWeekdays(record.weekdays),
        stringOrNull(record.time),
        stringOrNull(record.timezone),
      ]
        .filter(Boolean)
        .join(" ");
    case "once_at":
      return [kind, stringOrNull(record.run_at)].filter(Boolean).join(" ");
    default:
      return kind;
  }
}

export function formatScheduleRule(value: unknown) {
  const text = stringOrNull(value);
  if (text) {
    return text;
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  const record = value as Record<string, unknown>;
  const kind = stringOrNull(record.kind);
  switch (kind) {
    case "every_interval":
      return typeof record.interval_ms === "number"
        ? `Every ${formatScheduleDurationWords(record.interval_ms)}`
        : "Every interval";
    case "once_after":
      return typeof record.delay_ms === "number"
        ? `Once after ${formatScheduleDurationWords(record.delay_ms)}`
        : "Once after delay";
    case "every_day_at":
      return ["Daily", stringOrNull(record.time), stringOrNull(record.timezone)]
        .filter(Boolean)
        .join(" ");
    case "every_week_at":
      return [
        "Weekly",
        formatScheduleWeekdays(record.weekdays),
        stringOrNull(record.time),
        stringOrNull(record.timezone),
      ]
        .filter(Boolean)
        .join(" ");
    case "once_at":
      return "Once";
    default:
      return formatScheduleArgument(value);
  }
}

export function buildScheduleOccurrences(
  schedule: unknown,
  options: ScheduleOccurrenceOptions = {},
): ScheduleOccurrence[] {
  if (!schedule || typeof schedule !== "object" || Array.isArray(schedule)) {
    return [];
  }
  const record = schedule as Record<string, unknown>;
  const kind = stringOrNull(record.kind);
  const now = normalizeDate(options.now) ?? new Date();
  const limit = positiveInteger(options.limit) ?? DEFAULT_OCCURRENCE_LIMIT;
  const horizonDays =
    positiveInteger(options.horizonDays) ?? DEFAULT_OCCURRENCE_HORIZON_DAYS;
  const horizonEnd = new Date(now.getTime() + horizonDays * 24 * 60 * 60 * 1000);

  switch (kind) {
    case "every_interval":
      return buildIntervalOccurrences(record, now, horizonEnd, limit, options.nextFireAt);
    case "once_after":
      return oneShotOccurrence(
        parseDate(options.nextFireAt) ??
          (typeof record.delay_ms === "number"
            ? new Date(now.getTime() + record.delay_ms)
            : null),
        now,
        horizonEnd,
      );
    case "once_at":
      return oneShotOccurrence(parseDate(record.run_at), now, horizonEnd);
    case "every_day_at":
      return buildDailyOccurrences(record, now, horizonEnd, limit);
    case "every_week_at":
      return buildWeeklyOccurrences(record, now, horizonEnd, limit);
    default:
      return [];
  }
}

function formatScheduleDuration(timeoutMs: number) {
  if (timeoutMs % 1000 !== 0) {
    return `${timeoutMs}ms`;
  }

  const totalSeconds = timeoutMs / 1000;
  if (totalSeconds % 60 !== 0) {
    return `${totalSeconds}s`;
  }

  const totalMinutes = totalSeconds / 60;
  if (totalMinutes % 60 !== 0) {
    return `${totalMinutes}m`;
  }

  const totalHours = totalMinutes / 60;
  return `${totalHours}h`;
}

function formatScheduleDurationWords(timeoutMs: number) {
  if (timeoutMs % 1000 !== 0) {
    return `${timeoutMs} milliseconds`;
  }

  const totalSeconds = timeoutMs / 1000;
  if (totalSeconds % 60 !== 0) {
    return plural(totalSeconds, "second");
  }

  const totalMinutes = totalSeconds / 60;
  if (totalMinutes % 60 !== 0) {
    return plural(totalMinutes, "minute");
  }

  const totalHours = totalMinutes / 60;
  if (totalHours % 24 !== 0) {
    return plural(totalHours, "hour");
  }

  return plural(totalHours / 24, "day");
}

function plural(value: number, unit: string) {
  return `${value} ${unit}${value === 1 ? "" : "s"}`;
}

function formatScheduleWeekdays(value: unknown) {
  if (!Array.isArray(value)) {
    return null;
  }
  const weekdays = value.map(stringOrNull).filter(Boolean);
  return weekdays.length > 0 ? weekdays.join(",") : null;
}

function buildIntervalOccurrences(
  record: Record<string, unknown>,
  now: Date,
  horizonEnd: Date,
  limit: number,
  nextFireAt: string | null | undefined,
) {
  if (typeof record.interval_ms !== "number" || record.interval_ms <= 0) {
    return [];
  }
  let next =
    parseDate(nextFireAt) ?? new Date(now.getTime() + record.interval_ms);
  if (next.getTime() <= now.getTime()) {
    const elapsed = now.getTime() - next.getTime();
    const skippedIntervals = Math.floor(elapsed / record.interval_ms) + 1;
    next = new Date(next.getTime() + skippedIntervals * record.interval_ms);
  }

  const occurrences: ScheduleOccurrence[] = [];
  while (
    occurrences.length < limit &&
    next.getTime() <= horizonEnd.getTime()
  ) {
    occurrences.push({ startsAt: next.toISOString() });
    next = new Date(next.getTime() + record.interval_ms);
  }
  return occurrences;
}

function oneShotOccurrence(
  date: Date | null,
  now: Date,
  horizonEnd: Date,
) {
  if (
    !date ||
    date.getTime() <= now.getTime() ||
    date.getTime() > horizonEnd.getTime()
  ) {
    return [];
  }
  return [{ startsAt: date.toISOString() }];
}

function buildDailyOccurrences(
  record: Record<string, unknown>,
  now: Date,
  horizonEnd: Date,
  limit: number,
) {
  const time = parseClockTime(record.time);
  if (!time) {
    return [];
  }
  const timezone = stringOrNull(record.timezone);
  const starts = zonedDateStarts(now, horizonEnd, timezone);
  return starts
    .map((start) =>
      dateFromZonedParts(
        start.year,
        start.month,
        start.day,
        time.hour,
        time.minute,
        time.second,
        timezone,
      ),
    )
    .filter((date) => date.getTime() > now.getTime() && date.getTime() <= horizonEnd.getTime())
    .sort(compareDates)
    .slice(0, limit)
    .map((date) => ({ startsAt: date.toISOString() }));
}

function buildWeeklyOccurrences(
  record: Record<string, unknown>,
  now: Date,
  horizonEnd: Date,
  limit: number,
) {
  const time = parseClockTime(record.time);
  const weekdays = parseWeekdays(record.weekdays);
  if (!time || weekdays.length === 0) {
    return [];
  }
  const timezone = stringOrNull(record.timezone);
  const starts = zonedDateStarts(now, horizonEnd, timezone);
  return starts
    .filter((start) => weekdays.includes(start.weekday))
    .map((start) =>
      dateFromZonedParts(
        start.year,
        start.month,
        start.day,
        time.hour,
        time.minute,
        time.second,
        timezone,
      ),
    )
    .filter((date) => date.getTime() > now.getTime() && date.getTime() <= horizonEnd.getTime())
    .sort(compareDates)
    .slice(0, limit)
    .map((date) => ({ startsAt: date.toISOString() }));
}

function parseClockTime(value: unknown) {
  const text = stringOrNull(value);
  const match = text?.match(/^(\d{1,2}):(\d{2})(?::(\d{2}))?$/);
  if (!match) {
    return null;
  }
  const hour = Number(match[1]);
  const minute = Number(match[2]);
  const second = match[3] ? Number(match[3]) : 0;
  if (
    hour < 0 ||
    hour > 23 ||
    minute < 0 ||
    minute > 59 ||
    second < 0 ||
    second > 59
  ) {
    return null;
  }
  return { hour, minute, second };
}

function parseWeekdays(value: unknown) {
  if (!Array.isArray(value)) {
    return [];
  }
  return value
    .map(stringOrNull)
    .map((weekday) => (weekday ? WEEKDAY_INDEX[weekday.toLowerCase()] : undefined))
    .filter((weekday): weekday is number => typeof weekday === "number");
}

function zonedDateStarts(now: Date, horizonEnd: Date, timezone: string | null) {
  const start = datePartsForZone(now, timezone);
  const totalDays = Math.ceil(
    (horizonEnd.getTime() - now.getTime()) / (24 * 60 * 60 * 1000),
  );
  const dates: Array<{ year: number; month: number; day: number; weekday: number }> = [];
  const startUtc = Date.UTC(start.year, start.month - 1, start.day);
  for (let offset = 0; offset <= totalDays; offset += 1) {
    const date = new Date(startUtc + offset * 24 * 60 * 60 * 1000);
    dates.push({
      year: date.getUTCFullYear(),
      month: date.getUTCMonth() + 1,
      day: date.getUTCDate(),
      weekday: date.getUTCDay(),
    });
  }
  return dates;
}

function dateFromZonedParts(
  year: number,
  month: number,
  day: number,
  hour: number,
  minute: number,
  second: number,
  timezone: string | null,
) {
  if (!timezone) {
    return new Date(year, month - 1, day, hour, minute, second);
  }
  const utcGuess = Date.UTC(year, month - 1, day, hour, minute, second);
  const offset = timeZoneOffsetMs(new Date(utcGuess), timezone);
  const firstPass = new Date(utcGuess - offset);
  const refinedOffset = timeZoneOffsetMs(firstPass, timezone);
  return new Date(utcGuess - refinedOffset);
}

function datePartsForZone(date: Date, timezone: string | null) {
  if (!timezone) {
    return {
      year: date.getFullYear(),
      month: date.getMonth() + 1,
      day: date.getDate(),
    };
  }
  const parts = intlParts(date, timezone);
  return {
    year: parts.year,
    month: parts.month,
    day: parts.day,
  };
}

function timeZoneOffsetMs(date: Date, timezone: string) {
  const parts = intlParts(date, timezone);
  const asUtc = Date.UTC(
    parts.year,
    parts.month - 1,
    parts.day,
    parts.hour,
    parts.minute,
    parts.second,
  );
  return asUtc - date.getTime();
}

function intlParts(date: Date, timezone: string) {
  try {
    const formatter = new Intl.DateTimeFormat("en-US", {
      timeZone: timezone,
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      hourCycle: "h23",
    });
    const parts = Object.fromEntries(
      formatter
        .formatToParts(date)
        .filter((part) => part.type !== "literal")
        .map((part) => [part.type, Number(part.value)]),
    );
    return {
      year: parts.year,
      month: parts.month,
      day: parts.day,
      hour: parts.hour,
      minute: parts.minute,
      second: parts.second,
    };
  } catch {
    return {
      year: date.getFullYear(),
      month: date.getMonth() + 1,
      day: date.getDate(),
      hour: date.getHours(),
      minute: date.getMinutes(),
      second: date.getSeconds(),
    };
  }
}

function compareDates(left: Date, right: Date) {
  return left.getTime() - right.getTime();
}

function parseDate(value: unknown) {
  const text = stringOrNull(value);
  if (!text) {
    return null;
  }
  const date = new Date(text);
  return Number.isNaN(date.getTime()) ? null : date;
}

function normalizeDate(value: Date | string | number | undefined) {
  if (value instanceof Date) {
    return Number.isNaN(value.getTime()) ? null : value;
  }
  if (typeof value === "string" || typeof value === "number") {
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? null : date;
  }
  return null;
}

function positiveInteger(value: number | undefined) {
  return typeof value === "number" && Number.isFinite(value) && value > 0
    ? Math.floor(value)
    : null;
}

function safeJson(value: unknown) {
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function stringOrNull(value: unknown) {
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}
