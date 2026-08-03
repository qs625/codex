const test = require("node:test");
const assert = require("node:assert/strict");

const {
  browserNavigationDecision,
  browserNavigationEventDecision,
  browserNavigationEventTarget,
  normalizeBrowserTarget,
} = require("./browserPanelSecurity.cjs");

test("normalizeBrowserTarget allows http, https, and local dev targets", () => {
  assert.deepEqual(normalizeBrowserTarget("https://example.com/docs"), {
    ok: true,
    url: "https://example.com/docs",
  });
  assert.deepEqual(normalizeBrowserTarget("example.com"), {
    ok: true,
    url: "https://example.com/",
  });
  assert.deepEqual(normalizeBrowserTarget("localhost:5173/debug"), {
    ok: true,
    url: "http://localhost:5173/debug",
  });
});

test("browserNavigationDecision rejects unsafe redirect and frame targets", () => {
  for (const target of [
    "file:///tmp/secret.txt",
    "data:text/html,hello",
    "javascript:alert(1)",
  ]) {
    const decision = browserNavigationDecision(target);
    assert.equal(decision.allow, false);
  }
});

test("browserNavigationEventTarget handles legacy and Electron 37 frame events", () => {
  assert.equal(
    browserNavigationEventTarget({ url: "https://frame.example/" }),
    "https://frame.example/",
  );
  assert.equal(
    browserNavigationEventTarget({ url: "https://frame.example/" }, "https://legacy.example/"),
    "https://legacy.example/",
  );
});

test("browserNavigationEventDecision allows Electron 37 frame http targets", () => {
  assert.deepEqual(browserNavigationEventDecision({ url: "https://frame.example/" }), {
    allow: true,
    url: "https://frame.example/",
  });
  assert.equal(
    browserNavigationEventDecision({ url: "file:///tmp/secret.txt" }).allow,
    false,
  );
});
