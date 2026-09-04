import type { ConversationCell, ConversationEntry } from "../types";

export type LoadedCompactConversationDetails = {
  archivedCells: ConversationCell[];
  archivedEntryCount: number;
  replacementHistoryEntries: ConversationEntry[];
  replacementHistoryCells: ConversationCell[];
};

export type ConversationCellBuildOptions = {
  compactDetailsById?: Readonly<
    Record<string, LoadedCompactConversationDetails | undefined>
  >;
  terminalTurnIds?: ReadonlySet<string>;
};

export function buildConversationCells(
  entries: ConversationEntry[],
  previousCells?: ConversationCell[] | null,
  options?: ConversationCellBuildOptions,
): ConversationCell[] {
  const cells = foldTerminalTurnProcessCells(
    buildConversationCellsForSegment(entries, options),
    options?.terminalTurnIds,
  );
  return reuseConversationCells(cells, previousCells);
}

export function extractCompactConversationDetails(
  entries: ConversationEntry[],
  compactEntryId: string,
): LoadedCompactConversationDetails | null {
  const priorEntries: ConversationEntry[] = [];

  for (const entry of entries) {
    if (entry.kind === "compact" && entry.id === compactEntryId) {
      const prefixCells = buildConversationCellsForSegment(priorEntries);
      const archivedCells = collectArchivedCellsForCompact(
        prefixCells,
        entry.turnId,
      );
      const replacementHistoryEntries = entry.replacementHistoryEntries ?? [];
      return {
        archivedCells,
        archivedEntryCount: countConversationEntries(archivedCells),
        replacementHistoryEntries,
        replacementHistoryCells:
          buildConversationCellsForSegment(replacementHistoryEntries),
      };
    }
    priorEntries.push(entry);
  }

  return null;
}

function buildConversationCellsForSegment(
  entries: ConversationEntry[],
  options?: ConversationCellBuildOptions,
): ConversationCell[] {
  const cells: ConversationCell[] = [];
  let entryIndex = 0;

  while (entryIndex < entries.length) {
    const entry = entries[entryIndex];
    if (!entry) {
      entryIndex += 1;
      continue;
    }

    if (entry.kind === "compact") {
      const loadedDetails = options?.compactDetailsById?.[entry.id];
      const hydratedArchivedCells =
        loadedDetails?.archivedCells ?? entry.archivedCells ?? [];
      const localArchivedCells = collectArchivedCellsForCompact(cells, entry.turnId);
      const archivedCells =
        localArchivedCells.length > 0
          ? localArchivedCells
          : hydratedArchivedCells;
      const archivedEntryCount =
        localArchivedCells.length > 0
          ? countConversationEntries(localArchivedCells)
          : loadedDetails?.archivedEntryCount ?? entry.archivedEntryCount;
      const compactTurnCells = collectCompactTurnCellsBeforeCompact(
        cells,
        entry.turnId,
        options?.terminalTurnIds,
      );
      if (localArchivedCells.length > 0 || compactTurnCells.length > 0) {
        const visibleCells = cells.filter(
          (cell) =>
            !shouldArchiveCellForCompact(cell, entry.turnId) &&
            !shouldDiscardCellBeforeCompact(
              cell,
              entry.turnId,
              options?.terminalTurnIds,
            ),
        );
        cells.length = 0;
        cells.push(...visibleCells);
      }
      cells.push({
        id: entry.id,
        kind: "compact",
        entries: [
          loadedDetails
            ? {
                ...entry,
                archivedCells,
                archivedEntryCount,
                replacementHistoryEntries:
                  loadedDetails.replacementHistoryEntries,
                replacementHistoryCells: loadedDetails.replacementHistoryCells,
              }
            : archivedCells.length > 0 || archivedEntryCount !== undefined
              ? {
                  ...entry,
                  archivedCells,
                  archivedEntryCount,
                }
              : entry,
        ],
      });
      entryIndex += 1;
      continue;
    }

    const nextCellEntries = [entry];
    while (
      entryIndex + nextCellEntries.length < entries.length &&
      shouldMergeConversationEntry(
        {
          id: nextCellEntries[0]?.id ?? entry.id,
          kind: nextCellEntries[0]?.kind ?? entry.kind,
          entries: nextCellEntries,
        },
        entries[entryIndex + nextCellEntries.length]!,
      )
    ) {
      nextCellEntries.push(entries[entryIndex + nextCellEntries.length]!);
    }

    cells.push({
      id: nextCellEntries[0]?.id ?? entry.id,
      kind: nextCellEntries[0]?.kind ?? entry.kind,
      entries: nextCellEntries,
    });

    entryIndex += nextCellEntries.length;
  }

  return cells;
}

function foldTerminalTurnProcessCells(
  cells: ConversationCell[],
  terminalTurnIds: ReadonlySet<string> | undefined,
): ConversationCell[] {
  if (!terminalTurnIds || terminalTurnIds.size === 0) {
    return cells;
  }

  const foldedCells: ConversationCell[] = [];
  let pendingProcessCells: ConversationCell[] = [];

  const flushPendingProcessCells = () => {
    if (pendingProcessCells.length === 0) {
      return;
    }
    foldedCells.push(buildTurnProcessCell(pendingProcessCells));
    pendingProcessCells = [];
  };

  for (const cell of cells) {
    if (!shouldFoldTerminalProcessCell(cell, terminalTurnIds)) {
      flushPendingProcessCells();
      foldedCells.push(cell);
      continue;
    }

    const pendingTurnId = pendingProcessCells[0]?.entries[0]?.turnId;
    const nextTurnId = cell.entries[0]?.turnId;
    if (pendingTurnId && nextTurnId && pendingTurnId !== nextTurnId) {
      flushPendingProcessCells();
    }
    pendingProcessCells.push(cell);
  }

  flushPendingProcessCells();
  return foldedCells;
}

function shouldFoldTerminalProcessCell(
  cell: ConversationCell,
  terminalTurnIds: ReadonlySet<string>,
) {
  if (cell.kind === "turnProcess") {
    return false;
  }
  if (cell.kind === "compact") {
    return false;
  }
  if (cell.entries.length === 0) {
    return false;
  }
  const turnId = cell.entries[0]?.turnId;
  if (!turnId || !terminalTurnIds.has(turnId)) {
    return false;
  }
  if (cell.entries.some((entry) => entry.turnId !== turnId)) {
    return false;
  }
  return !cell.entries.some(isTerminalTurnVisibleAnchorEntry);
}

function isTerminalTurnVisibleAnchorEntry(entry: ConversationEntry) {
  return (
    entry.isTurnTerminalOutcome === true ||
    entry.sourceItemType === "userMessage" ||
    entry.sourceItemType === "agentMessage"
  );
}

function buildTurnProcessCell(cells: ConversationCell[]): ConversationCell {
  const entries = cells.flatMap((cell) => cell.entries);
  const firstEntry = entries[0];
  const lastEntry = entries.at(-1);
  return {
    id: `turn-process:${firstEntry?.turnId ?? "unknown"}:${firstEntry?.id ?? "start"}:${lastEntry?.id ?? "end"}:${entries.length}`,
    kind: "turnProcess",
    entries,
    collapsedCells: cells,
  };
}

function countConversationEntries(cells: ConversationCell[]) {
  return cells.reduce(
    (count, cell) =>
      count +
      cell.entries.reduce(
        (entryCount, entry) => {
          if (entry.kind === "archive") {
            return entryCount + (entry.archivedEntryCount ?? 0);
          }
          if (entry.kind === "compact") {
            return (
              entryCount +
              (entry.archivedEntryCount ?? 0) +
              1 +
              countConversationEntries(entry.replacementHistoryCells ?? [])
            );
          }
          return entryCount + 1;
        },
        0,
      ),
    0,
  );
}

function reuseConversationCells(
  cells: ConversationCell[],
  previousCells?: ConversationCell[] | null,
): ConversationCell[] {
  if (!previousCells) {
    return cells;
  }

  const previousCellsByKey = new Map(
    previousCells.map((cell) => [conversationCellReuseKey(cell), cell]),
  );

  return cells.map((cell) => {
    const existingCell = previousCellsByKey.get(conversationCellReuseKey(cell));
    if (
      existingCell &&
      existingCell.entries.length === cell.entries.length &&
      existingCell.entries.every(
        (entry, entryIndex) => entry === cell.entries[entryIndex],
      )
    ) {
      return existingCell;
    }
    return cell;
  });
}

function conversationCellReuseKey(cell: ConversationCell) {
  return `${cell.kind}:${cell.id}`;
}

function shouldMergeConversationEntry(
  cell: ConversationCell,
  nextEntry: ConversationEntry,
) {
  const previousEntry = cell.entries.at(-1);
  if (!previousEntry) {
    return false;
  }

  if (nextEntry.kind === "compact") {
    return false;
  }

  if (cell.kind === "artifact" || nextEntry.kind === "artifact") {
    return false;
  }

  if (cell.kind === "tool" && nextEntry.kind === "tool") {
    if (previousEntry.turnId !== nextEntry.turnId) {
      return false;
    }
    if (previousEntry.isReplacementHistory !== nextEntry.isReplacementHistory) {
      return false;
    }
    if (
      isStandaloneNotificationEntry(previousEntry) ||
      isStandaloneNotificationEntry(nextEntry)
    ) {
      return canMergeStandaloneToolNotifications(previousEntry, nextEntry);
    }
    return previousEntry.toolCategory === nextEntry.toolCategory;
  }

  if (
    cell.kind === "message" &&
    nextEntry.kind === "message" &&
    previousEntry.role === "agent" &&
    nextEntry.role === "agent" &&
    previousEntry.sourceItemType === "agentMessage" &&
    nextEntry.sourceItemType === "agentMessage"
  ) {
    if (previousEntry.turnId !== nextEntry.turnId) {
      return false;
    }
    if (previousEntry.isReplacementHistory !== nextEntry.isReplacementHistory) {
      return false;
    }
    if (previousEntry.messagePhase !== nextEntry.messagePhase) {
      return false;
    }
    return true;
  }

  return false;
}

function canMergeStandaloneToolNotifications(
  previousEntry: ConversationEntry,
  nextEntry: ConversationEntry,
) {
  return (
    isMergeableCommandNotificationEntry(previousEntry) &&
    isMergeableCommandNotificationEntry(nextEntry)
  );
}

function isMergeableCommandNotificationEntry(entry: ConversationEntry) {
  return entry.toolCategory === "commandNotification";
}

function isStandaloneNotificationEntry(entry: ConversationEntry) {
  return (
    entry.toolCategory === "commandNotification" ||
    entry.toolCategory === "childCompletion" ||
    entry.toolCategory === "subagentNotification"
  );
}

function collectArchivedCellsForCompact(
  cells: ConversationCell[],
  compactTurnId: string | undefined,
) {
  return cells.filter((cell) => shouldArchiveCellForCompact(cell, compactTurnId));
}

function collectCompactTurnCellsBeforeCompact(
  cells: ConversationCell[],
  compactTurnId: string | undefined,
  terminalTurnIds?: ReadonlySet<string>,
) {
  return cells.filter((cell) =>
    shouldDiscardCellBeforeCompact(cell, compactTurnId, terminalTurnIds),
  );
}

function shouldArchiveCellForCompact(
  cell: ConversationCell,
  compactTurnId: string | undefined,
) {
  if (!compactTurnId) {
    return false;
  }
  const cellTurnId = cell.entries.find((entry) => entry.turnId)?.turnId;
  return cellTurnId !== undefined && cellTurnId !== compactTurnId;
}

function shouldDiscardCellBeforeCompact(
  cell: ConversationCell,
  compactTurnId: string | undefined,
  terminalTurnIds?: ReadonlySet<string>,
) {
  if (!compactTurnId) {
    return false;
  }
  const cellTurnId = cell.entries.find((entry) => entry.turnId)?.turnId;
  if (cellTurnId !== compactTurnId) {
    return false;
  }
  if (
    terminalTurnIds?.has(compactTurnId) &&
    cell.entries.some(isTerminalTurnVisibleAnchorEntry)
  ) {
    return false;
  }
  return true;
}
