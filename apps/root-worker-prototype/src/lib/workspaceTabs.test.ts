import test from "node:test";
import assert from "node:assert/strict";

import {
  applyStoredWorkspaceTabOrder,
  closeWorkspaceTabById,
  mergeWorkspaceTabOrder,
  readStoredWorkspaceTabOrder,
  reorderWorkspaceTabs,
  sanitizeWorkspaceTabs,
  storeWorkspaceTabOrder,
  upsertWorkspaceTab,
  type WorkspaceObjectTab,
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

function tab(id: string, kind: WorkspaceObjectTab["kind"]): WorkspaceObjectTab {
  return {
    id,
    kind,
    title: id,
  };
}

test("reorderWorkspaceTabs moves object tabs before or after the target", () => {
  const tabs = [
    tab("conversation:root", "conversation"),
    tab("file:root:/tmp/a.ts", "file"),
    tab("file:root:/tmp/b.ts", "file"),
  ];

  assert.deepEqual(
    reorderWorkspaceTabs(tabs, "file:root:/tmp/b.ts", "file:root:/tmp/a.ts")
      .map((item) => item.id),
    ["conversation:root", "file:root:/tmp/b.ts", "file:root:/tmp/a.ts"],
  );
  assert.deepEqual(
    reorderWorkspaceTabs(
      tabs,
      "file:root:/tmp/a.ts",
      "file:root:/tmp/b.ts",
      "after",
    ).map((item) => item.id),
    ["conversation:root", "file:root:/tmp/b.ts", "file:root:/tmp/a.ts"],
  );
});

test("upsertWorkspaceTab updates an existing object tab without duplicating it", () => {
  const tabs = upsertWorkspaceTab(
    [tab("conversation:root", "conversation")],
    {
      id: "conversation:root",
      kind: "conversation",
      title: "/root",
      subtitle: "Complete",
      threadId: "root",
    },
  );

  assert.deepEqual(tabs, [
    {
      id: "conversation:root",
      kind: "conversation",
      title: "/root",
      subtitle: "Complete",
      threadId: "root",
    },
  ]);
});

test("closeWorkspaceTabById removes only the central workspace tab", () => {
  const tabs = [
    tab("conversation:root", "conversation"),
    tab("file:root:/tmp/a.ts", "file"),
    tab("browser:main", "browser"),
    tab("terminal:root", "terminal"),
  ];

  assert.deepEqual(
    closeWorkspaceTabById(tabs, "file:root:/tmp/a.ts").map((item) => item.id),
    ["conversation:root", "browser:main", "terminal:root"],
  );
  assert.deepEqual(
    closeWorkspaceTabById(tabs, "missing").map((item) => item.id),
    tabs.map((item) => item.id),
  );
});

test("sanitizeWorkspaceTabs drops invalid and duplicate object tabs", () => {
  assert.deepEqual(
    sanitizeWorkspaceTabs([
      tab("conversation:root", "conversation"),
      tab("conversation:root", "conversation"),
      { id: "", kind: "file", title: "bad" } as WorkspaceObjectTab,
      { id: "surface:files", kind: "files", title: "Files" } as never,
    ]).map((item) => item.id),
    ["conversation:root"],
  );
});

test("stored workspace object tab order is best effort", () => {
  const storage = makeStorage(
    JSON.stringify(["file:root:/tmp/b.ts", 1, "conversation:root"]),
  );

  assert.deepEqual(readStoredWorkspaceTabOrder(storage), [
    "file:root:/tmp/b.ts",
    "conversation:root",
  ]);

  const ordered = applyStoredWorkspaceTabOrder(
    [
      tab("conversation:root", "conversation"),
      tab("file:root:/tmp/a.ts", "file"),
      tab("file:root:/tmp/b.ts", "file"),
    ],
    readStoredWorkspaceTabOrder(storage),
  );
  assert.deepEqual(ordered.map((item) => item.id), [
    "file:root:/tmp/b.ts",
    "conversation:root",
    "file:root:/tmp/a.ts",
  ]);

  storeWorkspaceTabOrder(ordered, storage);

  assert.deepEqual(JSON.parse(storage.read() ?? "[]"), [
    "file:root:/tmp/b.ts",
    "conversation:root",
    "file:root:/tmp/a.ts",
  ]);
});

test("workspace tab order storage preserves ids that are not live yet", () => {
  const storage = makeStorage();
  const liveTabs = [tab("conversation:a", "conversation")];
  const previousOrder = ["conversation:b", "conversation:a"];

  assert.deepEqual(mergeWorkspaceTabOrder(liveTabs, previousOrder), [
    "conversation:b",
    "conversation:a",
  ]);
  assert.deepEqual(storeWorkspaceTabOrder(liveTabs, storage, previousOrder), [
    "conversation:b",
    "conversation:a",
  ]);
  assert.deepEqual(JSON.parse(storage.read() ?? "[]"), [
    "conversation:b",
    "conversation:a",
  ]);
});
