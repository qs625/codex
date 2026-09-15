import type { ConversationCell, ConversationEntry } from "../types";

const RESTART_RECOVERY_NOTICE_ID_PREFIX = "runtime-restart-recovery:";
const RUNTIME_RESTART_REQUEST_ID_PATTERN = /\bcall_[A-Za-z0-9]+\b/;

export function filterConversationCellsForDisplay(
  cells: ConversationCell[],
): ConversationCell[] {
  const topLevelRecoveryFacts = collectRecoveryFactKeys(
    cells.flatMap((cell) => cell.entries),
  );
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
  return cells.flatMap((cell) => {
    const entries = cell.entries
      .filter((entry) =>
        consumeRecoveryFactIfDuplicate(
          entry,
          isTopLevel
            ? state.topLevelSeenRecoveryFacts
            : state.nestedSeenRecoveryFacts,
        ),
      )
      .filter((entry) => entry.toolCategory !== "commandNotification")
      .map((entry) => filterConversationEntryForDisplay(entry, state));
    return entries.length > 0 ? [{ ...cell, entries }] : [];
  });
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

function collectRecoveryFactKeys(entries: ConversationEntry[]) {
  const keys = new Set<string>();
  for (const entry of entries) {
    const key = restartRecoveryFactKey(entry);
    if (key) {
      keys.add(key);
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
