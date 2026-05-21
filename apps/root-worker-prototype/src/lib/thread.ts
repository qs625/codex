import type { TaskFilter, Thread, ThreadItem, TodoCardItem, TreeNode, Turn } from "../types";

export function pickInitialThread(threads: Thread[]) {
  return [...threads].sort((left, right) => right.updatedAt - left.updatedAt)[0] ?? null;
}

export function pickInitialRootThread(threads: Thread[]) {
  return pickInitialThread(threads.filter(isRootThread));
}

export function buildAgentTree(
  threads: Thread[],
  cachedThreadParents: Record<string, string>,
  selectedTreeRootId: string | null,
): TreeNode[] {
  if (threads.length === 0 && !selectedTreeRootId) {
    return [];
  }

  const threadById = new Map(threads.map((thread) => [thread.id, thread]));
  const roots = threads.filter((thread) => !getParentThreadId(thread));
  const byParent = new Map<string, string[]>();
  for (const thread of threads) {
    const parentId = getParentThreadId(thread) ?? cachedThreadParents[thread.id] ?? null;
    if (!parentId) {
      continue;
    }
    const list = byParent.get(parentId) ?? [];
    list.push(thread.id);
    byParent.set(parentId, list);
  }

  for (const [childId, parentId] of Object.entries(cachedThreadParents)) {
    const list = byParent.get(parentId) ?? [];
    if (!list.includes(childId)) {
      list.push(childId);
      byParent.set(parentId, list);
    }
  }

  const buildNode = (threadId: string): TreeNode => {
    const thread = threadById.get(threadId) ?? null;
    const childIds = [...(byParent.get(threadId) ?? [])].sort((left, right) => {
      const leftThread = threadById.get(left);
      const rightThread = threadById.get(right);
      if (leftThread && rightThread) {
        return leftThread.createdAt - rightThread.createdAt;
      }
      if (leftThread) {
        return -1;
      }
      if (rightThread) {
        return 1;
      }
      return left.localeCompare(right);
    });

    return {
      key: threadId,
      label: thread ? getThreadLabel(thread) : trimThreadId(threadId),
      path: thread ? getThreadPath(thread) : "Starting worker…",
      thread,
      threadId,
      isPlaceholder: thread === null,
      children: childIds.map(buildNode),
    };
  };

  const rootIds = [
    ...new Set([
      ...roots.map((thread) => thread.id),
      ...(selectedTreeRootId ? [selectedTreeRootId] : []),
    ]),
  ];

  return rootIds
    .sort((left, right) => {
      const leftThread = threadById.get(left);
      const rightThread = threadById.get(right);
      if (leftThread && rightThread) {
        return leftThread.createdAt - rightThread.createdAt;
      }
      if (leftThread) {
        return -1;
      }
      if (rightThread) {
        return 1;
      }
      return left.localeCompare(right);
    })
    .map(buildNode);
}

export function buildTodoItems(threads: Thread[], filter: TaskFilter): TodoCardItem[] {
  const items = [...threads]
    .sort((left, right) => right.updatedAt - left.updatedAt)
    .map((thread) => {
      const status = mapTaskStatus(thread.status);
      return {
        id: thread.id,
        title: getThreadLabel(thread),
        ownerPath: getThreadPath(thread),
        status,
        statusLabel: capitalize(status),
        updatedLabel: formatUpdatedLabel(thread.updatedAt),
        summary: thread.preview,
        threadId: thread.id,
      };
    });

  if (filter === "all") {
    return items;
  }
  return items.filter((item) => item.status === filter);
}

export function getThreadLabel(thread: Thread) {
  const path = getThreadPath(thread);
  const last = path.split("/").filter(Boolean).at(-1);
  return thread.name ?? thread.agentNickname ?? last ?? "root";
}

export function getThreadPath(thread: Thread) {
  const source = thread.source;
  if (typeof source === "object" && "subAgent" in source) {
    const subAgent = source.subAgent;
    if (typeof subAgent === "object" && "thread_spawn" in subAgent) {
      return subAgent.thread_spawn.agent_path ?? `/root/${thread.agentNickname ?? thread.id.slice(0, 6)}`;
    }
  }
  return "/root";
}

export function getParentThreadId(thread: Thread) {
  const source = thread.source;
  if (typeof source === "object" && "subAgent" in source) {
    const subAgent = source.subAgent;
    if (typeof subAgent === "object" && "thread_spawn" in subAgent) {
      return subAgent.thread_spawn.parent_thread_id;
    }
  }
  return null;
}

export function isRootThread(thread: Thread) {
  return !getParentThreadId(thread);
}

export function getTreeRootThreadId(
  threads: Thread[],
  threadId: string,
  cachedThreadParents: Record<string, string> = {},
) {
  const byId = new Map(threads.map((thread) => [thread.id, thread]));
  let current = byId.get(threadId) ?? null;
  let currentId = current?.id ?? threadId;
  let parentId = current ? getParentThreadId(current) : cachedThreadParents[currentId] ?? null;

  while (parentId) {
    const parent = byId.get(parentId);
    if (!parent) {
      currentId = parentId;
      parentId = cachedThreadParents[currentId] ?? null;
      continue;
    }
    current = parent;
    currentId = parent.id;
    parentId = getParentThreadId(parent);
  }

  return current?.id ?? currentId;
}

export function getThreadDepth(
  threads: Thread[],
  threadId: string,
  cachedThreadParents: Record<string, string> = {},
) {
  const byId = new Map(threads.map((thread) => [thread.id, thread]));
  let depth = 0;
  let current = byId.get(threadId) ?? null;
  let parentId = current ? getParentThreadId(current) : cachedThreadParents[threadId] ?? null;

  while (parentId) {
    depth += 1;
    current = byId.get(parentId) ?? null;
    parentId = current ? getParentThreadId(current) : cachedThreadParents[parentId] ?? null;
  }

  return depth;
}

export function getCachedDescendantIds(
  cachedThreadParents: Record<string, string>,
  rootThreadId: string,
) {
  const descendants = new Set<string>();
  let changed = true;

  while (changed) {
    changed = false;
    for (const [childId, parentId] of Object.entries(cachedThreadParents)) {
      if (parentId === rootThreadId || descendants.has(parentId)) {
        if (!descendants.has(childId)) {
          descendants.add(childId);
          changed = true;
        }
      }
    }
  }

  return descendants;
}

export function pruneCachedThreadParents(
  threads: Thread[],
  cachedThreadParents: Record<string, string>,
) {
  const liveThreadIds = new Set(threads.map((thread) => thread.id));
  const next: Record<string, string> = {};

  for (const [childId, parentId] of Object.entries(cachedThreadParents)) {
    if (liveThreadIds.has(childId) || liveThreadIds.has(parentId)) {
      next[childId] = parentId;
    }
  }

  return next;
}

export function getAgentRoleLabel(thread: Thread) {
  const source = thread.source;
  if (typeof source === "object" && "subAgent" in source) {
    return "Worker Agent";
  }
  return "Root Agent";
}

export function getPresenceLabel(status: string) {
  return threadStatusClass(status) === "doing" ? "Online" : "Idle";
}

export function updateThreadTurn(thread: Thread, turn: Turn) {
  const turns = thread.turns.some((existing) => existing.id === turn.id)
    ? thread.turns.map((existing) => (existing.id === turn.id ? mergeTurn(existing, turn) : existing))
    : [...thread.turns, turn];
  return { ...thread, turns };
}

export function updateThreadItem(thread: Thread, turnId: string, item: ThreadItem) {
  return {
    ...thread,
    turns: thread.turns.map((turn) => {
      if (turn.id !== turnId) {
        return turn;
      }
      const items = turn.items.some((existing) => existing.id === item.id)
        ? turn.items.map((existing) => (existing.id === item.id ? item : existing))
        : [...turn.items, item];
      return { ...turn, items };
    }),
  };
}

export function appendAgentDelta(thread: Thread, turnId: string, itemId: string, delta: string) {
  const turns = thread.turns.some((turn) => turn.id === turnId)
    ? thread.turns.map((turn) => {
        if (turn.id !== turnId) {
          return turn;
        }
        const hasItem = turn.items.some((item) => item.id === itemId);
        const items = hasItem
          ? turn.items.map((item) =>
              item.id === itemId && item.type === "agentMessage"
                ? { ...item, text: item.text + delta }
                : item,
            )
          : [
              ...turn.items,
              {
                type: "agentMessage",
                id: itemId,
                text: delta,
                phase: null,
                memoryCitation: null,
              } satisfies ThreadItem,
            ];
        return { ...turn, items };
      })
    : [
        ...thread.turns,
        {
          id: turnId,
          items: [
            {
              type: "agentMessage",
              id: itemId,
              text: delta,
              phase: null,
              memoryCitation: null,
            },
          ],
          itemsView: "full",
          status: "running",
          error: null,
          startedAt: null,
          completedAt: null,
          durationMs: null,
        },
      ];
  return { ...thread, turns };
}

export function mergeTurn(existing: Turn, next: Turn): Turn {
  const mergedItems = [...existing.items];
  for (const item of next.items) {
    const index = mergedItems.findIndex((candidate) => candidate.id === item.id);
    if (index === -1) {
      mergedItems.push(item);
    } else {
      mergedItems[index] = item;
    }
  }
  return {
    ...existing,
    ...next,
    items: mergedItems,
  };
}

export function upsertThread(threads: Thread[], next: Thread) {
  const existing = threads.find((thread) => thread.id === next.id);
  if (!existing) {
    return [...threads, next];
  }
  return threads.map((thread) => (thread.id === next.id ? { ...thread, ...next } : thread));
}

export function threadStatusClass(status: string) {
  switch (status) {
    case "running":
      return "doing";
    case "interrupted":
    case "errored":
      return "blocked";
    case "completed":
      return "done";
    default:
      return "todo";
  }
}

export function countDescendants(node: TreeNode): number {
  return node.children.reduce((count, child) => count + 1 + countDescendants(child), 0);
}

export function formatClockTime(unixSeconds: number) {
  return new Intl.DateTimeFormat("en-US", {
    hour: "numeric",
    minute: "2-digit",
  }).format(new Date(unixSeconds * 1000));
}

export function formatUpdatedLabel(unixSeconds: number) {
  const target = new Date(unixSeconds * 1000);
  const now = new Date();
  const diffDays = Math.floor((now.getTime() - target.getTime()) / 86_400_000);
  if (diffDays <= 0) {
    return `Updated ${formatClockTime(unixSeconds)}`;
  }
  if (diffDays === 1) {
    return "Updated Yesterday";
  }
  return `Updated ${target.toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
  })}`;
}

export function capitalize(value: string) {
  return value.charAt(0).toUpperCase() + value.slice(1);
}

export function trimPath(path: string) {
  return path.split("/").slice(-2).join("/");
}

export function trimThreadId(threadId: string) {
  return threadId.slice(0, 8);
}

function mapTaskStatus(status: string): Exclude<TaskFilter, "all"> {
  const mapped = threadStatusClass(status);
  if (mapped === "blocked" || mapped === "done" || mapped === "doing") {
    return mapped;
  }
  return "todo";
}
