const test = require("node:test");
const assert = require("node:assert/strict");
const path = require("node:path");

const { parseLocalFileTarget } = require("./fileTargets.cjs");

test("parseLocalFileTarget keeps line and column for relative local files", () => {
  const defaultWorkspace = "/workspace/root";
  const result = parseLocalFileTarget("./src/lib.rs:14:9", defaultWorkspace);

  assert.deepEqual(result, {
    path: path.resolve(defaultWorkspace, "./src/lib.rs"),
    line: 14,
    column: 9,
  });
});

test("parseLocalFileTarget ignores location parsing for extensionless paths", () => {
  const defaultWorkspace = "/workspace/root";
  const result = parseLocalFileTarget("./BUILD:12", defaultWorkspace);

  assert.deepEqual(result, {
    path: path.resolve(defaultWorkspace, "./BUILD:12"),
    line: null,
    column: null,
  });
});
