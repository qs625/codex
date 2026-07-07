import test from "node:test";
import assert from "node:assert/strict";

import {
  readStoredRightPanelView,
  storeRightPanelView,
} from "./rightPanelView";

function makeStorage(initialValue: string | null = null) {
  let value = initialValue;
  return {
    getItem(_key: string) {
      return value;
    },
    setItem(_key: string, next: string) {
      value = next;
    },
  };
}

test("readStoredRightPanelView restores a valid stored view", () => {
  assert.equal(readStoredRightPanelView(makeStorage("skills")), "skills");
  assert.equal(readStoredRightPanelView(makeStorage("git")), "git");
});

test("readStoredRightPanelView falls back for invalid stored values", () => {
  assert.equal(readStoredRightPanelView(makeStorage("unknown")), "todo");
  assert.equal(readStoredRightPanelView(makeStorage(null)), "todo");
});

test("readStoredRightPanelView falls back when localStorage is unavailable", () => {
  assert.equal(readStoredRightPanelView(), "todo");
});

test("readStoredRightPanelView falls back when localStorage access throws", () => {
  const originalWindow = Reflect.get(globalThis, "window") as Window | undefined;
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: Object.defineProperty({}, "localStorage", {
      configurable: true,
      get() {
        throw new Error("blocked");
      },
    }),
  });

  try {
    assert.equal(readStoredRightPanelView(), "todo");
  } finally {
    if (originalWindow === undefined) {
      Reflect.deleteProperty(globalThis, "window");
    } else {
      Object.defineProperty(globalThis, "window", {
        configurable: true,
        value: originalWindow,
      });
    }
  }
});

test("storeRightPanelView persists the selected view", () => {
  const storage = makeStorage();

  storeRightPanelView("preview", storage);

  assert.equal(readStoredRightPanelView(storage), "preview");
});
