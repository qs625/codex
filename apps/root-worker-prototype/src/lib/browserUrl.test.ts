import test from "node:test";
import assert from "node:assert/strict";

import { normalizeBrowserUrl } from "./browserUrl";

test("normalizeBrowserUrl keeps explicit http and https URLs", () => {
  assert.deepEqual(normalizeBrowserUrl("https://example.com/docs"), {
    ok: true,
    url: "https://example.com/docs",
  });
  assert.deepEqual(normalizeBrowserUrl("http://127.0.0.1:3000"), {
    ok: true,
    url: "http://127.0.0.1:3000/",
  });
});

test("normalizeBrowserUrl adds a safe default scheme", () => {
  assert.deepEqual(normalizeBrowserUrl("example.com"), {
    ok: true,
    url: "https://example.com/",
  });
  assert.deepEqual(normalizeBrowserUrl("localhost:5173/debug"), {
    ok: true,
    url: "http://localhost:5173/debug",
  });
});

test("normalizeBrowserUrl rejects dangerous schemes", () => {
  assert.equal(normalizeBrowserUrl("javascript:alert(1)").ok, false);
  assert.equal(normalizeBrowserUrl("data:text/html,hello").ok, false);
  assert.equal(normalizeBrowserUrl("file:///tmp/index.html").ok, false);
});

test("normalizeBrowserUrl rejects empty and invalid targets", () => {
  assert.equal(normalizeBrowserUrl(" ").ok, false);
  assert.equal(normalizeBrowserUrl("http://").ok, false);
});
