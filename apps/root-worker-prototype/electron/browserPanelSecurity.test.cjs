const test = require("node:test");
const assert = require("node:assert/strict");

const {
  browserNavigationDecision,
  browserNavigationEventDecision,
  browserNavigationEventTarget,
  normalizeBrowserDebugTarget,
  normalizeBrowserTarget,
} = require("./browserPanelSecurity.cjs");

test("normalizeBrowserTarget keeps explicit URLs and local dev targets", () => {
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
  assert.deepEqual(normalizeBrowserTarget("file:///tmp/index.html"), {
    ok: true,
    url: "file:///tmp/index.html",
  });
  assert.deepEqual(normalizeBrowserTarget("custom-scheme:foo"), {
    ok: true,
    url: "custom-scheme:foo",
  });
});

test("browserNavigationDecision allows explicit schemes", () => {
  for (const target of ["file:///tmp/secret.txt", "data:text/html,hello", "custom-scheme:foo"]) {
    const decision = browserNavigationDecision(target);
    assert.equal(decision.allow, true);
    assert.equal(decision.url, target);
  }
});

test("normalizeBrowserDebugTarget preserves about:blank as CDP bootstrap", () => {
  assert.deepEqual(normalizeBrowserDebugTarget("about:blank"), {
    ok: true,
    url: null,
  });
  assert.deepEqual(normalizeBrowserDebugTarget(" https://example.com "), {
    ok: true,
    url: "https://example.com/",
  });
  assert.deepEqual(normalizeBrowserDebugTarget("file:///tmp/secret.txt"), {
    ok: true,
    url: "file:///tmp/secret.txt",
  });
});

test("browserNavigationEventTarget handles legacy and Electron 37 frame events", () => {
  assert.equal(
    browserNavigationEventTarget({ url: "https://frame.example/" }),
    "https://frame.example/",
  );
  assert.equal(
    browserNavigationEventTarget({}, { url: "https://details.example/" }),
    "https://details.example/",
  );
  assert.equal(
    browserNavigationEventTarget({ url: "https://frame.example/" }, "https://legacy.example/"),
    "https://legacy.example/",
  );
});

test("browserNavigationEventDecision allows Electron 37 frame explicit targets", () => {
  assert.deepEqual(browserNavigationEventDecision({ url: "https://frame.example/" }), {
    allow: true,
    url: "https://frame.example/",
  });
  assert.deepEqual(
    browserNavigationEventDecision({}, { url: "https://details.example/" }),
    {
      allow: true,
      url: "https://details.example/",
    },
  );
  assert.deepEqual(browserNavigationEventDecision({ url: "file:///tmp/secret.txt" }), {
    allow: true,
    url: "file:///tmp/secret.txt",
  });
  assert.deepEqual(browserNavigationEventDecision({ url: "custom-scheme:foo" }), {
    allow: true,
    url: "custom-scheme:foo",
  });
});

test("browser navigation rejects empty and invalid targets", () => {
  assert.equal(browserNavigationDecision(" ").allow, false);
  assert.equal(browserNavigationDecision("http://").allow, false);
  assert.equal(browserNavigationEventDecision({ url: null }).allow, false);
});
