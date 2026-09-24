const DEFAULT_BROWSER_SURFACE_ID = "browser-surface-default";

function browserSurfaceIdFromPayload(payload) {
  const surfaceId =
    payload && typeof payload === "object" && typeof payload.surfaceId === "string"
      ? payload.surfaceId.trim()
      : "";
  return surfaceId || DEFAULT_BROWSER_SURFACE_ID;
}

function browserTabIdFromPayload(payload) {
  return payload && typeof payload === "object" && typeof payload.tabId === "string"
    ? payload.tabId
    : null;
}

function browserTargetFromPayload(payload) {
  return payload && typeof payload === "object" && "target" in payload
    ? payload.target
    : payload;
}

function browserBoundsFromPayload(payload) {
  if (!payload || typeof payload !== "object") {
    return payload;
  }
  const { x, y, width, height, sequence } = payload;
  return { x, y, width, height, sequence };
}

function createBrowserPanelSurfaceState(initialBoundsUpdate) {
  return {
    visible: false,
    bounds: initialBoundsUpdate.bounds,
    boundsSequence: initialBoundsUpdate.sequence,
    visibleSurfaceIds: new Set(),
    boundsBySurfaceId: new Map([
      [DEFAULT_BROWSER_SURFACE_ID, initialBoundsUpdate.bounds],
    ]),
    boundsSequenceBySurfaceId: new Map([
      [DEFAULT_BROWSER_SURFACE_ID, initialBoundsUpdate.sequence],
    ]),
    attachedTabIdBySurfaceId: new Map(),
    attachedTabId: null,
  };
}

function browserPanelBoundsForSurface(panel, surfaceId) {
  return panel.boundsBySurfaceId.get(surfaceId) ?? panel.bounds;
}

function updateBrowserPanelAttachedTabId(panel) {
  panel.attachedTabId =
    panel.attachedTabIdBySurfaceId.get(DEFAULT_BROWSER_SURFACE_ID) ??
    panel.attachedTabIdBySurfaceId.values().next().value ??
    null;
  return panel.attachedTabId;
}

function rememberBrowserPanelAttachedTab(panel, surfaceId, tabId) {
  panel.attachedTabIdBySurfaceId.set(surfaceId, tabId);
  return updateBrowserPanelAttachedTabId(panel);
}

function forgetBrowserPanelAttachedSurface(panel, surfaceId) {
  const attachedTabId = panel.attachedTabIdBySurfaceId.get(surfaceId) ?? null;
  panel.attachedTabIdBySurfaceId.delete(surfaceId);
  updateBrowserPanelAttachedTabId(panel);
  return attachedTabId;
}

function forgetBrowserPanelTabFromSurfaces(panel, tabId) {
  for (const [surfaceId, attachedTabId] of panel.attachedTabIdBySurfaceId) {
    if (attachedTabId === tabId) {
      panel.attachedTabIdBySurfaceId.delete(surfaceId);
    }
  }
  return updateBrowserPanelAttachedTabId(panel);
}

function showBrowserPanelSurface(panel, surfaceId) {
  panel.visible = true;
  panel.visibleSurfaceIds.add(surfaceId);
}

function hideBrowserPanelSurface(panel, surfaceId) {
  panel.visibleSurfaceIds.delete(surfaceId);
  panel.visible = panel.visibleSurfaceIds.size > 0;
}

function hideAllBrowserPanelSurfaces(panel) {
  panel.visibleSurfaceIds.clear();
  panel.visible = false;
}

module.exports = {
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
  updateBrowserPanelAttachedTabId,
};
