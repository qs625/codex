const TERMINAL_DISPLAY_PREFERENCES_STORAGE_KEY =
  "root-worker-prototype:terminal-display-preferences";

const NERD_FONT_FAMILY_STACK =
  '"CaskaydiaCove Nerd Font Mono", "CaskaydiaMono Nerd Font Mono", "CaskaydiaCove NFM", "CaskaydiaMono NFM", "Cascadia Code NF", "Symbols Nerd Font", "JetBrainsMono Nerd Font Mono"';

export const TERMINAL_FONT_FAMILIES = [
  {
    id: "nerd",
    label: "Nerd Font",
    value: `${NERD_FONT_FAMILY_STACK}, "JetBrains Mono", "Cascadia Code", "SFMono-Regular", Menlo, monospace`,
  },
  {
    id: "system",
    label: "System monospace",
    value: `${NERD_FONT_FAMILY_STACK}, "SFMono-Regular", "Cascadia Code", "Liberation Mono", Menlo, monospace`,
  },
  {
    id: "cascadia",
    label: "Cascadia Code",
    value:
      '"Cascadia Code NF", "CaskaydiaCove Nerd Font Mono", "CaskaydiaMono Nerd Font Mono", "Symbols Nerd Font", "Cascadia Code", "SFMono-Regular", "Liberation Mono", Menlo, monospace',
  },
  {
    id: "jetbrains",
    label: "JetBrains Mono",
    value:
      '"JetBrainsMono Nerd Font Mono", "Symbols Nerd Font", "JetBrains Mono", "Cascadia Code", "SFMono-Regular", Menlo, monospace',
  },
] as const;

export type TerminalFontFamilyId =
  (typeof TERMINAL_FONT_FAMILIES)[number]["id"];

export type TerminalDisplayPreferences = {
  fontFamily: TerminalFontFamilyId;
  fontSize: number;
  lineHeight: number;
};

type TerminalDisplayPreferencePatch = Partial<TerminalDisplayPreferences>;
type PreferenceStorage = Pick<Storage, "getItem" | "setItem" | "removeItem">;
type TerminalPreferenceNumberKey = "fontSize" | "lineHeight";
type TerminalPreferenceNumberMode = "interactive" | "stored";
type TerminalPreferenceNumberRule = {
  key: TerminalPreferenceNumberKey;
  fallback: number;
  min: number;
  max: number;
  step: number;
};

export const DEFAULT_TERMINAL_DISPLAY_PREFERENCES: TerminalDisplayPreferences =
  {
    fontFamily: "nerd",
    fontSize: 12,
    lineHeight: 1.18,
  };

const FONT_SIZE_MIN = 10;
const FONT_SIZE_MAX = 22;
const LINE_HEIGHT_MIN = 1;
const LINE_HEIGHT_MAX = 2;

const TERMINAL_PREFERENCE_NUMBER_RULES: Record<
  TerminalPreferenceNumberKey,
  TerminalPreferenceNumberRule
> = {
  fontSize: {
    key: "fontSize",
    fallback: DEFAULT_TERMINAL_DISPLAY_PREFERENCES.fontSize,
    min: FONT_SIZE_MIN,
    max: FONT_SIZE_MAX,
    step: 1,
  },
  lineHeight: {
    key: "lineHeight",
    fallback: DEFAULT_TERMINAL_DISPLAY_PREFERENCES.lineHeight,
    min: LINE_HEIGHT_MIN,
    max: LINE_HEIGHT_MAX,
    step: 0.05,
  },
};

class TerminalDisplayPreferenceSource {
  private constructor(
    private readonly candidate: Partial<TerminalDisplayPreferences>,
  ) {}

  static from(value: unknown) {
    return new TerminalDisplayPreferenceSource(
      value && typeof value === "object"
        ? (value as Partial<TerminalDisplayPreferences>)
        : {},
    );
  }

  project(mode: TerminalPreferenceNumberMode): TerminalDisplayPreferences {
    return {
      fontFamily: this.fontFamily(),
      fontSize: this.number(TERMINAL_PREFERENCE_NUMBER_RULES.fontSize, mode),
      lineHeight: this.number(
        TERMINAL_PREFERENCE_NUMBER_RULES.lineHeight,
        mode,
      ),
    };
  }

  private fontFamily(): TerminalFontFamilyId {
    return isTerminalFontFamily(this.candidate.fontFamily)
      ? this.candidate.fontFamily
      : DEFAULT_TERMINAL_DISPLAY_PREFERENCES.fontFamily;
  }

  private number(
    rule: TerminalPreferenceNumberRule,
    mode: TerminalPreferenceNumberMode,
  ): number {
    const value = this.candidate[rule.key];
    return mode === "stored"
      ? persistedNumber(value, rule)
      : interactiveNumber(value, rule);
  }
}

export function readTerminalDisplayPreferences(
  storage: PreferenceStorage | null | undefined = getLocalStorage(),
): TerminalDisplayPreferences {
  if (!storage) {
    return DEFAULT_TERMINAL_DISPLAY_PREFERENCES;
  }

  try {
    const stored = storage.getItem(TERMINAL_DISPLAY_PREFERENCES_STORAGE_KEY);
    if (!stored) {
      return DEFAULT_TERMINAL_DISPLAY_PREFERENCES;
    }
    return readPersistedTerminalDisplayPreferences(JSON.parse(stored));
  } catch {
    return DEFAULT_TERMINAL_DISPLAY_PREFERENCES;
  }
}

export function updateTerminalDisplayPreferences(
  current: TerminalDisplayPreferences,
  patch: TerminalDisplayPreferencePatch,
): TerminalDisplayPreferences {
  return normalizeTerminalDisplayPreferences({ ...current, ...patch });
}

export function resetTerminalDisplayPreferences(
  storage: PreferenceStorage | null | undefined = getLocalStorage(),
): TerminalDisplayPreferences {
  if (storage) {
    try {
      storage.removeItem(TERMINAL_DISPLAY_PREFERENCES_STORAGE_KEY);
    } catch {
      // Keep the in-memory reset even when persistence is unavailable.
    }
  }
  return DEFAULT_TERMINAL_DISPLAY_PREFERENCES;
}

export function storeTerminalDisplayPreferences(
  preferences: TerminalDisplayPreferences,
  storage: PreferenceStorage | null | undefined = getLocalStorage(),
) {
  if (!storage) {
    return;
  }

  try {
    storage.setItem(
      TERMINAL_DISPLAY_PREFERENCES_STORAGE_KEY,
      JSON.stringify(normalizeTerminalDisplayPreferences(preferences)),
    );
  } catch {
    // The current display update must not depend on storage availability.
  }
}

export function terminalFontFamilyValue(
  fontFamily: TerminalFontFamilyId,
): string {
  return (
    TERMINAL_FONT_FAMILIES.find((option) => option.id === fontFamily)?.value ??
    TERMINAL_FONT_FAMILIES[0].value
  );
}

function normalizeTerminalDisplayPreferences(
  value: unknown,
): TerminalDisplayPreferences {
  return TerminalDisplayPreferenceSource.from(value).project("interactive");
}

function readPersistedTerminalDisplayPreferences(
  value: unknown,
): TerminalDisplayPreferences {
  return TerminalDisplayPreferenceSource.from(value).project("stored");
}

function isTerminalFontFamily(value: unknown): value is TerminalFontFamilyId {
  return TERMINAL_FONT_FAMILIES.some((option) => option.id === value);
}

function isStoredNumberInRange(
  value: unknown,
  min: number,
  max: number,
): value is number {
  return (
    typeof value === "number" &&
    Number.isFinite(value) &&
    value >= min &&
    value <= max
  );
}

function persistedNumber(
  value: unknown,
  { fallback, min, max }: TerminalPreferenceNumberRule,
) {
  return isStoredNumberInRange(value, min, max) ? value : fallback;
}

function interactiveNumber(
  value: unknown,
  { fallback, min, max, step }: TerminalPreferenceNumberRule,
) {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return fallback;
  }
  const clamped = Math.min(max, Math.max(min, value));
  return Math.round(clamped / step) * step;
}

function getLocalStorage(): PreferenceStorage | null {
  if (typeof window === "undefined") {
    return null;
  }

  try {
    return window.localStorage;
  } catch {
    return null;
  }
}
