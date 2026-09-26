const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");

function readSource(relativePath) {
  return readFileSync(join(__dirname, relativePath), "utf8");
}

function sourceIndex(source, needle, fromIndex = 0) {
  const index = source.indexOf(needle, fromIndex);
  assert.notEqual(index, -1, `Expected source to contain ${JSON.stringify(needle)}`);
  return index;
}

function sourceSlice(source, startNeedle, endNeedle, fromIndex = 0) {
  const start = sourceIndex(source, startNeedle, fromIndex);
  const end = sourceIndex(source, endNeedle, start);
  return source.slice(start, end);
}

module.exports = {
  readSource,
  sourceIndex,
  sourceSlice,
};
