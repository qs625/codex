function mergeThreadSnapshots(existing, next) {
  const normalizedNext = normalizeThreadSnapshot(next);
  if (!existing) {
    return normalizedNext;
  }

  const turns = mergeTurns(existing.turns ?? [], normalizedNext.turns ?? []);
  const tokenUsage =
    normalizedNext.tokenUsage ??
    existing.tokenUsage ??
    existing.threadUsage?.tokenUsage ??
    null;
  const contextUsage =
    normalizedNext.contextUsage ??
    existing.contextUsage ??
    existing.threadUsage?.contextUsage ??
    null;

  return {
    ...existing,
    ...normalizedNext,
    threadUsage: {
      tokenUsage: normalizedNext.threadUsage?.tokenUsage ?? tokenUsage,
      contextUsage: normalizedNext.threadUsage?.contextUsage ?? contextUsage,
    },
    tokenUsage,
    contextUsage,
    turns,
  };
}

function normalizeThreadSnapshot(thread) {
  const turns = (thread.turns ?? []).reduce((normalizedTurns, turn) => {
    const normalizedTurn = normalizeTurnSnapshot(turn);
    const existingIndex = normalizedTurns.findIndex(
      (candidate) => candidate.id === normalizedTurn.id,
    );
    if (existingIndex !== -1) {
      return normalizedTurns.map((existing, index) =>
        index === existingIndex
          ? mergeTurn(existing, normalizedTurn)
          : existing,
      );
    }

    const incomingMatcher = createTurnItemMatcher(
      buildTurnItemIndex([
        { turn: normalizedTurn, items: normalizedTurn.items ?? [] },
      ]),
    );
    const retainedExistingTurns = normalizedTurns.flatMap((existing) =>
      getRetainedUnmatchedTurn(existing, incomingMatcher),
    );
    const existingMatcher = createTurnItemMatcher(
      buildTurnItemIndex(
        retainedExistingTurns.map((existing) => ({
          turn: existing,
          items: existing.items ?? [],
        })),
      ),
    );
    return [
      ...retainedExistingTurns,
      ...getRetainedUnmatchedTurn(normalizedTurn, existingMatcher),
    ];
  }, []);

  if (
    turns.length === (thread.turns ?? []).length &&
    turns.every((turn, index) => turn === thread.turns?.[index])
  ) {
    return thread;
  }
  return { ...thread, turns };
}

function normalizeTurnSnapshot(turn) {
  const items = (turn.items ?? []).reduce((normalizedItems, item) => {
    const normalizedItem = normalizeThreadItemSnapshot(item);
    if (!normalizedItem) {
      return normalizedItems;
    }
    const existingIndex = findMatchingThreadItemIndex(
      normalizedItems,
      normalizedItem,
    );
    if (existingIndex === -1) {
      return [...normalizedItems, normalizedItem];
    }
    return normalizedItems.map((existing, index) =>
      index === existingIndex
        ? mergeThreadItem(existing, normalizedItem)
        : existing,
    );
  }, []);

  if (
    items.length === (turn.items ?? []).length &&
    items.every((item, index) => item === turn.items?.[index])
  ) {
    return turn;
  }
  return { ...turn, items };
}

function mergeTurns(existingTurns, nextTurns) {
  const nextTurnIds = new Set(nextTurns.map((turn) => turn.id));
  const turns = nextTurns.map((turn) => {
    const existingTurn = existingTurns.find(
      (candidate) => candidate.id === turn.id,
    );
    return existingTurn ? mergeTurn(existingTurn, turn) : turn;
  });
  const nextItemsIndex = buildTurnItemIndex(
    turns.map((turn) => ({ turn, items: turn.items ?? [] })),
  );
  const nextItemsMatcher = createTurnItemMatcher(nextItemsIndex);

  for (const turn of existingTurns) {
    if (nextTurnIds.has(turn.id)) {
      continue;
    }
    turns.push(...getRetainedUnmatchedTurn(turn, nextItemsMatcher));
  }

  return turns;
}

function getRetainedUnmatchedTurn(turn, matcher) {
  const normalizedItems = (turn.items ?? [])
    .map(normalizeThreadItemSnapshot);
  const normalizedTurn =
    normalizedItems.length === (turn.items ?? []).length &&
    normalizedItems.every((item, index) => item === turn.items?.[index])
    ? turn
    : { ...turn, items: normalizedItems };

  if (!isTurnInFlight(turn) && !isLiveDerivedCompletedAgentTurn(turn)) {
    return [normalizedTurn];
  }

  const items = normalizedItems.filter(
    (item) => !consumeMatchingTurnItem(matcher, turn, item),
  );
  if (items.length === 0) {
    return [];
  }
  return [
    items.length === normalizedItems.length
      ? normalizedTurn
      : { ...turn, items },
  ];
}

function isLiveDerivedCompletedAgentTurn(turn) {
  return (
    turn.status === "completed" &&
    (turn.items ?? []).length > 0 &&
    (turn.items ?? []).every(
      (item) =>
        item.type === "agentMessage" || isCollabCompletionNotificationItem(item),
    ) &&
    (turn.itemsView !== "full" ||
      (turn.items ?? []).every(isCollabCompletionNotificationItem))
  );
}

function mergeTurn(existing, next) {
  const existingItems = (existing.items ?? []).map(normalizeThreadItemSnapshot);
  const nextItems = (next.items ?? []).map(normalizeThreadItemSnapshot);
  const existingItemsById = new Map(
    existingItems.map((item) => [item.id, item]),
  );
  const mergedItems = nextItems.map((item) => {
    const existingItem = existingItemsById.get(item.id);
    return existingItem ? mergeThreadItem(existingItem, item) : item;
  });

  if (isTurnInFlight(existing) || isTurnInFlight(next)) {
    const mergedItemsMatcher = createTurnItemMatcher(
      buildTurnItemIndex([{ turn: next, items: mergedItems }]),
    );
    for (const item of existingItems) {
      if (!consumeMatchingTurnItem(mergedItemsMatcher, existing, item)) {
        insertTurnItemByTimestamp(mergedItems, item);
      }
    }
  }

  return {
    ...existing,
    ...next,
    items: mergedItems,
  };
}

function insertTurnItemByTimestamp(items, item) {
  const timestampMs = threadItemOrderTimestampMs(item);
  if (timestampMs === null) {
    items.push(item);
    return;
  }

  const insertIndex = items.findIndex((candidate) => {
    const candidateTimestampMs = threadItemOrderTimestampMs(candidate);
    return candidateTimestampMs !== null && candidateTimestampMs > timestampMs;
  });
  if (insertIndex === -1) {
    items.push(item);
    return;
  }

  items.splice(insertIndex, 0, item);
}

function threadItemOrderTimestampMs(item) {
  const timestampMs =
    item.startedAtMs ??
    item.completedAtMs ??
    ("createdAtMs" in item ? item.createdAtMs : null);
  return typeof timestampMs === "number" && Number.isFinite(timestampMs)
    ? timestampMs
    : null;
}

function mergeThreadItem(existing, next) {
  const normalizedExisting = normalizeThreadItemSnapshot(existing);
  const normalizedNext = normalizeThreadItemSnapshot(next);
  if (!normalizedExisting) {
    return normalizedNext ?? next;
  }
  if (!normalizedNext) {
    return normalizedExisting;
  }
  if (normalizedExisting !== existing || normalizedNext !== next) {
    return mergeThreadItem(normalizedExisting, normalizedNext);
  }

  if (existing.type === "agentMessage" && next.type === "agentMessage") {
    return {
      ...existing,
      ...next,
      text: preferMoreCompleteText(existing.text, next.text),
    };
  }

  return next;
}

function normalizeThreadItemSnapshot(item) {
  return item;
}

function findMatchingThreadItemIndex(items, nextItem) {
  return items.findIndex((item) => item.id === nextItem.id);
}

function buildTurnItemIndex(entries) {
  const ids = new Set();

  for (const { items } of entries) {
    for (const item of items) {
      if (!item) {
        continue;
      }
      ids.add(item.id);
    }
  }

  return { ids };
}

function createTurnItemMatcher(index) {
  return {
    index,
  };
}

function consumeMatchingTurnItem(matcher, turn, item) {
  void turn;
  return matcher.index.ids.has(item.id);
}

function isCollabCompletionNotificationItem(item) {
  return (
    item.type === "collabAgentStatusUpdate" ||
    (item.type === "collabAgentMessage" && item.operation === "childCompletion")
  );
}

function isTurnInFlight(turn) {
  return turn.status === "running" || turn.status === "inProgress";
}

function preferMoreCompleteText(existing, next) {
  if (existing === next) {
    return next;
  }
  if (existing.startsWith(next)) {
    return existing;
  }
  return next;
}

module.exports = {
  mergeThreadSnapshots,
  normalizeThreadSnapshot,
};
