import type { ConversationCell, ConversationEntry } from "../types";

const RESTART_RECOVERY_NOTICE_ID_PREFIX = "runtime-restart-recovery:";
const RUNTIME_RESTART_REQUEST_ID_PATTERN = /\bcall_[A-Za-z0-9]+\b/;

export function filterConversationCellsForDisplay(
  cells: ConversationCell[],
): ConversationCell[] {
  const topLevelRecoveryFacts = collectTopLevelRecoveryFactKeys(cells);
  return filterConversationCellsForDisplayWithState(
    cells,
    {
      topLevelSeenRecoveryFacts: new Set(),
      nestedSeenRecoveryFacts: new Set(topLevelRecoveryFacts),
    },
    true,
  );
}

type RecoveryFactFilterState = {
  topLevelSeenRecoveryFacts: Set<string>;
  nestedSeenRecoveryFacts: Set<string>;
};

function filterConversationCellsForDisplayWithState(
  cells: ConversationCell[],
  state: RecoveryFactFilterState,
  isTopLevel: boolean,
): ConversationCell[] {
  const displayCells: ConversationCell[] = [];
  for (const cell of cells) {
    const entries: ConversationEntry[] = [];
    for (const entry of cell.entries) {
      if (
        !consumeRecoveryFactIfDuplicate(
          entry,
          isTopLevel
            ? state.topLevelSeenRecoveryFacts
            : state.nestedSeenRecoveryFacts,
        ) ||
        entry.toolCategory === "commandNotification"
      ) {
        continue;
      }
      entries.push(filterConversationEntryForDisplay(entry, state));
    }
    if (entries.length > 0) {
      displayCells.push({ ...cell, entries });
    }
  }
  return displayCells;
}

function filterConversationEntryForDisplay(
  entry: ConversationEntry,
  state: RecoveryFactFilterState,
): ConversationEntry {
  return {
    ...entry,
    ...(entry.replacementHistoryCells
      ? {
          replacementHistoryCells: filterConversationCellsForDisplayWithState(
            entry.replacementHistoryCells,
            state,
            false,
          ),
        }
      : {}),
    ...(entry.archivedCells
      ? {
          archivedCells: filterConversationCellsForDisplayWithState(
            entry.archivedCells,
            state,
            false,
          ),
        }
      : {}),
  };
}

function collectTopLevelRecoveryFactKeys(cells: ConversationCell[]) {
  const keys = new Set<string>();
  for (const cell of cells) {
    for (const entry of cell.entries) {
      const key = restartRecoveryFactKey(entry);
      if (key) {
        keys.add(key);
      }
    }
  }
  return keys;
}

function consumeRecoveryFactIfDuplicate(
  entry: ConversationEntry,
  seenRecoveryFacts: Set<string>,
) {
  const factKey = restartRecoveryFactKey(entry);
  if (!factKey) {
    return true;
  }
  if (seenRecoveryFacts.has(factKey)) {
    return false;
  }
  seenRecoveryFacts.add(factKey);
  return true;
}

function restartRecoveryFactKey(entry: ConversationEntry) {
  if (entry.kind !== "message" || entry.role !== "user") {
    return null;
  }
  if (entry.id.startsWith(RESTART_RECOVERY_NOTICE_ID_PREFIX)) {
    return entry.id;
  }
  if (!entry.text.includes("Runtime Capsule")) {
    return null;
  }
  const requestId = entry.text.match(RUNTIME_RESTART_REQUEST_ID_PATTERN)?.[0];
  return requestId ? `${RESTART_RECOVERY_NOTICE_ID_PREFIX}${requestId}` : null;
}
