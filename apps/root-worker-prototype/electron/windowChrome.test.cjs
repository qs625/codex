const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");
const test = require("node:test");

test("main window uses minimal macOS titlebar chrome without going frameless", () => {
  const mainSource = readFileSync(join(__dirname, "main.cjs"), "utf8");

  assert.match(mainSource, /process\.platform === "darwin"/);
  assert.match(mainSource, /titleBarStyle: "hiddenInset"/);
  assert.match(mainSource, /trafficLightPosition: \{ x: 14, y: 14 \}/);
  assert.doesNotMatch(mainSource, /frame:\s*false/);
});
