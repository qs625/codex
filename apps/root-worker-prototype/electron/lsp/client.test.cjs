const test = require("node:test");
const assert = require("node:assert/strict");

const { LspClient, normalizeLocations } = require("./client.cjs");

test("normalizeLocations uses LocationLink target selection range", () => {
  assert.deepEqual(
    normalizeLocations({
      targetUri: "file:///tmp/src/Button.tsx",
      targetRange: {
        start: { line: 9, character: 0 },
        end: { line: 14, character: 1 },
      },
      targetSelectionRange: {
        start: { line: 10, character: 16 },
        end: { line: 10, character: 22 },
      },
    }),
    [
      {
        uri: "file:///tmp/src/Button.tsx",
        range: {
          start: { line: 10, character: 16 },
          end: { line: 10, character: 22 },
        },
      },
    ],
  );
});

test("openDocument sends full-text didChange when an already open document changes", async () => {
  const notifications = [];
  const client = Object.create(LspClient.prototype);
  client.adapter = {
    languageIdForFile(filePath) {
      return filePath.endsWith(".tsx") ? "typescriptreact" : "typescript";
    },
  };
  client.openDocuments = new Map();
  client.notify = (method, params) => {
    notifications.push({ method, params });
  };

  await client.openDocument("/tmp/src/App.tsx", "export const value = 1;\n");
  await client.openDocument("/tmp/src/App.tsx", "export const value = 1;\n");
  await client.openDocument("/tmp/src/App.tsx", "export const value = 2;\n");

  assert.equal(notifications.length, 2);
  assert.equal(notifications[0].method, "textDocument/didOpen");
  assert.equal(notifications[0].params.textDocument.languageId, "typescriptreact");
  assert.equal(notifications[0].params.textDocument.version, 1);
  assert.equal(notifications[1].method, "textDocument/didChange");
  assert.deepEqual(notifications[1].params, {
    textDocument: {
      uri: "file:///tmp/src/App.tsx",
      version: 2,
    },
    contentChanges: [{ text: "export const value = 2;\n" }],
  });
});
