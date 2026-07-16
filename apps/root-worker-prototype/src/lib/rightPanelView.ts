import type { RightPanelView } from "../types";

const RIGHT_PANEL_VIEW_STORAGE_KEY = "root-worker-prototype:right-panel-view";

const DEFAULT_RIGHT_PANEL_VIEW: RightPanelView = "skills";
const RIGHT_PANEL_VIEWS = new Set<RightPanelView>(["preview", "skills", "git"]);

type PanelViewStorage = Pick<Storage, "getItem" | "setItem">;

export function readStoredRightPanelView(
  storage: PanelViewStorage | null | undefined = getLocalStorage(),
): RightPanelView {
  if (!storage) {
    return DEFAULT_RIGHT_PANEL_VIEW;
  }

  try {
    const stored = storage.getItem(RIGHT_PANEL_VIEW_STORAGE_KEY);
    return isRightPanelView(stored) ? stored : DEFAULT_RIGHT_PANEL_VIEW;
  } catch {
    return DEFAULT_RIGHT_PANEL_VIEW;
  }
}

export function storeRightPanelView(
  view: RightPanelView,
  storage: PanelViewStorage | null | undefined = getLocalStorage(),
) {
  if (!storage) {
    return;
  }

  try {
    storage.setItem(RIGHT_PANEL_VIEW_STORAGE_KEY, view);
  } catch {
    // Ignore storage failures; the panel should still switch immediately.
  }
}

function isRightPanelView(value: string | null): value is RightPanelView {
  return value !== null && RIGHT_PANEL_VIEWS.has(value as RightPanelView);
}

function getLocalStorage(): PanelViewStorage | null {
  if (typeof window === "undefined") {
    return null;
  }

  try {
    return window.localStorage;
  } catch {
    return null;
  }
}
