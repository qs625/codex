export type WorkspaceObjectTabKind =
  | "conversation"
  | "file"
  | "diff"
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
  gitDiffTargetId?: string | null;
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
  return OrderedWorkspaceTabs.from(tabs).reorder(
    draggedTabId,
    targetTabId,
    placement,
  );
}

export function upsertWorkspaceTab(
  tabs: readonly WorkspaceObjectTab[],
  tab: WorkspaceObjectTab,
): WorkspaceObjectTab[] {
  return OrderedWorkspaceTabs.from(tabs).upsert(tab);
}

export function closeWorkspaceTabById<T extends { id: string }>(
  tabs: readonly T[],
  tabId: string,
): T[] {
  return tabs.filter((tab) => tab.id !== tabId);
}

export function resolveActiveWorkspaceTabId<T extends { id: string }>(
  tabs: readonly T[],
  activeTabId: string | null | undefined,
  preferredTabId: string | null | undefined = null,
): string | null {
  return OrderedWorkspaceTabs.from(tabs).resolveActiveId(
    activeTabId,
    preferredTabId,
  );
}

export function sanitizeWorkspaceTabs(
  tabs: readonly WorkspaceObjectTab[],
): WorkspaceObjectTab[] {
  return SanitizedWorkspaceTabs.from(tabs).tabs;
}

export function readStoredWorkspaceTabOrder(
  storage: WorkspaceTabStorage | null | undefined = getLocalStorage(),
): string[] {
  return StoredWorkspaceTabOrder.read(storage).ids;
}

export function applyStoredWorkspaceTabOrder(
  tabs: readonly WorkspaceObjectTab[],
  storedOrder: readonly string[],
): WorkspaceObjectTab[] {
  return StoredWorkspaceTabOrder.from(storedOrder).applyTo(tabs);
}

export function storeWorkspaceTabOrder(
  tabs: readonly WorkspaceObjectTab[],
  storage: WorkspaceTabStorage | null | undefined = getLocalStorage(),
  previousOrder: readonly string[] = [],
): string[] {
  const order = StoredWorkspaceTabOrder.merge(tabs, previousOrder);
  order.write(storage);
  return order.ids;
}

export function mergeWorkspaceTabOrder(
  tabs: readonly WorkspaceObjectTab[],
  previousOrder: readonly string[] = [],
): string[] {
  return StoredWorkspaceTabOrder.merge(tabs, previousOrder).ids;
}

class OrderedWorkspaceTabs<T extends { id: string }> {
  private constructor(private readonly tabs: readonly T[]) {}

  static from<T extends { id: string }>(tabs: readonly T[]) {
    return new OrderedWorkspaceTabs(tabs);
  }

  reorder(
    draggedTabId: string,
    targetTabId: string,
    placement: WorkspaceTabDropPlacement,
  ): T[] {
    if (draggedTabId === targetTabId) {
      return this.toArray();
    }
    const nextTabs = this.toArray();
    const draggedIndex = nextTabs.findIndex((tab) => tab.id === draggedTabId);
    if (
      draggedIndex === -1 ||
      !nextTabs.some((tab) => tab.id === targetTabId)
    ) {
      return nextTabs;
    }
    const [dragged] = nextTabs.splice(draggedIndex, 1);
    const targetIndex = nextTabs.findIndex((tab) => tab.id === targetTabId);
    const insertIndex = placement === "after" ? targetIndex + 1 : targetIndex;
    nextTabs.splice(insertIndex, 0, dragged!);
    return nextTabs;
  }

  upsert(tab: T): T[] {
    const existingIndex = this.tabs.findIndex((item) => item.id === tab.id);
    if (existingIndex === -1) {
      return [...this.tabs, tab];
    }
    return this.tabs.map((item, index) =>
      index === existingIndex ? { ...item, ...tab } : item,
    );
  }

  resolveActiveId(
    activeTabId: string | null | undefined,
    preferredTabId: string | null | undefined,
  ): string | null {
    if (activeTabId && this.has(activeTabId)) {
      return activeTabId;
    }
    if (preferredTabId && this.has(preferredTabId)) {
      return preferredTabId;
    }
    return this.tabs[0]?.id ?? null;
  }

  private has(tabId: string) {
    return this.tabs.some((tab) => tab.id === tabId);
  }

  private toArray() {
    return [...this.tabs];
  }
}

class SanitizedWorkspaceTabs {
  private constructor(readonly tabs: WorkspaceObjectTab[]) {}

  static from(tabs: readonly WorkspaceObjectTab[]) {
    const seen = new Set<string>();
    const next: WorkspaceObjectTab[] = [];
    for (const tab of tabs) {
      if (!isWorkspaceObjectTab(tab) || seen.has(tab.id)) {
        continue;
      }
      seen.add(tab.id);
      next.push(tab);
    }
    return new SanitizedWorkspaceTabs(next);
  }

  ids() {
    return this.tabs.map((tab) => tab.id);
  }
}

class StoredWorkspaceTabOrder {
  private constructor(readonly ids: string[]) {}

  static from(ids: readonly string[]) {
    return new StoredWorkspaceTabOrder([...ids]);
  }

  static read(storage: WorkspaceTabStorage | null | undefined) {
    if (!storage) {
      return new StoredWorkspaceTabOrder([]);
    }

    try {
      const stored = storage.getItem(WORKSPACE_TAB_ORDER_STORAGE_KEY);
      if (!stored) {
        return new StoredWorkspaceTabOrder([]);
      }
      const parsed = JSON.parse(stored);
      return new StoredWorkspaceTabOrder(
        Array.isArray(parsed)
          ? parsed.filter((value): value is string => typeof value === "string")
          : [],
      );
    } catch {
      return new StoredWorkspaceTabOrder([]);
    }
  }

  static merge(
    tabs: readonly WorkspaceObjectTab[],
    previousOrder: readonly string[],
  ) {
    const currentIds = SanitizedWorkspaceTabs.from(tabs).ids();
    if (currentIds.length < 2) {
      const previousIdSet = new Set(previousOrder);
      return new StoredWorkspaceTabOrder([
        ...previousOrder,
        ...currentIds.filter((tabId) => !previousIdSet.has(tabId)),
      ]);
    }
    const currentIdSet = new Set(currentIds);
    return new StoredWorkspaceTabOrder([
      ...currentIds,
      ...previousOrder.filter((tabId) => !currentIdSet.has(tabId)),
    ]);
  }

  applyTo(tabs: readonly WorkspaceObjectTab[]) {
    const sanitizedTabs = SanitizedWorkspaceTabs.from(tabs).tabs;
    if (this.ids.length === 0) {
      return sanitizedTabs;
    }
    const tabById = new Map(sanitizedTabs.map((tab) => [tab.id, tab]));
    const next: WorkspaceObjectTab[] = [];
    for (const tabId of this.ids) {
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

  write(storage: WorkspaceTabStorage | null | undefined) {
    if (!storage) {
      return;
    }

    try {
      storage.setItem(
        WORKSPACE_TAB_ORDER_STORAGE_KEY,
        JSON.stringify(this.ids),
      );
    } catch {
      // Best-effort preference only; tab switching should never depend on storage.
    }
  }
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
      tab.kind === "diff" ||
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
