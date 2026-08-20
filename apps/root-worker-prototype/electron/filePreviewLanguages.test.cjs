const test = require("node:test");
const assert = require("node:assert/strict");

const { languageForFilePath } = require("./filePreviewLanguages.cjs");

test("languageForFilePath maps Go files to Monaco's Go language id", () => {
  assert.equal(languageForFilePath("/workspace/main.go"), "go");
  assert.equal(languageForFilePath("/workspace/MAIN.GO"), "go");
});

test("languageForFilePath keeps existing preview language routing", () => {
  assert.equal(languageForFilePath("/workspace/App.tsx"), "typescript");
  assert.equal(languageForFilePath("/workspace/lib.rs"), "rust");
  assert.equal(languageForFilePath("/workspace/README.md"), "markdown");
  assert.equal(languageForFilePath("/workspace/script.sh"), "shell");
  assert.equal(languageForFilePath("/workspace/unknown.custom"), "plaintext");
});
