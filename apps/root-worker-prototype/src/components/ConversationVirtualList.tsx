import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";

import { EventRow, MessageRow, ToolRow } from "./Conversation";
import {
  buildConversationVirtualLayout,
  estimateConversationCellHeight,
  findConversationWindow,
} from "../lib/conversationVirtualization";
import type { ConversationCell } from "../types";

type ConversationVirtualListProps = {
  cells: ConversationCell[];
  containerRef: RefObject<HTMLDivElement | null>;
  onOpenLocalFile: (target: string) => void;
};

type ViewportState = {
  scrollTop: number;
  viewportHeight: number;
};

export function ConversationVirtualList({
  cells,
  containerRef,
  onOpenLocalFile,
}: ConversationVirtualListProps) {
  const measuredHeightsRef = useRef<Map<string, number>>(new Map());
  const layoutRef = useRef<ReturnType<typeof buildConversationVirtualLayout> | null>(
    null,
  );
  const [heightVersion, setHeightVersion] = useState(0);
  const [viewport, setViewport] = useState<ViewportState>({
    scrollTop: 0,
    viewportHeight: 0,
  });
  const [openToolCellIds, setOpenToolCellIds] = useState<Set<string>>(
    () => new Set(),
  );

  useEffect(() => {
    const liveCellIds = new Set(cells.map((cell) => cell.id));
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
    const shouldStickToBottom =
      container != null &&
      container.scrollHeight -
        container.clientHeight -
        container.scrollTop <=
        24;
    if (container && currentTop < container.scrollTop) {
      container.scrollTop += roundedHeight - previousHeight;
    }

    setHeightVersion((version) => version + 1);

    if (container && shouldStickToBottom) {
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
              top={layout.offsets[index] ?? 0}
              onHeightChange={(height) =>
                handleCellHeightChange(cell, index, height)
              }
            >
              {renderConversationCell(
                cell,
                openToolCellIds.has(cell.id),
                onOpenLocalFile,
                handleToolOpenChange,
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
  onOpenLocalFile: (target: string) => void,
  onToolOpenChange: (cellId: string, isOpen: boolean) => void,
) {
  if (cell.kind === "event") {
    return <EventRow entry={cell.entries[0]} />;
  }

  if (cell.kind === "tool") {
    return (
      <ToolRow
        entries={cell.entries}
        isOpen={isToolOpen}
        onToggleOpen={(open) => onToolOpenChange(cell.id, open)}
      />
    );
  }

  return (
    <MessageRow entries={cell.entries} onOpenLocalFile={onOpenLocalFile} />
  );
}

function MeasuredConversationCell({
  children,
  onHeightChange,
  top,
}: {
  children: ReactNode;
  onHeightChange: (height: number) => void;
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
      className="conversation-virtual-row"
      style={{ transform: `translateY(${top}px)` }}
    >
      {children}
    </div>
  );
}
