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
  return ScheduleDisplayValue.from(value)?.argumentText() ?? null;
}

export function formatScheduleRule(value: unknown) {
  return ScheduleDisplayValue.from(value)?.ruleText() ?? null;
}

export function buildScheduleOccurrences(
  schedule: unknown,
  options: ScheduleOccurrenceOptions = {},
): ScheduleOccurrence[] {
  return ScheduleOccurrenceProjection.from(schedule, options).occurrences();
}

class ScheduleDisplayValue {
  private constructor(
    private readonly source: unknown,
    private readonly record: Record<string, unknown> | null,
    private readonly text: string | null,
  ) {}

  static from(value: unknown) {
    const text = stringOrNull(value);
    if (text) {
      return new ScheduleDisplayValue(value, /*record*/ null, text);
    }
    if (!value || typeof value !== "object" || Array.isArray(value)) {
      return null;
    }
    return new ScheduleDisplayValue(
      value,
      value as Record<string, unknown>,
      /*text*/ null,
    );
  }

  argumentText() {
    if (this.text) {
      return this.text;
    }
    const record = this.record;
    if (!record) {
      return null;
    }
    const kind = stringOrNull(record.kind);
    if (!kind) {
      return safeJson(this.source);
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

  ruleText() {
    if (this.text) {
      return this.text;
    }
    const record = this.record;
    if (!record) {
      return null;
    }
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
        return [
          "Daily",
          stringOrNull(record.time),
          stringOrNull(record.timezone),
        ]
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
        return this.argumentText();
    }
  }
}

class ScheduleOccurrenceProjection {
  private readonly now: Date;
  private readonly horizonEnd: Date;
  private readonly limit: number;
  private readonly nextFireAt: string | null | undefined;

  private constructor(
    private readonly record: Record<string, unknown> | null,
    options: ScheduleOccurrenceOptions,
  ) {
    this.now = normalizeDate(options.now) ?? new Date();
    this.limit = positiveInteger(options.limit) ?? DEFAULT_OCCURRENCE_LIMIT;
    const horizonDays =
      positiveInteger(options.horizonDays) ?? DEFAULT_OCCURRENCE_HORIZON_DAYS;
    this.horizonEnd = new Date(
      this.now.getTime() + horizonDays * 24 * 60 * 60 * 1000,
    );
    this.nextFireAt = options.nextFireAt;
  }

  static from(schedule: unknown, options: ScheduleOccurrenceOptions) {
    if (!schedule || typeof schedule !== "object" || Array.isArray(schedule)) {
      return new ScheduleOccurrenceProjection(/*record*/ null, options);
    }
    return new ScheduleOccurrenceProjection(
      schedule as Record<string, unknown>,
      options,
    );
  }

  occurrences() {
    const kind = stringOrNull(this.record?.kind);
    switch (kind) {
      case "every_interval":
        return this.intervalOccurrences();
      case "once_after":
        return this.onceAfterOccurrence();
      case "once_at":
        return this.oneShotOccurrence(parseDate(this.record?.run_at));
      case "every_day_at":
        return this.dailyOccurrences();
      case "every_week_at":
        return this.weeklyOccurrences();
      default:
        return [];
    }
  }

  private intervalOccurrences() {
    const intervalMs = this.numberField("interval_ms");
    if (intervalMs === null || intervalMs <= 0) {
      return [];
    }

    let next =
      parseDate(this.nextFireAt) ?? new Date(this.now.getTime() + intervalMs);
    if (next.getTime() <= this.now.getTime()) {
      const elapsed = this.now.getTime() - next.getTime();
      const skippedIntervals = Math.floor(elapsed / intervalMs) + 1;
      next = new Date(next.getTime() + skippedIntervals * intervalMs);
    }

    return this.collectForward(
      next,
      (date) => new Date(date.getTime() + intervalMs),
    );
  }

  private onceAfterOccurrence() {
    return this.oneShotOccurrence(
      parseDate(this.nextFireAt) ??
        this.numberField(
          "delay_ms",
          (delayMs) => new Date(this.now.getTime() + delayMs),
        ),
    );
  }

  private oneShotOccurrence(date: Date | null) {
    if (
      !date ||
      date.getTime() <= this.now.getTime() ||
      date.getTime() > this.horizonEnd.getTime()
    ) {
      return [];
    }
    return [{ startsAt: date.toISOString() }];
  }

  private dailyOccurrences() {
    const time = parseClockTime(this.record?.time);
    if (!time) {
      return [];
    }
    const timezone = stringOrNull(this.record?.timezone);
    return this.zonedOccurrences(time, timezone);
  }

  private weeklyOccurrences() {
    const time = parseClockTime(this.record?.time);
    const weekdays = parseWeekdays(this.record?.weekdays);
    if (!time || weekdays.length === 0) {
      return [];
    }
    const timezone = stringOrNull(this.record?.timezone);
    return this.zonedOccurrences(time, timezone, (start) =>
      weekdays.includes(start.weekday),
    );
  }

  private zonedOccurrences(
    time: ClockTime,
    timezone: string | null,
    includeStart: (start: ZonedDateStart) => boolean = () => true,
  ) {
    return zonedDateStarts(this.now, this.horizonEnd, timezone)
      .filter(includeStart)
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
      .filter((date) => this.isWithinOccurrenceWindow(date))
      .sort(compareDates)
      .slice(0, this.limit)
      .map((date) => ({ startsAt: date.toISOString() }));
  }

  private collectForward(
    first: Date,
    nextDate: (date: Date) => Date,
  ): ScheduleOccurrence[] {
    const occurrences: ScheduleOccurrence[] = [];
    let next = first;
    while (
      occurrences.length < this.limit &&
      next.getTime() <= this.horizonEnd.getTime()
    ) {
      occurrences.push({ startsAt: next.toISOString() });
      next = nextDate(next);
    }
    return occurrences;
  }

  private isWithinOccurrenceWindow(date: Date) {
    return (
      date.getTime() > this.now.getTime() &&
      date.getTime() <= this.horizonEnd.getTime()
    );
  }

  private numberField(field: string): number | null;
  private numberField<T>(field: string, map: (value: number) => T): T | null;
  private numberField<T>(field: string, map?: (value: number) => T) {
    const value = this.record?.[field];
    if (typeof value !== "number") {
      return null;
    }
    return map ? map(value) : value;
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

type ClockTime = {
  hour: number;
  minute: number;
  second: number;
};

type ZonedDateStart = {
  year: number;
  month: number;
  day: number;
  weekday: number;
};

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
    .map((weekday) =>
      weekday ? WEEKDAY_INDEX[weekday.toLowerCase()] : undefined,
    )
    .filter((weekday): weekday is number => typeof weekday === "number");
}

function zonedDateStarts(now: Date, horizonEnd: Date, timezone: string | null) {
  const start = datePartsForZone(now, timezone);
  const totalDays = Math.ceil(
    (horizonEnd.getTime() - now.getTime()) / (24 * 60 * 60 * 1000),
  );
  const dates: Array<{
    year: number;
    month: number;
    day: number;
    weekday: number;
  }> = [];
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
