import test from "node:test";
import assert from "node:assert/strict";

import {
  getProjectFilePreview,
  rememberProjectFilePreview,
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
