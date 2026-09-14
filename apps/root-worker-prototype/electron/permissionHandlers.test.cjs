const test = require("node:test");
const assert = require("node:assert/strict");

const {
  allowBrowserPanelPermission,
  allowDefaultSessionPermission,
  configurePermissionHandlers,
} = require("./permissionHandlers.cjs");

test("allowBrowserPanelPermission allows representative browser permissions", () => {
  for (const permission of [
    "media",
    "geolocation",
    "notifications",
    "clipboard-read",
    "local-network-access",
    "local-network",
    "loopback-network",
    "unknown",
  ]) {
    assert.equal(allowBrowserPanelPermission({ permission }), true);
  }
});

test("allowDefaultSessionPermission keeps media scoped out of Browser panel contents", () => {
  const appWebContents = { id: 1 };
  const browserPanelWebContents = { id: 2 };
  const isBrowserPanelWebContents = (webContents) =>
    webContents === browserPanelWebContents;

  assert.equal(
    allowDefaultSessionPermission({
      webContents: appWebContents,
      permission: "media",
      isBrowserPanelWebContents,
    }),
    true,
  );
  assert.equal(
    allowDefaultSessionPermission({
      webContents: browserPanelWebContents,
      permission: "media",
      isBrowserPanelWebContents,
    }),
    false,
  );
  assert.equal(
    allowDefaultSessionPermission({
      webContents: appWebContents,
      permission: "geolocation",
      isBrowserPanelWebContents,
    }),
    false,
  );
});

test("configurePermissionHandlers wires check and request details", () => {
  const calls = [];
  const fakeSession = {
    setPermissionCheckHandler(handler) {
      this.checkHandler = handler;
    },
    setPermissionRequestHandler(handler) {
      this.requestHandler = handler;
    },
  };
  const webContents = { id: 3 };

  configurePermissionHandlers(fakeSession, (request) => {
    calls.push(request);
    return request.permission === "media";
  });

  assert.equal(
    fakeSession.checkHandler(
      webContents,
      "media",
      "https://example.com",
      { requestingUrl: "https://example.com/camera", isMainFrame: true },
    ),
    true,
  );
  assert.equal(fakeSession.checkHandler(webContents, "geolocation"), false);

  let requestGranted = null;
  fakeSession.requestHandler(
    webContents,
    "media",
    (granted) => {
      requestGranted = granted;
    },
    { requestingUrl: "https://example.com/camera", isMainFrame: true },
  );
  assert.equal(requestGranted, true);

  assert.deepEqual(calls.map((call) => call.requestType), [
    "check",
    "check",
    "request",
  ]);
  assert.equal(calls[0].requestingOrigin, "https://example.com");
  assert.equal(calls[2].details.requestingUrl, "https://example.com/camera");
});
