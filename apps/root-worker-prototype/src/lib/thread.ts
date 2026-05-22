import type { TaskFilter, Thread, ThreadItem, TodoCardItem, TreeNode, Turn } from "../types";

export function pickInitialThread(threads: Thread[]) {
  return [...threads].sort((left, right) => right.updatedAt - left.updatedAt)[0] ?? null;
}

export function pickInitialRootThread(threads: Thread[]) {
  return pickInitialThread(threads.filter((thread) => isRootThread(thread) && getThreadPath(thread) === "/root"));
}

export function buildAgentTree(
  threads: Thread[],
  selectedTreeRootId: string | null,
): TreeNode[] {
  if (threads.length === 0 && !selectedTreeRootId) {
    return [];
  }

  const threadById = new Map(threads.map((thread) => [thread.id, thread]));
  const roots = threads.filter((thread) => !getParentThreadId(thread));
  const byParent = new Map<string, string[]>();
  for (const thread of threads) {
    const parentId = getParentThreadId(thread);
    if (!parentId) {
      continue;
    }
    const list = byParent.get(parentId) ?? [];
    list.push(thread.id);
    byParent.set(parentId, list);
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
      label: thread ? getTreeNodeLabel(thread) : trimThreadId(threadId),
      path: thread ? getTreeNodeSubtitle(thread) : "Starting worker…",
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

export function getTreeNodeLabel(thread: Thread) {
  if (isRootThread(thread)) {
    return getThreadPath(thread);
  }
  return getThreadPath(thread).split("/").filter(Boolean).at(-1) ?? getThreadLabel(thread);
}

export function getTreeNodeSubtitle(thread: Thread) {
  return thread.agentRole ?? getAgentRoleLabel(thread);
}

export function getThreadPath(thread: Thread) {
  const threadSpawn = getThreadSpawnSource(thread);
  if (threadSpawn) {
    return (
      threadSpawn.agentPath ??
      threadSpawn.agent_path ??
      `/root/${thread.agentNickname ?? thread.id.slice(0, 6)}`
    );
  }
  return "/root";
}

export function isSubagentThread(thread: Thread) {
  return getThreadSpawnSource(thread) !== null || thread.threadSource === "subagent";
}

export function getParentThreadId(thread: Thread) {
  const threadSpawn = getThreadSpawnSource(thread);
  if (threadSpawn) {
    return threadSpawn.parentThreadId ?? threadSpawn.parent_thread_id ?? null;
  }
  return null;
}

export function isRootThread(thread: Thread) {
  return !getParentThreadId(thread);
}

export function getTreeRootThreadId(
  threads: Thread[],
  threadId: string,
) {
  const byId = new Map(threads.map((thread) => [thread.id, thread]));
  let current = byId.get(threadId) ?? null;
  let currentId = current?.id ?? threadId;
  let parentId = current ? getParentThreadId(current) : null;

  while (parentId) {
    const parent = byId.get(parentId);
    if (!parent) {
      return currentId;
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
) {
  const byId = new Map(threads.map((thread) => [thread.id, thread]));
  let depth = 0;
  let current = byId.get(threadId) ?? null;
  let parentId = current ? getParentThreadId(current) : null;

  while (parentId) {
    depth += 1;
    current = byId.get(parentId) ?? null;
    parentId = current ? getParentThreadId(current) : null;
  }

  return depth;
}

export function getThreadSubtreeIds(
  threads: Thread[],
  rootThreadId: string,
) {
  const byParent = new Map<string, string[]>();
  for (const thread of threads) {
    const parentId = getParentThreadId(thread);
    if (!parentId) {
      continue;
    }
    const children = byParent.get(parentId) ?? [];
    children.push(thread.id);
    byParent.set(parentId, children);
  }

  const subtreeIds = new Set<string>();
  const pending = [rootThreadId];
  while (pending.length > 0) {
    const threadId = pending.pop();
    if (!threadId || subtreeIds.has(threadId)) {
      continue;
    }
    subtreeIds.add(threadId);
    pending.push(...(byParent.get(threadId) ?? []));
  }

  return subtreeIds;
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

export function getThreadModelLabel(thread: Thread | null) {
  if (!thread) {
    return "unknown";
  }
  return thread.model ?? thread.modelProvider ?? "unknown";
}

export function getThreadReasoningLabel(thread: Thread | null) {
  if (!thread) {
    return "default";
  }
  return thread.reasoningEffort ?? "default";
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

function getThreadSpawnSource(thread: Thread): Record<string, unknown> | null {
  const source = thread.source;
  if (!source || typeof source !== "object") {
    return null;
  }

  const subAgentSource =
    ("subAgent" in source ? source.subAgent : undefined) ??
    ("subagent" in source ? source.subagent : undefined);
  if (!subAgentSource || typeof subAgentSource !== "object") {
    return null;
  }

  const threadSpawn =
    ("thread_spawn" in subAgentSource ? subAgentSource.thread_spawn : undefined) ??
    ("threadSpawn" in subAgentSource ? subAgentSource.threadSpawn : undefined);
  return threadSpawn && typeof threadSpawn === "object" ? (threadSpawn as Record<string, unknown>) : null;
}
