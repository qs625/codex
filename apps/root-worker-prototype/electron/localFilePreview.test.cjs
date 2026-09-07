const test = require("node:test");
const assert = require("node:assert/strict");

const {
  buildPdfPreview,
  createPdfPreviewUrl,
  pdfMimeForExtension,
} = require("./localFilePreview.cjs");

test("pdfMimeForExtension recognizes PDF extensions case-insensitively", () => {
  assert.equal(pdfMimeForExtension(".pdf"), "application/pdf");
  assert.equal(pdfMimeForExtension(".PDF"), "application/pdf");
  assert.equal(pdfMimeForExtension(".txt"), null);
});

test("createPdfPreviewUrl encodes token and file name without exposing a path", () => {
  assert.equal(
    createPdfPreviewUrl("token value", "spec v1.PDF"),
    "morpheus-file-preview://pdf/token%20value/spec%20v1.PDF",
  );
});

test("buildPdfPreview returns bounded metadata and a preview protocol URL", () => {
  assert.deepEqual(
    buildPdfPreview("/tmp/Project Docs/spec v1.PDF", 2048, "token-1"),
    {
      path: "/tmp/Project Docs/spec v1.PDF",
      mimeType: "application/pdf",
      name: "spec v1.PDF",
      byteSize: 2048,
      url: "morpheus-file-preview://pdf/token-1/spec%20v1.PDF",
    },
  );
});
