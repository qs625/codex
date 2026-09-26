import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

export function readSource(url: string | URL): string {
  return readFileSync(url, "utf8");
}

export function sourceIndex(
  source: string,
  needle: string,
  fromIndex = 0,
): number {
  const index = source.indexOf(needle, fromIndex);
  assert.notEqual(index, -1, `Expected source to contain ${JSON.stringify(needle)}`);
  return index;
}

export function sourceSlice(
  source: string,
  startNeedle: string,
  endNeedle: string,
  fromIndex = 0,
): string {
  const start = sourceIndex(source, startNeedle, fromIndex);
  const end = sourceIndex(source, endNeedle, start);
  return source.slice(start, end);
}
