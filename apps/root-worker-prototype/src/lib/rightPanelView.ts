import type { RightPanelView } from "../types";

const RIGHT_PANEL_VIEW_STORAGE_KEY = "root-worker-prototype:right-panel-view";

const RIGHT_PANEL_VIEWS = new Set<RightPanelView>(["todo", "preview", "skills"]);

type PanelViewStorage = Pick<Storage, "getItem" | "setItem">;

export function readStoredRightPanelView(
  storage: PanelViewStorage | null | undefined = getLocalStorage(),
): RightPanelView {
  if (!storage) {
    return "todo";
  }

  try {
    const stored = storage.getItem(RIGHT_PANEL_VIEW_STORAGE_KEY);
    return isRightPanelView(stored) ? stored : "todo";
  } catch {
    return "todo";
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
