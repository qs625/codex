export function formatResponseItemDetails(sections: Array<[string, unknown]>) {
  return sections
    .filter(([, value]) => value !== null && value !== undefined && value !== "")
    .map(([label, value]) => `${label}\n${formatUnknownValue(value)}`)
    .join("\n\n");
}

export function parseMaybeJsonString(value: unknown) {
  if (typeof value !== "string") {
    return value;
  }
  try {
    return JSON.parse(value);
  } catch {
    return value;
  }
}

export function formatRawJson(value: unknown) {
  return `Raw item\n${formatUnknownValue(value)}`;
}

export function formatUnknownValue(value: unknown) {
  if (typeof value === "string") {
    return value;
  }
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

export function safeJson(value: unknown) {
  if (typeof value === "string") {
    return value;
  }

  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

export function formatSecondsDuration(totalSeconds: number) {
  if (!Number.isFinite(totalSeconds) || totalSeconds <= 0) {
    return "0s";
  }

  const totalMilliseconds = Math.round(totalSeconds * 1000);
  return formatMillisecondsDuration(totalMilliseconds);
}

export function formatMillisecondsDuration(totalMilliseconds: number) {
  if (!Number.isFinite(totalMilliseconds) || totalMilliseconds <= 0) {
    return "0ms";
  }

  if (totalMilliseconds < 1000) {
    return `${Math.round(totalMilliseconds)}ms`;
  }

  const roundedSeconds = Math.round(totalMilliseconds / 1000);
  if (roundedSeconds >= 60) {
    const minutes = Math.floor(roundedSeconds / 60);
    const remainingSeconds = roundedSeconds % 60;
    if (remainingSeconds === 0) {
      return `${minutes}m`;
    }
    return `${minutes}m ${remainingSeconds}s`;
  }

  const seconds = totalMilliseconds / 1000;
  return `${formatDurationNumber(seconds)}s`;
}

export function formatDurationNumber(value: number) {
  if (Number.isInteger(value)) {
    return value.toString();
  }
  return value.toFixed(2).replace(/\.?0+$/u, "");
}

export function previewInlineText(value: unknown, maxChars: number) {
  const text = stringOrNull(value)?.replace(/\s+/g, " ") ?? null;
  if (!text) {
    return null;
  }
  const chars = Array.from(text);
  return chars.length > maxChars
    ? `${chars.slice(0, maxChars).join("").trimEnd()}…`
    : text;
}

export type BoundedText = {
  text: string;
  truncated: boolean;
  omittedChars: number;
};

export function previewBlockText(
  text: string,
  options: { maxChars: number; maxLines?: number },
): BoundedText {
  const maxChars = Math.max(0, Math.floor(options.maxChars));
  const maxLines =
    options.maxLines === undefined
      ? null
      : Math.max(0, Math.floor(options.maxLines));
  let endIndex = Math.min(text.length, maxChars);

  if (maxLines !== null) {
    const lineEndIndex = indexAfterLineLimit(text, maxLines);
    if (lineEndIndex !== null) {
      endIndex = Math.min(endIndex, lineEndIndex);
    }
  }

  if (endIndex >= text.length) {
    return { text, truncated: false, omittedChars: 0 };
  }

  const preview = text.slice(0, endIndex).trimEnd();
  const omittedChars = text.length - endIndex;
  return {
    text: `${preview}\n[truncated: ${formatOmittedCharacterCount(omittedChars)} characters omitted]`,
    truncated: true,
    omittedChars,
  };
}

function indexAfterLineLimit(text: string, maxLines: number) {
  if (maxLines <= 0) {
    return 0;
  }

  let searchFrom = 0;
  for (let line = 1; line <= maxLines; line += 1) {
    const newlineIndex = text.indexOf("\n", searchFrom);
    if (newlineIndex === -1) {
      return null;
    }
    if (line === maxLines) {
      return newlineIndex;
    }
    searchFrom = newlineIndex + 1;
  }

  return null;
}

function formatOmittedCharacterCount(value: number) {
  return new Intl.NumberFormat("en-US").format(Math.max(0, value));
}

export function stringOrNull(value: unknown) {
  return typeof value === "string" && value.trim().length > 0
    ? value.trim()
    : null;
}

export function numberOrNull(value: unknown) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

export function objectOrNull(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

export function stringOrNumberId(value: unknown) {
  if (typeof value === "number" && Number.isFinite(value)) {
    return String(value);
  }
  return stringOrNull(value);
}

export function stringOrFallback(value: unknown, fallback: string) {
  return stringOrNull(value) ?? fallback;
}
