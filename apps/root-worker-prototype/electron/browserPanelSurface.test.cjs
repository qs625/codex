const assert = require("node:assert/strict");
const test = require("node:test");

const {
  DEFAULT_BROWSER_SURFACE_ID,
  browserBoundsFromPayload,
  browserPanelBoundsForSurface,
  browserSurfaceIdFromPayload,
  browserTabIdFromPayload,
  browserTargetFromPayload,
  createBrowserPanelSurfaceState,
  forgetBrowserPanelAttachedSurface,
  forgetBrowserPanelTabFromSurfaces,
  hideAllBrowserPanelSurfaces,
  hideBrowserPanelSurface,
  rememberBrowserPanelAttachedTab,
  showBrowserPanelSurface,
} = require("./browserPanelSurface.cjs");

test("payload helpers normalize surface, tab, target, and bounds", () => {
  assert.equal(browserSurfaceIdFromPayload(null), DEFAULT_BROWSER_SURFACE_ID);
  assert.equal(browserSurfaceIdFromPayload({ surfaceId: " workspace " }), "workspace");
  assert.equal(browserTabIdFromPayload({ tabId: "tab-1" }), "tab-1");
  assert.equal(browserTabIdFromPayload({ tabId: 1 }), null);
  assert.equal(browserTargetFromPayload({ target: "https://example.com" }), "https://example.com");
  assert.deepEqual(
    browserBoundsFromPayload({ x: 1, y: 2, width: 3, height: 4, sequence: 5, ignored: true }),
    { x: 1, y: 2, width: 3, height: 4, sequence: 5 },
  );
});

test("surface state tracks independent visibility and default bounds", () => {
  const defaultBounds = { x: 0, y: 0, width: 100, height: 80 };
  const panel = createBrowserPanelSurfaceState({
    bounds: defaultBounds,
    sequence: 10,
  });

  assert.equal(panel.visible, false);
  assert.deepEqual(browserPanelBoundsForSurface(panel, "missing"), defaultBounds);

  showBrowserPanelSurface(panel, DEFAULT_BROWSER_SURFACE_ID);
  showBrowserPanelSurface(panel, "workspace-surface");
  assert.equal(panel.visible, true);
  assert.deepEqual([...panel.visibleSurfaceIds], [
    DEFAULT_BROWSER_SURFACE_ID,
    "workspace-surface",
  ]);

  hideBrowserPanelSurface(panel, "workspace-surface");
  assert.equal(panel.visible, true);
  assert.deepEqual([...panel.visibleSurfaceIds], [DEFAULT_BROWSER_SURFACE_ID]);

  hideBrowserPanelSurface(panel, DEFAULT_BROWSER_SURFACE_ID);
  assert.equal(panel.visible, false);
  assert.deepEqual([...panel.visibleSurfaceIds], []);
});

test("attached tab bookkeeping prefers default surface then any remaining surface", () => {
  const panel = createBrowserPanelSurfaceState({
    bounds: { x: 0, y: 0, width: 100, height: 80 },
    sequence: 10,
  });

  rememberBrowserPanelAttachedTab(panel, "workspace-surface", "tab-workspace");
  assert.equal(panel.attachedTabId, "tab-workspace");

  rememberBrowserPanelAttachedTab(panel, DEFAULT_BROWSER_SURFACE_ID, "tab-default");
  assert.equal(panel.attachedTabId, "tab-default");

  assert.equal(
    forgetBrowserPanelAttachedSurface(panel, DEFAULT_BROWSER_SURFACE_ID),
    "tab-default",
  );
  assert.equal(panel.attachedTabId, "tab-workspace");

  forgetBrowserPanelTabFromSurfaces(panel, "tab-workspace");
  assert.equal(panel.attachedTabId, null);
});

test("hideAllBrowserPanelSurfaces clears every visible surface", () => {
  const panel = createBrowserPanelSurfaceState({
    bounds: { x: 0, y: 0, width: 100, height: 80 },
    sequence: 10,
  });
  showBrowserPanelSurface(panel, DEFAULT_BROWSER_SURFACE_ID);
  showBrowserPanelSurface(panel, "workspace-surface");

  hideAllBrowserPanelSurfaces(panel);

  assert.equal(panel.visible, false);
  assert.deepEqual([...panel.visibleSurfaceIds], []);
});
