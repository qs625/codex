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
  containerRef: RefObject<HTMLDivElement | null>;
  focusedItem: { itemId: string; token: number } | null;
  onOpenLocalFile: (target: string) => void;
  searchCurrentCellId: string | null;
  searchMatchCellIds: Set<string>;
};

type ViewportState = {
  scrollTop: number;
  viewportHeight: number;
};

export function ConversationVirtualList({
  cells,
  containerRef,
  focusedItem,
  onOpenLocalFile,
  searchCurrentCellId,
  searchMatchCellIds,
}: ConversationVirtualListProps) {
  const measuredHeightsRef = useRef<Map<string, number>>(new Map());
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
    if (!focusedItemId) {
      return;
    }

    const index = cells.findIndex((cell) =>
      cell.entries.some((entry) => entry.id === focusedItemId),
    );
    if (index === -1) {
      return;
    }

    const cell = cells[index];
    const container = containerRef.current;
    const top = layoutRef.current?.offsets[index] ?? 0;
    if (container) {
      container.scrollTo({
        top: Math.max(0, top - 24),
        behavior: "smooth",
      });
    }
    setHighlightedCellId(cell.id);
    const timeout = window.setTimeout(() => {
      setHighlightedCellId((current) => (current === cell.id ? null : current));
    }, 2000);
    return () => window.clearTimeout(timeout);
  }, [cells, containerRef, focusedItemId, focusedItemToken]);

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
    const roundedHeight = Math.ceil(measuredHeight);
    const previousHeight =
      measuredHeightsRef.current.get(cell.id) ??
      estimateConversationCellHeight(cell);
    if (roundedHeight === previousHeight) {
      return;
    }

    measuredHeightsRef.current.set(cell.id, roundedHeight);

    const container = containerRef.current;
    const currentTop = layoutRef.current?.offsets[index] ?? 0;
    const scrollPlan = container
      ? planConversationHeightChangeScroll({
          cellTop: currentTop,
          heightDelta: roundedHeight - previousHeight,
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
                openToolCellIds.has(cell.id),
                selectedToolEntryIds.get(cell.id) ?? null,
                onOpenLocalFile,
                handleToolOpenChange,
                handleToolEntrySelection,
              )}
            </MeasuredConversationCell>
          );
        })}
    </div>
  );
}

function renderConversationCell(
  cell: ConversationCell,
  isToolOpen: boolean,
  selectedToolEntryId: string | null,
  onOpenLocalFile: (target: string) => void,
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
      />
    );
  }

  if (cell.kind === "compact") {
    return <CompactRow entry={cell.entries[0]} />;
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

    const resizeObserver = new ResizeObserver(() => {
      notifyHeight();
    });
    resizeObserver.observe(element);

    return () => {
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
