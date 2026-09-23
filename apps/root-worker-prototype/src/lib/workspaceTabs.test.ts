import test from "node:test";
import assert from "node:assert/strict";

import {
  readStoredWorkspaceTabOrder,
  reorderWorkspaceTabs,
  storeWorkspaceTabOrder,
} from "./workspaceTabs";

function makeStorage(initialValue: string | null = null) {
  let value = initialValue;
  return {
    getItem: () => value,
    setItem: (_key: string, nextValue: string) => {
      value = nextValue;
    },
    read: () => value,
  };
}

test("reorderWorkspaceTabs moves a dragged tab before the target", () => {
  assert.deepEqual(
    reorderWorkspaceTabs(
      ["conversation", "files", "terminal", "browser"],
      "browser",
      "files",
    ),
    ["conversation", "browser", "files", "terminal"],
  );
});

test("reorderWorkspaceTabs moves a dragged tab after the target", () => {
  assert.deepEqual(
    reorderWorkspaceTabs(
      ["conversation", "files", "terminal", "browser"],
      "files",
      "terminal",
      "after",
    ),
    ["conversation", "terminal", "files", "browser"],
  );
  assert.deepEqual(
    reorderWorkspaceTabs(
      ["conversation", "files", "terminal", "browser"],
      "terminal",
      "browser",
      "after",
    ),
    ["conversation", "files", "browser", "terminal"],
  );
});

test("stored workspace tab order is sanitized and completed", () => {
  const storage = makeStorage(
    JSON.stringify(["browser", "unknown", "browser", "conversation"]),
  );

  assert.deepEqual(readStoredWorkspaceTabOrder(storage), [
    "browser",
    "conversation",
    "files",
    "terminal",
  ]);
});

test("workspace tab order storage is best effort", () => {
  const storage = makeStorage();

  storeWorkspaceTabOrder(
    ["terminal", "conversation", "files", "browser"],
    storage,
  );

  assert.deepEqual(JSON.parse(storage.read() ?? "[]"), [
    "terminal",
    "conversation",
    "files",
    "browser",
  ]);
});
