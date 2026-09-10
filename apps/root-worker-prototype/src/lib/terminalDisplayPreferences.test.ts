import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_TERMINAL_DISPLAY_PREFERENCES,
  readTerminalDisplayPreferences,
  resetTerminalDisplayPreferences,
  storeTerminalDisplayPreferences,
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
      fontFamily: "system",
      fontSize: 12,
      lineHeight: 1.18,
    },
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
