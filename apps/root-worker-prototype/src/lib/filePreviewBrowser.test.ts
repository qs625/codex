import test from "node:test";
import assert from "node:assert/strict";

import type { FilePreview } from "../types";
import {
  filePreviewOpenInBrowserActionVisible,
  isHtmlFilePreview,
  localPathToFileUrl,
} from "./filePreviewBrowser";

function makePreview(overrides: Partial<FilePreview> = {}): FilePreview {
  return {
    path: "/tmp/index.html",
    displayPath: "index.html",
    content: "",
    language: "html",
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
    pdf: null,
    ...overrides,
  };
}

test("detects HTML file previews from language and file extensions", () => {
  assert.equal(isHtmlFilePreview(makePreview({ language: "HTML" })), true);
  assert.equal(
    isHtmlFilePreview(
      makePreview({
        path: "/tmp/share.htm",
        displayPath: "share.htm",
        language: "plaintext",
      }),
    ),
    true,
  );
  assert.equal(
    isHtmlFilePreview(
      makePreview({
        path: "/tmp/share.md",
        displayPath: "share.md",
        language: "markdown",
      }),
    ),
    false,
  );
});

test("shows the Browser action only for loaded preview-mode HTML files", () => {
  const preview = makePreview();

  assert.equal(
    filePreviewOpenInBrowserActionVisible({
      filePanelView: "preview",
      preview,
      previewError: null,
      previewLoading: false,
    }),
    true,
  );
  assert.equal(
    filePreviewOpenInBrowserActionVisible({
      filePanelView: "tree",
      preview,
      previewError: null,
      previewLoading: false,
    }),
    false,
  );
  assert.equal(
    filePreviewOpenInBrowserActionVisible({
      filePanelView: "preview",
      preview,
      previewError: "Failed",
      previewLoading: false,
    }),
    false,
  );
  assert.equal(
    filePreviewOpenInBrowserActionVisible({
      filePanelView: "preview",
      preview,
      previewError: null,
      previewLoading: true,
    }),
    false,
  );
});

test("converts local paths to encoded file URLs", () => {
  assert.equal(
    localPathToFileUrl("/tmp/share page #1?.html"),
    "file:///tmp/share%20page%20%231%3F.html",
  );
  assert.equal(
    localPathToFileUrl("C:\\Users\\Ada\\share page #1.html"),
    "file:///C:/Users/Ada/share%20page%20%231.html",
  );
  assert.equal(
    localPathToFileUrl("\\\\server\\share\\share page.html"),
    "file://server/share/share%20page.html",
  );
});
