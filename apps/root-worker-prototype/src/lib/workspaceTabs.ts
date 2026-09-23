export type WorkspaceTabId = "conversation" | "files" | "terminal" | "browser";
export type WorkspaceTabDropPlacement = "before" | "after";

export const WORKSPACE_TAB_IDS: WorkspaceTabId[] = [
  "conversation",
  "files",
  "terminal",
  "browser",
];

const WORKSPACE_TAB_ORDER_STORAGE_KEY =
  "root-worker-prototype:workspace-tab-order";

type WorkspaceTabStorage = Pick<Storage, "getItem" | "setItem">;

export function reorderWorkspaceTabs(
  order: readonly WorkspaceTabId[],
  draggedTab: WorkspaceTabId,
  targetTab: WorkspaceTabId,
  placement: WorkspaceTabDropPlacement = "before",
): WorkspaceTabId[] {
  if (draggedTab === targetTab) {
    return sanitizeWorkspaceTabOrder(order);
  }
  const nextOrder = sanitizeWorkspaceTabOrder(order);
  const draggedIndex = nextOrder.indexOf(draggedTab);
  if (draggedIndex === -1 || !nextOrder.includes(targetTab)) {
    return nextOrder;
  }
  const [dragged] = nextOrder.splice(draggedIndex, 1);
  const targetIndex = nextOrder.indexOf(targetTab);
  const insertIndex = placement === "after" ? targetIndex + 1 : targetIndex;
  nextOrder.splice(insertIndex, 0, dragged!);
  return nextOrder;
}

export function readStoredWorkspaceTabOrder(
  storage: WorkspaceTabStorage | null | undefined = getLocalStorage(),
): WorkspaceTabId[] {
  if (!storage) {
    return [...WORKSPACE_TAB_IDS];
  }

  try {
    const stored = storage.getItem(WORKSPACE_TAB_ORDER_STORAGE_KEY);
    if (!stored) {
      return [...WORKSPACE_TAB_IDS];
    }
    const parsed = JSON.parse(stored);
    return Array.isArray(parsed)
      ? sanitizeWorkspaceTabOrder(parsed)
      : [...WORKSPACE_TAB_IDS];
  } catch {
    return [...WORKSPACE_TAB_IDS];
  }
}

export function storeWorkspaceTabOrder(
  order: readonly WorkspaceTabId[],
  storage: WorkspaceTabStorage | null | undefined = getLocalStorage(),
) {
  if (!storage) {
    return;
  }

  try {
    storage.setItem(
      WORKSPACE_TAB_ORDER_STORAGE_KEY,
      JSON.stringify(sanitizeWorkspaceTabOrder(order)),
    );
  } catch {
    // Best-effort preference only; tab switching should never depend on storage.
  }
}

function sanitizeWorkspaceTabOrder(order: readonly unknown[]): WorkspaceTabId[] {
  const seen = new Set<WorkspaceTabId>();
  const next: WorkspaceTabId[] = [];
  for (const tab of order) {
    if (!isWorkspaceTabId(tab) || seen.has(tab)) {
      continue;
    }
    seen.add(tab);
    next.push(tab);
  }
  for (const tab of WORKSPACE_TAB_IDS) {
    if (!seen.has(tab)) {
      next.push(tab);
    }
  }
  return next;
}

function isWorkspaceTabId(value: unknown): value is WorkspaceTabId {
  return (
    typeof value === "string" &&
    WORKSPACE_TAB_IDS.includes(value as WorkspaceTabId)
  );
}

function getLocalStorage(): WorkspaceTabStorage | null {
  if (typeof window === "undefined") {
    return null;
  }

  try {
    return window.localStorage;
  } catch {
    return null;
  }
}
