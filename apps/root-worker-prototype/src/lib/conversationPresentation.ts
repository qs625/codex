import type { ConversationCell, ConversationEntry } from "../types";

export function filterConversationCellsForDisplay(
  cells: ConversationCell[],
): ConversationCell[] {
  return cells.flatMap((cell) => {
    const entries = cell.entries
      .filter((entry) => entry.toolCategory !== "commandNotification")
      .map(filterConversationEntryForDisplay);
    return entries.length > 0 ? [{ ...cell, entries }] : [];
  });
}

function filterConversationEntryForDisplay(
  entry: ConversationEntry,
): ConversationEntry {
  return {
    ...entry,
    ...(entry.replacementHistoryCells
      ? {
          replacementHistoryCells: filterConversationCellsForDisplay(
            entry.replacementHistoryCells,
          ),
        }
      : {}),
    ...(entry.archivedCells
      ? { archivedCells: filterConversationCellsForDisplay(entry.archivedCells) }
      : {}),
  };
}
