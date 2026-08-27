import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";

import {
  ArchivedHistoryRow,
  ArtifactRow,
  CompactRow,
  EventRow,
  MessageRow,
  ToolRow,
} from "./Conversation";
import {
  buildConversationVirtualLayout,
  estimateConversationCellHeight,
  findConversationWindow,
} from "../lib/conversationVirtualization";
import { planConversationHeightChangeScroll } from "../lib/conversationScroll";
import type { ConversationCell } from "../types";

type ConversationVirtualListProps = {
  cells: ConversationCell[];
  compactHistoryById: Readonly<
    Record<string, { isLoading: boolean; isExpanded: boolean; error: string | null }>
  >;
  containerRef: RefObject<HTMLDivElement | null>;
  focusedItem: { itemId: string; token: number } | null;
  onToggleCompactHistory: (entryId: string) => void;
  onOpenLocalFile: (target: string) => void;
  onOpenArtifactUrl: (url: string) => void;
  searchCurrentCellId: string | null;
  searchMatchCellIds: Set<string>;
};

type ViewportState = {
  scrollTop: number;
  viewportHeight: number;
};

type FocusedItemRequest = { itemId: string; token: number };

type PendingFocusRequest = FocusedItemRequest & {
  attempts: number;
};

export const MAX_FOCUSED_ITEM_SCROLL_ATTEMPTS = 3;

export function shouldHandleFocusedItemRequest({
  focusedItem,
  lastHandledRequest,
}: {
  focusedItem: FocusedItemRequest | null;
  lastHandledRequest: FocusedItemRequest | null;
}) {
  return (
    focusedItem !== null &&
    (lastHandledRequest === null ||
      lastHandledRequest.itemId !== focusedItem.itemId ||
      lastHandledRequest.token !== focusedItem.token)
  );
}

export function planFocusedItemScrollAttempt({
  attempts,
  targetMeasured,
  maxAttempts = MAX_FOCUSED_ITEM_SCROLL_ATTEMPTS,
}: {
  attempts: number;
  targetMeasured: boolean;
  maxAttempts?: number;
}) {
  const nextAttempts = attempts + 1;
  return {
    behavior: attempts === 0 ? "smooth" : "auto",
    nextAttempts,
    shouldComplete: targetMeasured || nextAttempts >= maxAttempts,
  } satisfies {
    behavior: ScrollBehavior;
    nextAttempts: number;
    shouldComplete: boolean;
  };
}

export function planConversationCellMeasurement({
  measuredHeight,
  previousHeight,
  wasMeasured,
}: {
  measuredHeight: number;
  previousHeight: number;
  wasMeasured: boolean;
}) {
  const roundedHeight = Math.ceil(measuredHeight);
  return {
    roundedHeight,
    heightChanged: roundedHeight !== previousHeight,
    shouldBumpHeightVersion: !wasMeasured || roundedHeight !== previousHeight,
  };
}

export function ConversationVirtualList({
  cells,
  compactHistoryById,
  containerRef,
  focusedItem,
  onToggleCompactHistory,
  onOpenLocalFile,
  onOpenArtifactUrl,
  searchCurrentCellId,
  searchMatchCellIds,
}: ConversationVirtualListProps) {
  const measuredHeightsRef = useRef<Map<string, number>>(new Map());
  const measuredCellIdsRef = useRef<Set<string>>(new Set());
  const layoutRef = useRef<ReturnType<
    typeof buildConversationVirtualLayout
  > | null>(null);
  const [heightVersion, setHeightVersion] = useState(0);
  const [viewport, setViewport] = useState<ViewportState>({
    scrollTop: 0,
    viewportHeight: 0,
  });
  const [openToolCellIds, setOpenToolCellIds] = useState<Set<string>>(
    () => new Set(),
  );
  const [selectedToolEntryIds, setSelectedToolEntryIds] = useState<
    Map<string, string>
  >(() => new Map());
  const [highlightedCellId, setHighlightedCellId] = useState<string | null>(
    null,
  );
  const lastHandledFocusRequestRef = useRef<{
    itemId: string;
    token: number;
  } | null>(null);
  const pendingFocusRequestRef = useRef<PendingFocusRequest | null>(null);

  useEffect(() => {
    const liveCellIds = new Set(cells.map((cell) => cell.id));
    const entryIdsByCellId = new Map(
      cells.map((cell) => [
        cell.id,
        new Set(cell.entries.map((entry) => entry.id)),
      ]),
    );
    let removedMeasuredHeights = false;

    for (const cellId of measuredHeightsRef.current.keys()) {
      if (!liveCellIds.has(cellId)) {
        measuredHeightsRef.current.delete(cellId);
        removedMeasuredHeights = true;
      }
    }
    for (const cellId of measuredCellIdsRef.current.keys()) {
      if (!liveCellIds.has(cellId)) {
        measuredCellIdsRef.current.delete(cellId);
        removedMeasuredHeights = true;
      }
    }

    if (removedMeasuredHeights) {
      setHeightVersion((version) => version + 1);
    }

    setOpenToolCellIds((current) => {
      const next = new Set(
        Array.from(current).filter((cellId) => liveCellIds.has(cellId)),
      );
      return next.size === current.size ? current : next;
    });

    setSelectedToolEntryIds((current) => {
      const next = new Map(
        Array.from(current.entries()).filter(
          ([cellId, entryId]) =>
            liveCellIds.has(cellId) &&
            (entryIdsByCellId.get(cellId)?.has(entryId) ?? false),
        ),
      );
      return next.size === current.size ? current : next;
    });
  }, [cells]);

  useLayoutEffect(() => {
    const container = containerRef.current;
    if (!container) {
      return;
    }

    let frameId = 0;
    const resizeObserver = new ResizeObserver(() => {
      cancelAnimationFrame(frameId);
      frameId = window.requestAnimationFrame(syncViewport);
    });

    const syncViewport = () => {
      setViewport((current) => {
        const next = {
          scrollTop: container.scrollTop,
          viewportHeight: container.clientHeight,
        };
        return current.scrollTop === next.scrollTop &&
          current.viewportHeight === next.viewportHeight
          ? current
          : next;
      });
    };

    const handleScroll = () => {
      cancelAnimationFrame(frameId);
      frameId = window.requestAnimationFrame(syncViewport);
    };

    syncViewport();
    resizeObserver.observe(container);
    container.addEventListener("scroll", handleScroll, { passive: true });

    return () => {
      cancelAnimationFrame(frameId);
      resizeObserver.disconnect();
      container.removeEventListener("scroll", handleScroll);
    };
  }, [containerRef]);

  useLayoutEffect(() => {
    const container = containerRef.current;
    if (!container) {
      return;
    }

    setViewport((current) => {
      const next = {
        scrollTop: container.scrollTop,
        viewportHeight: container.clientHeight,
      };
      return current.scrollTop === next.scrollTop &&
        current.viewportHeight === next.viewportHeight
        ? current
        : next;
    });
  }, [cells, containerRef, heightVersion]);

  const layout = useMemo(
    () => buildConversationVirtualLayout(cells, measuredHeightsRef.current),
    [cells, heightVersion],
  );
  layoutRef.current = layout;

  const focusedItemId = focusedItem?.itemId ?? null;
  const focusedItemToken = focusedItem?.token ?? null;

  useEffect(() => {
    const nextFocusedItem =
      focusedItemId && focusedItemToken !== null
        ? { itemId: focusedItemId, token: focusedItemToken }
        : null;

    if (!nextFocusedItem) {
      pendingFocusRequestRef.current = null;
      return;
    }

    if (
      shouldHandleFocusedItemRequest({
        focusedItem: nextFocusedItem,
        lastHandledRequest: lastHandledFocusRequestRef.current,
      })
    ) {
      const pending = pendingFocusRequestRef.current;
      if (
        !pending ||
        pending.itemId !== nextFocusedItem.itemId ||
        pending.token !== nextFocusedItem.token
      ) {
        pendingFocusRequestRef.current = {
          ...nextFocusedItem,
          attempts: 0,
        };
      }
    }

    const pendingRequest = pendingFocusRequestRef.current;
    if (!pendingRequest) {
      return;
    }
    const index = cells.findIndex((cell) =>
      cell.entries.some((entry) =>
        conversationEntryContainsId(entry, pendingRequest.itemId),
      ),
    );
    if (index === -1) {
      return;
    }

    const cell = cells[index];
    const container = containerRef.current;
    const top = layoutRef.current?.offsets[index] ?? 0;
    if (container) {
      const plan = planFocusedItemScrollAttempt({
        attempts: pendingRequest.attempts,
        targetMeasured: measuredCellIdsRef.current.has(cell.id),
      });
      container.scrollTo({
        top: Math.max(0, top - 24),
        behavior: plan.behavior,
      });
      pendingFocusRequestRef.current = plan.shouldComplete
        ? null
        : {
            ...pendingRequest,
            attempts: plan.nextAttempts,
          };
      if (plan.shouldComplete) {
        lastHandledFocusRequestRef.current = {
          itemId: pendingRequest.itemId,
          token: pendingRequest.token,
        };
      }
    }
    setHighlightedCellId(cell.id);
    const timeout = window.setTimeout(() => {
      setHighlightedCellId((current) => (current === cell.id ? null : current));
    }, 2000);
    return () => window.clearTimeout(timeout);
  }, [cells, containerRef, focusedItemId, focusedItemToken, heightVersion]);

  const visibleWindow = useMemo(
    () =>
      findConversationWindow(
        layout,
        viewport.scrollTop,
        viewport.viewportHeight,
      ),
    [layout, viewport],
  );

  function handleCellHeightChange(
    cell: ConversationCell,
    index: number,
    measuredHeight: number,
  ) {
    const previousHeight =
      measuredHeightsRef.current.get(cell.id) ??
      estimateConversationCellHeight(cell);
    const measurementPlan = planConversationCellMeasurement({
      measuredHeight,
      previousHeight,
      wasMeasured: measuredCellIdsRef.current.has(cell.id),
    });
    measuredCellIdsRef.current.add(cell.id);
    if (!measurementPlan.shouldBumpHeightVersion) {
      return;
    }
    if (!measurementPlan.heightChanged) {
      setHeightVersion((version) => version + 1);
      return;
    }

    measuredHeightsRef.current.set(cell.id, measurementPlan.roundedHeight);

    const container = containerRef.current;
    const currentTop = layoutRef.current?.offsets[index] ?? 0;
    const scrollPlan = container
      ? planConversationHeightChangeScroll({
          cellTop: currentTop,
          heightDelta: measurementPlan.roundedHeight - previousHeight,
          metrics: {
            scrollHeight: container.scrollHeight,
            clientHeight: container.clientHeight,
            scrollTop: container.scrollTop,
          },
        })
      : null;
    if (container && scrollPlan && scrollPlan.scrollTopAdjustment !== 0) {
      container.scrollTop += scrollPlan.scrollTopAdjustment;
    }

    setHeightVersion((version) => version + 1);

    if (container && scrollPlan?.shouldStickToBottom) {
      window.requestAnimationFrame(() => {
        container.scrollTop = container.scrollHeight;
      });
    }
  }

  function handleToolOpenChange(cellId: string, isOpen: boolean) {
    setOpenToolCellIds((current) => {
      const next = new Set(current);
      if (isOpen) {
        next.add(cellId);
      } else {
        next.delete(cellId);
      }
      if (next.size === current.size) {
        let identical = true;
        for (const value of next) {
          if (!current.has(value)) {
            identical = false;
            break;
          }
        }
        if (identical) {
          return current;
        }
      }
      return next;
    });

    if (!isOpen) {
      setSelectedToolEntryIds((current) => {
        if (!current.has(cellId)) {
          return current;
        }
        const next = new Map(current);
        next.delete(cellId);
        return next;
      });
    }
  }

  function handleToolEntrySelection(cellId: string, entryId: string | null) {
    setSelectedToolEntryIds((current) => {
      const existing = current.get(cellId) ?? null;
      if (existing === entryId) {
        return current;
      }
      const next = new Map(current);
      if (entryId) {
        next.set(cellId, entryId);
      } else {
        next.delete(cellId);
      }
      return next;
    });
  }

  return (
    <div
      className="conversation-virtual-list"
      style={{ height: `${layout.totalHeight}px` }}
    >
      {cells
        .slice(visibleWindow.startIndex, visibleWindow.endIndex)
        .map((cell, relativeIndex) => {
          const index = visibleWindow.startIndex + relativeIndex;
          return (
            <MeasuredConversationCell
              key={cell.id}
              highlighted={cell.id === highlightedCellId}
              searchCurrent={cell.id === searchCurrentCellId}
              searchMatch={searchMatchCellIds.has(cell.id)}
              top={layout.offsets[index] ?? 0}
              onHeightChange={(height) =>
                handleCellHeightChange(cell, index, height)
              }
            >
              {renderConversationCell(
                cell,
                compactHistoryById[cell.entries[0]?.id ?? cell.id] ?? null,
                openToolCellIds.has(cell.id),
                selectedToolEntryIds.get(cell.id) ?? null,
                onToggleCompactHistory,
                onOpenLocalFile,
                onOpenArtifactUrl,
                handleToolOpenChange,
                handleToolEntrySelection,
              )}
            </MeasuredConversationCell>
          );
        })}
    </div>
  );
}

function conversationEntryContainsId(
  entry: ConversationCell["entries"][number],
  targetId: string,
): boolean {
  if (entry.id === targetId) {
    return true;
  }

  if (
    entry.replacementHistoryEntries?.some((nestedEntry) =>
      conversationEntryContainsId(nestedEntry, targetId),
    )
  ) {
    return true;
  }

  if (
    entry.archivedCells?.some((cell) =>
      cell.entries.some((nestedEntry) =>
        conversationEntryContainsId(nestedEntry, targetId),
      ),
    )
  ) {
    return true;
  }

  return false;
}

function renderConversationCell(
  cell: ConversationCell,
  compactHistoryState:
    | { isLoading: boolean; isExpanded: boolean; error: string | null }
    | null,
  isToolOpen: boolean,
  selectedToolEntryId: string | null,
  onToggleCompactHistory: (entryId: string) => void,
  onOpenLocalFile: (target: string) => void,
  onOpenArtifactUrl: (url: string) => void,
  onToolOpenChange: (cellId: string, isOpen: boolean) => void,
  onToolEntrySelection: (cellId: string, entryId: string | null) => void,
) {
  if (cell.kind === "event") {
    return <EventRow entry={cell.entries[0]} />;
  }

  if (cell.kind === "archive") {
    return (
      <ArchivedHistoryRow
        entry={cell.entries[0]}
        onOpenLocalFile={onOpenLocalFile}
        onOpenArtifactUrl={onOpenArtifactUrl}
      />
    );
  }

  if (cell.kind === "compact") {
    const entry = cell.entries[0];
    return (
      <CompactRow
        entry={entry}
        isExpanded={compactHistoryState?.isExpanded ?? false}
        isLoading={compactHistoryState?.isLoading ?? false}
        loadError={compactHistoryState?.error ?? null}
        onToggleExpanded={() => onToggleCompactHistory(entry.id)}
        onOpenLocalFile={onOpenLocalFile}
        onOpenArtifactUrl={onOpenArtifactUrl}
      />
    );
  }

  if (cell.kind === "tool") {
    return (
      <ToolRow
        entries={cell.entries}
        isOpen={isToolOpen}
        onToggleOpen={(open) => onToolOpenChange(cell.id, open)}
        selectedEntryId={selectedToolEntryId}
        onSelectEntry={(entryId) => onToolEntrySelection(cell.id, entryId)}
      />
    );
  }

  if (cell.kind === "artifact") {
    return (
      <ArtifactRow
        entry={cell.entries[0]}
        onOpenArtifactUrl={onOpenArtifactUrl}
      />
    );
  }

  return (
    <MessageRow entries={cell.entries} onOpenLocalFile={onOpenLocalFile} />
  );
}

function MeasuredConversationCell({
  children,
  highlighted,
  onHeightChange,
  searchCurrent,
  searchMatch,
  top,
}: {
  children: ReactNode;
  highlighted: boolean;
  onHeightChange: (height: number) => void;
  searchCurrent: boolean;
  searchMatch: boolean;
  top: number;
}) {
  const elementRef = useRef<HTMLDivElement | null>(null);
  const onHeightChangeRef = useRef(onHeightChange);

  useEffect(() => {
    onHeightChangeRef.current = onHeightChange;
  }, [onHeightChange]);

  useLayoutEffect(() => {
    const element = elementRef.current;
    if (!element) {
      return;
    }

    const notifyHeight = () => {
      onHeightChangeRef.current(element.getBoundingClientRect().height);
    };

    notifyHeight();
    const frameId = window.requestAnimationFrame(notifyHeight);

    const resizeObserver = new ResizeObserver(() => {
      notifyHeight();
    });
    resizeObserver.observe(element);

    return () => {
      window.cancelAnimationFrame(frameId);
      resizeObserver.disconnect();
    };
  }, []);

  return (
    <div
      ref={elementRef}
      className={buildConversationRowClassName({
        highlighted,
        searchCurrent,
        searchMatch,
      })}
      style={{ transform: `translateY(${top}px)` }}
    >
      {children}
    </div>
  );
}

export function buildConversationRowClassName({
  highlighted,
  searchCurrent,
  searchMatch,
}: {
  highlighted: boolean;
  searchCurrent: boolean;
  searchMatch: boolean;
}) {
  const classNames = ["conversation-virtual-row"];
  if (highlighted) {
    classNames.push("highlighted");
  }
  if (searchMatch) {
    classNames.push("search-match");
  }
  if (searchCurrent) {
    classNames.push("search-current");
  }
  return classNames.join(" ");
}
