export type WorkspaceObjectTabKind =
  | "conversation"
  | "file"
  | "terminal"
  | "browser";

export type WorkspaceObjectTab = {
  id: string;
  kind: WorkspaceObjectTabKind;
  title: string;
  subtitle?: string | null;
  threadId?: string | null;
  rootId?: string | null;
  path?: string | null;
  url?: string | null;
  browserTabId?: string | null;
  terminalTabId?: string | null;
  terminalSessionId?: string | null;
  cwd?: string | null;
  status?: string | null;
};

export type WorkspaceTabDropPlacement = "before" | "after";

const WORKSPACE_TAB_ORDER_STORAGE_KEY =
  "root-worker-prototype:workspace-object-tab-order";

type WorkspaceTabStorage = Pick<Storage, "getItem" | "setItem">;

export function reorderWorkspaceTabs<T extends { id: string }>(
  tabs: readonly T[],
  draggedTabId: string,
  targetTabId: string,
  placement: WorkspaceTabDropPlacement = "before",
): T[] {
  if (draggedTabId === targetTabId) {
    return [...tabs];
  }
  const nextTabs = [...tabs];
  const draggedIndex = nextTabs.findIndex((tab) => tab.id === draggedTabId);
  if (draggedIndex === -1 || !nextTabs.some((tab) => tab.id === targetTabId)) {
    return nextTabs;
  }
  const [dragged] = nextTabs.splice(draggedIndex, 1);
  const targetIndex = nextTabs.findIndex((tab) => tab.id === targetTabId);
  const insertIndex = placement === "after" ? targetIndex + 1 : targetIndex;
  nextTabs.splice(insertIndex, 0, dragged!);
  return nextTabs;
}

export function upsertWorkspaceTab(
  tabs: readonly WorkspaceObjectTab[],
  tab: WorkspaceObjectTab,
): WorkspaceObjectTab[] {
  const existingIndex = tabs.findIndex((item) => item.id === tab.id);
  if (existingIndex === -1) {
    return [...tabs, tab];
  }
  return tabs.map((item, index) =>
    index === existingIndex ? { ...item, ...tab } : item,
  );
}

export function closeWorkspaceTabById<T extends { id: string }>(
  tabs: readonly T[],
  tabId: string,
): T[] {
  return tabs.filter((tab) => tab.id !== tabId);
}

export function sanitizeWorkspaceTabs(
  tabs: readonly WorkspaceObjectTab[],
): WorkspaceObjectTab[] {
  const seen = new Set<string>();
  const next: WorkspaceObjectTab[] = [];
  for (const tab of tabs) {
    if (!isWorkspaceObjectTab(tab) || seen.has(tab.id)) {
      continue;
    }
    seen.add(tab.id);
    next.push(tab);
  }
  return next;
}

export function readStoredWorkspaceTabOrder(
  storage: WorkspaceTabStorage | null | undefined = getLocalStorage(),
): string[] {
  if (!storage) {
    return [];
  }

  try {
    const stored = storage.getItem(WORKSPACE_TAB_ORDER_STORAGE_KEY);
    if (!stored) {
      return [];
    }
    const parsed = JSON.parse(stored);
    return Array.isArray(parsed)
      ? parsed.filter((value): value is string => typeof value === "string")
      : [];
  } catch {
    return [];
  }
}

export function applyStoredWorkspaceTabOrder(
  tabs: readonly WorkspaceObjectTab[],
  storedOrder: readonly string[],
): WorkspaceObjectTab[] {
  const sanitizedTabs = sanitizeWorkspaceTabs(tabs);
  if (storedOrder.length === 0) {
    return sanitizedTabs;
  }
  const tabById = new Map(sanitizedTabs.map((tab) => [tab.id, tab]));
  const next: WorkspaceObjectTab[] = [];
  for (const tabId of storedOrder) {
    const tab = tabById.get(tabId);
    if (!tab) {
      continue;
    }
    next.push(tab);
    tabById.delete(tabId);
  }
  next.push(...tabById.values());
  return next;
}

export function storeWorkspaceTabOrder(
  tabs: readonly WorkspaceObjectTab[],
  storage: WorkspaceTabStorage | null | undefined = getLocalStorage(),
  previousOrder: readonly string[] = [],
): string[] {
  const order = mergeWorkspaceTabOrder(tabs, previousOrder);
  if (!storage) {
    return order;
  }

  try {
    storage.setItem(WORKSPACE_TAB_ORDER_STORAGE_KEY, JSON.stringify(order));
  } catch {
    // Best-effort preference only; tab switching should never depend on storage.
  }
  return order;
}

export function mergeWorkspaceTabOrder(
  tabs: readonly WorkspaceObjectTab[],
  previousOrder: readonly string[] = [],
): string[] {
  const currentIds = sanitizeWorkspaceTabs(tabs).map((tab) => tab.id);
  if (currentIds.length < 2) {
    const previousIdSet = new Set(previousOrder);
    return [
      ...previousOrder,
      ...currentIds.filter((tabId) => !previousIdSet.has(tabId)),
    ];
  }
  const currentIdSet = new Set(currentIds);
  return [
    ...currentIds,
    ...previousOrder.filter((tabId) => !currentIdSet.has(tabId)),
  ];
}

function isWorkspaceObjectTab(value: unknown): value is WorkspaceObjectTab {
  if (!value || typeof value !== "object") {
    return false;
  }
  const tab = value as Partial<WorkspaceObjectTab>;
  return (
    typeof tab.id === "string" &&
    tab.id.length > 0 &&
    typeof tab.title === "string" &&
    tab.title.length > 0 &&
    (tab.kind === "conversation" ||
      tab.kind === "file" ||
      tab.kind === "terminal" ||
      tab.kind === "browser")
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
