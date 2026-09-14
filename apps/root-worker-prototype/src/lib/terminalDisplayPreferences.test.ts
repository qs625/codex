import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_TERMINAL_DISPLAY_PREFERENCES,
  TERMINAL_FONT_FAMILIES,
  readTerminalDisplayPreferences,
  resetTerminalDisplayPreferences,
  storeTerminalDisplayPreferences,
  terminalFontFamilyValue,
  updateTerminalDisplayPreferences,
} from "./terminalDisplayPreferences";

function makeStorage(initialValue: string | null = null) {
  let value = initialValue;
  return {
    getItem(_key: string) {
      return value;
    },
    setItem(_key: string, next: string) {
      value = next;
    },
    removeItem(_key: string) {
      value = null;
    },
  };
}

test("terminal display preferences fall back for missing, malformed, and invalid storage", () => {
  assert.deepEqual(
    readTerminalDisplayPreferences(makeStorage()),
    DEFAULT_TERMINAL_DISPLAY_PREFERENCES,
  );
  assert.deepEqual(
    readTerminalDisplayPreferences(makeStorage("{")),
    DEFAULT_TERMINAL_DISPLAY_PREFERENCES,
  );
  assert.deepEqual(
    readTerminalDisplayPreferences(
      makeStorage(
        JSON.stringify({
          fontFamily: "unknown",
          fontSize: 100,
          lineHeight: 0,
        }),
      ),
    ),
    {
      fontFamily: DEFAULT_TERMINAL_DISPLAY_PREFERENCES.fontFamily,
      fontSize: 12,
      lineHeight: 1.18,
    },
  );
});

test("terminal display preferences default to a Nerd Font-friendly stack", () => {
  const defaultStack = terminalFontFamilyValue(
    DEFAULT_TERMINAL_DISPLAY_PREFERENCES.fontFamily,
  );
  const optionLabels = TERMINAL_FONT_FAMILIES.map((option) => option.label);

  assert.equal(DEFAULT_TERMINAL_DISPLAY_PREFERENCES.fontFamily, "nerd");
  assert.ok(optionLabels.includes("Nerd Font"));
  assert.match(defaultStack, /"CaskaydiaCove Nerd Font Mono"/);
  assert.match(defaultStack, /"Cascadia Code NF"/);
  assert.match(defaultStack, /"Symbols Nerd Font"/);
  assert.match(defaultStack, /"Cascadia Code"/);
  assert.match(defaultStack, /monospace/);
});

test("terminal display preferences keep legacy font ids valid", () => {
  for (const fontFamily of ["system", "cascadia", "jetbrains"] as const) {
    assert.equal(
      readTerminalDisplayPreferences(
        makeStorage(
          JSON.stringify({
            fontFamily,
            fontSize: 12,
            lineHeight: 1.18,
          }),
        ),
      ).fontFamily,
      fontFamily,
    );
  }
});

test("legacy terminal font options also prefer Nerd Font fallbacks", () => {
  assert.match(
    terminalFontFamilyValue("system"),
    /"CaskaydiaCove Nerd Font Mono".*monospace/,
  );
  assert.match(
    terminalFontFamilyValue("cascadia"),
    /"Cascadia Code NF".*"Cascadia Code".*monospace/,
  );
  assert.match(
    terminalFontFamilyValue("jetbrains"),
    /"JetBrainsMono Nerd Font Mono".*"JetBrains Mono".*monospace/,
  );
});

test("terminal display preferences restore valid persisted values", () => {
  const storage = makeStorage();
  storeTerminalDisplayPreferences(
    {
      fontFamily: "jetbrains",
      fontSize: 15,
      lineHeight: 1.35,
    },
    storage,
  );

  assert.deepEqual(readTerminalDisplayPreferences(storage), {
    fontFamily: "jetbrains",
    fontSize: 15,
    lineHeight: 1.35,
  });
});

test("terminal display preferences normalize updates and reset to defaults", () => {
  const storage = makeStorage();
  const updated = updateTerminalDisplayPreferences(
    DEFAULT_TERMINAL_DISPLAY_PREFERENCES,
    {
      fontFamily: "cascadia",
      fontSize: 13,
      lineHeight: 1.25,
    },
  );
  storeTerminalDisplayPreferences(updated, storage);

  assert.deepEqual(resetTerminalDisplayPreferences(storage), DEFAULT_TERMINAL_DISPLAY_PREFERENCES);
  assert.deepEqual(
    readTerminalDisplayPreferences(storage),
    DEFAULT_TERMINAL_DISPLAY_PREFERENCES,
  );
});
