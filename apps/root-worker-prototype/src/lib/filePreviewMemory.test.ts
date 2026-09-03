import test from "node:test";
import assert from "node:assert/strict";

import {
  getProjectFilePreview,
  rememberProjectFilePreview,
  rememberSavedProjectFilePreview,
  shouldRestoreProjectFilePreview,
  type FilePreviewMemoryByRootId,
} from "./filePreviewMemory";
import type { FilePreview } from "../types";

function makePreview(path: string): FilePreview {
  return {
    path,
    displayPath: path.split("/").at(-1) ?? path,
    content: "",
    language: "typescript",
    line: null,
    column: null,
    lsp: {
      enabled: false,
      languageId: null,
      lspStatus: {
        phase: "plain",
        detail: null,
      },
      serverLabel: null,
      workspaceRoot: null,
      reason: null,
    },
    image: null,
  };
}

test("remembers one file preview per project root", () => {
  const projectOnePreview = makePreview("/work/project-one/file1.ts");
  const projectTwoPreview = makePreview("/work/project-two/file2.ts");

  const memory = rememberProjectFilePreview(
    rememberProjectFilePreview({}, "root-1", projectOnePreview),
    "root-2",
    projectTwoPreview,
  );

  assert.equal(getProjectFilePreview(memory, "root-1"), projectOnePreview);
  assert.equal(getProjectFilePreview(memory, "root-2"), projectTwoPreview);
});

test("leaves preview memory unchanged without a project root", () => {
  const memory: FilePreviewMemoryByRootId = {
    "root-1": makePreview("/work/project-one/file1.ts"),
  };

  assert.equal(
    rememberProjectFilePreview(memory, null, makePreview("/tmp/ignored.ts")),
    memory,
  );
});

test("remembers saved previews only for the current root and path", () => {
  const original = makePreview("/work/project-one/file1.ts");
  const saved = {
    ...original,
    content: "saved",
  };
  const otherRootPreview = makePreview("/work/project-two/file2.ts");
  const memory: FilePreviewMemoryByRootId = {
    "root-1": original,
    "root-2": otherRootPreview,
  };

  assert.deepEqual(
    rememberSavedProjectFilePreview(
      memory,
      "root-2",
      "root-1",
      otherRootPreview,
      saved,
    ),
    memory,
  );
  assert.deepEqual(
    rememberSavedProjectFilePreview(
      memory,
      "root-1",
      "root-1",
      makePreview("/work/project-one/other.ts"),
      saved,
    ),
    memory,
  );

  const updated = rememberSavedProjectFilePreview(
    memory,
    "root-1",
    "root-1",
    original,
    saved,
  );

  assert.equal(updated["root-1"], saved);
  assert.equal(updated["root-2"], otherRootPreview);
});

test("returns no preview for project roots without memory", () => {
  assert.equal(getProjectFilePreview({}, "root-missing"), null);
  assert.equal(getProjectFilePreview({}, null), null);
});

test("restores project previews only when the file preview pane is visible", () => {
  assert.equal(shouldRestoreProjectFilePreview("preview", "preview"), true);
  assert.equal(shouldRestoreProjectFilePreview("preview", "tree"), false);
  assert.equal(shouldRestoreProjectFilePreview("skills", "preview"), false);
  assert.equal(shouldRestoreProjectFilePreview("git", "preview"), false);
});
