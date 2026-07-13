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

function formatScheduleWeekdays(value: unknown) {
  if (!Array.isArray(value)) {
    return null;
  }
  const weekdays = value.map(stringOrNull).filter(Boolean);
  return weekdays.length > 0 ? weekdays.join(",") : null;
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
