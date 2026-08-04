import React, { memo, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import { MarkdownContent } from "../lib/markdown";
import type {
  ApprovalDecision,
  ApprovalRequest,
  ConversationCell,
  ConversationEntry,
} from "../types";
import {
  BranchIcon,
  CodeIcon,
  DocumentIcon,
  GearIcon,
  RobotIcon,
  ShareIcon,
  TerminalIcon,
  UserIcon,
} from "./icons";

type MessageRowProps = {
  entries: ConversationEntry[];
  onOpenLocalFile?: (target: string) => void;
};

type EventRowProps = {
  entry: ConversationEntry;
};

type ToolRowProps = {
  entries: ConversationEntry[];
  isOpen?: boolean;
  onToggleOpen?: (open: boolean) => void;
  selectedEntryId?: string | null;
  onSelectEntry?: (entryId: string | null) => void;
};

type CompactRowProps = {
  entry: ConversationEntry;
  isExpanded?: boolean;
  isLoading?: boolean;
  loadError?: string | null;
  onToggleExpanded?: () => void;
  onOpenLocalFile?: (target: string) => void;
};

type ArchivedHistoryRowProps = {
  entry: ConversationEntry;
  onOpenLocalFile?: (target: string) => void;
};

type ApprovalRequestsPanelProps = {
  requests: ApprovalRequest[];
  onRespond: (request: ApprovalRequest, decision: ApprovalDecision) => void;
};

type LocalImageCacheEntry = {
  objectUrl: string;
  byteSize: number;
  lastAccessedAt: number;
};

const localImageCache = new Map<string, LocalImageCacheEntry>();
const LOCAL_IMAGE_CACHE_MAX_ITEMS = 24;
const LOCAL_IMAGE_CACHE_MAX_BYTES = 64 * 1024 * 1024;

const ZOOM_MIN = 0.25;
const ZOOM_MAX = 8;
const TOOL_DONE_STATUSES = new Set(["completed", "success", "succeeded"]);
const TOOL_DOING_STATUSES = new Set([
  "running",
  "in_progress",
  "inprogress",
  "pending",
  "queued",
  "started",
]);
const TOOL_BLOCKED_STATUSES = new Set([
  "failed",
  "error",
  "errored",
  "aborted",
  "cancelled",
  "canceled",
  "timed_out",
]);

function clampScale(value: number) {
  return Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, value));
}

function toolStatusClass(status?: string) {
  const normalized = status?.trim().toLowerCase();
  if (!normalized) {
    return "todo";
  }
  if (TOOL_DONE_STATUSES.has(normalized)) {
    return "done";
  }
  if (TOOL_DOING_STATUSES.has(normalized)) {
    return "doing";
  }
  if (TOOL_BLOCKED_STATUSES.has(normalized)) {
    return "blocked";
  }
  return "todo";
}

function toolGroupStatusClass(entries: ConversationEntry[]) {
  const classes = entries.map((entry) => toolStatusClass(entry.toolStatus));
  if (classes.every((statusClass) => statusClass === "done")) {
    return "done";
  }
  if (classes.includes("blocked")) {
    return "blocked";
  }
  if (
    classes.includes("doing") ||
    classes.includes("done")
  ) {
    return "doing";
  }
  return "todo";
}

function currentLocalImageCacheBytes() {
  let total = 0;
  for (const entry of localImageCache.values()) {
    total += entry.byteSize;
  }
  return total;
}

function touchLocalImageCache(path: string) {
  const entry = localImageCache.get(path);
  if (!entry) {
    return null;
  }
  entry.lastAccessedAt = Date.now();
  return entry.objectUrl;
}

function pruneLocalImageCache() {
  while (
    localImageCache.size > LOCAL_IMAGE_CACHE_MAX_ITEMS ||
    currentLocalImageCacheBytes() > LOCAL_IMAGE_CACHE_MAX_BYTES
  ) {
    let oldestPath: string | null = null;
    let oldestTimestamp = Number.POSITIVE_INFINITY;
    for (const [path, entry] of localImageCache.entries()) {
      if (entry.lastAccessedAt < oldestTimestamp) {
        oldestPath = path;
        oldestTimestamp = entry.lastAccessedAt;
      }
    }
    if (!oldestPath) {
      break;
    }
    const removed = localImageCache.get(oldestPath);
    if (removed) {
      URL.revokeObjectURL(removed.objectUrl);
    }
    localImageCache.delete(oldestPath);
  }
}

function cacheLocalImage(
  path: string,
  payload: { bytes: ArrayBuffer; mimeType: string; byteSize: number },
) {
  const existing = localImageCache.get(path);
  if (existing) {
    existing.lastAccessedAt = Date.now();
    return existing.objectUrl;
  }
  const objectUrl = URL.createObjectURL(
    new Blob([payload.bytes], { type: payload.mimeType }),
  );
  localImageCache.set(path, {
    objectUrl,
    byteSize: payload.byteSize,
    lastAccessedAt: Date.now(),
  });
  pruneLocalImageCache();
  return objectUrl;
}

async function convertBlobToPng(blob: Blob): Promise<Blob> {
  const objectUrl = URL.createObjectURL(blob);
  try {
    const image = await new Promise<HTMLImageElement>((resolve, reject) => {
      const next = new Image();
      next.onload = () => resolve(next);
      next.onerror = () =>
        reject(new Error("Failed to decode image for clipboard"));
      next.src = objectUrl;
    });
    const canvas = document.createElement("canvas");
    canvas.width = image.naturalWidth || image.width;
    canvas.height = image.naturalHeight || image.height;
    const context = canvas.getContext("2d");
    if (!context) {
      throw new Error("Canvas 2D context unavailable");
    }
    context.drawImage(image, 0, 0);
    return await new Promise<Blob>((resolve, reject) => {
      canvas.toBlob(
        (result) =>
          result ? resolve(result) : reject(new Error("Failed to encode PNG")),
        "image/png",
      );
    });
  } finally {
    URL.revokeObjectURL(objectUrl);
  }
}

function ImageLightbox({
  src,
  alt,
  onClose,
}: {
  src: string;
  alt: string;
  onClose: () => void;
}) {
  const [scale, setScale] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [dragging, setDragging] = useState(false);
  const [copyStatus, setCopyStatus] = useState<
    "idle" | "copying" | "copied" | "error"
  >("idle");
  const copyResetRef = useRef<number | null>(null);
  const dragStateRef = useRef<{
    pointerId: number;
    startX: number;
    startY: number;
    startPan: { x: number; y: number };
  } | null>(null);

  function resetView() {
    setScale(1);
    setPan({ x: 0, y: 0 });
  }

  function adjustScale(delta: number) {
    setScale((current) => {
      const next = clampScale(current + delta);
      if (next === 1) {
        setPan({ x: 0, y: 0 });
      }
      return next;
    });
  }

  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      } else if (event.key === "+" || event.key === "=") {
        event.preventDefault();
        adjustScale(0.25);
      } else if (event.key === "-" || event.key === "_") {
        event.preventDefault();
        adjustScale(-0.25);
      } else if (event.key === "0") {
        event.preventDefault();
        resetView();
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    document.body.classList.add("is-image-lightbox-open");
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
      document.body.classList.remove("is-image-lightbox-open");
    };
  }, [onClose]);

  useEffect(() => {
    return () => {
      if (copyResetRef.current != null) {
        window.clearTimeout(copyResetRef.current);
      }
    };
  }, []);

  function handleWheel(event: React.WheelEvent<HTMLDivElement>) {
    if (event.deltaY === 0) {
      return;
    }
    const factor = event.ctrlKey || event.metaKey ? -0.02 : -0.0035;
    setScale((current) => {
      const next = clampScale(current + event.deltaY * factor);
      if (next === 1) {
        setPan({ x: 0, y: 0 });
      }
      return next;
    });
  }

  function handlePointerDown(event: React.PointerEvent<HTMLImageElement>) {
    if (event.button !== 0 || scale <= 1) {
      return;
    }
    event.preventDefault();
    const target = event.currentTarget;
    target.setPointerCapture?.(event.pointerId);
    dragStateRef.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      startPan: { ...pan },
    };
    setDragging(true);
  }

  function handlePointerMove(event: React.PointerEvent<HTMLImageElement>) {
    const drag = dragStateRef.current;
    if (!drag || drag.pointerId !== event.pointerId) {
      return;
    }
    setPan({
      x: drag.startPan.x + (event.clientX - drag.startX),
      y: drag.startPan.y + (event.clientY - drag.startY),
    });
  }

  function endDrag(event: React.PointerEvent<HTMLImageElement>) {
    if (dragStateRef.current?.pointerId === event.pointerId) {
      dragStateRef.current = null;
      setDragging(false);
    }
  }

  function handleDoubleClick() {
    if (scale === 1) {
      setScale(2);
    } else {
      resetView();
    }
  }

  async function handleCopy() {
    if (copyStatus === "copying") {
      return;
    }
    setCopyStatus("copying");
    try {
      const response = await fetch(src);
      const blob = await response.blob();
      const pngBlob =
        blob.type === "image/png" ? blob : await convertBlobToPng(blob);
      if (typeof ClipboardItem === "undefined" || !navigator.clipboard?.write) {
        throw new Error("Clipboard image API unavailable");
      }
      await navigator.clipboard.write([
        new ClipboardItem({ "image/png": pngBlob }),
      ]);
      setCopyStatus("copied");
    } catch {
      setCopyStatus("error");
    } finally {
      if (copyResetRef.current != null) {
        window.clearTimeout(copyResetRef.current);
      }
      copyResetRef.current = window.setTimeout(() => {
        setCopyStatus("idle");
        copyResetRef.current = null;
      }, 1600);
    }
  }

  const imageCursor = scale > 1 ? (dragging ? "grabbing" : "grab") : "zoom-in";
  const copyLabel =
    copyStatus === "copied"
      ? "Copied"
      : copyStatus === "error"
        ? "Copy failed"
        : copyStatus === "copying"
          ? "Copying…"
          : "Copy";

  return createPortal(
    <div
      className="image-lightbox"
      role="dialog"
      aria-modal="true"
      aria-label={alt}
      onClick={onClose}
      onWheel={handleWheel}
    >
      <div
        className="image-lightbox-toolbar"
        onClick={(event) => event.stopPropagation()}
        onWheel={(event) => event.stopPropagation()}
      >
        <button
          type="button"
          className="image-lightbox-tool"
          aria-label="Zoom out"
          disabled={scale <= ZOOM_MIN}
          onClick={() => adjustScale(-0.25)}
        >
          −
        </button>
        <span className="image-lightbox-scale">{Math.round(scale * 100)}%</span>
        <button
          type="button"
          className="image-lightbox-tool"
          aria-label="Zoom in"
          disabled={scale >= ZOOM_MAX}
          onClick={() => adjustScale(0.25)}
        >
          +
        </button>
        <span className="image-lightbox-separator" />
        <button
          type="button"
          className="image-lightbox-tool"
          aria-label="Reset zoom"
          onClick={resetView}
        >
          Reset
        </button>
        <button
          type="button"
          className={`image-lightbox-tool image-lightbox-copy is-${copyStatus}`}
          aria-label="Copy image to clipboard"
          onClick={() => void handleCopy()}
        >
          {copyLabel}
        </button>
      </div>
      <button
        type="button"
        className="image-lightbox-close"
        aria-label="Close image preview"
        onClick={(event) => {
          event.stopPropagation();
          onClose();
        }}
      >
        ×
      </button>
      <div
        className="image-lightbox-stage"
        onClick={(event) => event.stopPropagation()}
        onDoubleClick={handleDoubleClick}
      >
        <img
          src={src}
          alt={alt}
          className="image-lightbox-image"
          draggable={false}
          style={{
            transform: `translate(${pan.x}px, ${pan.y}px) scale(${scale})`,
            cursor: imageCursor,
            transition: dragging ? "none" : "transform 90ms ease-out",
          }}
          onPointerDown={handlePointerDown}
          onPointerMove={handlePointerMove}
          onPointerUp={endDrag}
          onPointerCancel={endDrag}
        />
      </div>
    </div>,
    document.body,
  );
}

export function ZoomableImage({
  src,
  alt,
  className,
}: {
  src: string;
  alt: string;
  className?: string;
}) {
  const [zoomed, setZoomed] = useState(false);

  return (
    <>
      <img
        src={src}
        alt={alt}
        className={`${className ?? ""} image-zoom-trigger`.trim()}
        onClick={() => setZoomed(true)}
        role="button"
        tabIndex={0}
        onKeyDown={(event) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            setZoomed(true);
          }
        }}
      />
      {zoomed ? (
        <ImageLightbox src={src} alt={alt} onClose={() => setZoomed(false)} />
      ) : null}
    </>
  );
}

export function LocalImagePreview({
  path,
  label,
  className,
}: {
  path: string;
  label: string;
  className: string;
}) {
  const cached = touchLocalImageCache(path);
  const [objectUrl, setObjectUrl] = useState<string | null>(cached);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const nextCached = touchLocalImageCache(path);
    if (nextCached) {
      setObjectUrl(nextCached);
      setError(null);
      return;
    }

    let cancelled = false;
    setObjectUrl(null);
    setError(null);

    void window.codexDesktop
      .readLocalImage(path)
      .then((result) => {
        if (cancelled) {
          return;
        }
        setObjectUrl(cacheLocalImage(path, result));
      })
      .catch((loadError: unknown) => {
        if (cancelled) {
          return;
        }
        setError(
          loadError instanceof Error ? loadError.message : String(loadError),
        );
      });

    return () => {
      cancelled = true;
    };
  }, [path]);

  if (error) {
    return (
      <div
        className={`${className} attachment-image-loading`}
        role="img"
        aria-label={error}
      />
    );
  }

  return objectUrl ? (
    <ZoomableImage src={objectUrl} alt={label} className={className} />
  ) : (
    <div
      className={`${className} attachment-image-loading`}
      role="img"
      aria-label={`Loading ${label}`}
    />
  );
}

function LocalImage({ path, label }: { path: string; label: string }) {
  return (
    <figure className="attachment-image-card">
      <LocalImagePreview
        path={path}
        label={label}
        className="attachment-image"
      />
      <figcaption>{label}</figcaption>
    </figure>
  );
}

export function ThinkingIndicator() {
  return (
    <div className="thinking-row" role="status" aria-live="polite">
      <div className="message-avatar agent thinking-avatar">
        <RobotIcon />
      </div>
      <div className="thinking-bubble">
        <span className="thinking-dot" />
        <span className="thinking-dot" />
        <span className="thinking-dot" />
        <span className="thinking-label">Thinking</span>
      </div>
    </div>
  );
}

export function ApprovalRequestsPanel({
  requests,
  onRespond,
}: ApprovalRequestsPanelProps) {
  if (requests.length === 0) {
    return null;
  }

  return (
    <section className="approval-request-strip" aria-label="Pending approvals">
      {requests.map((request) => (
        <article
          key={String(request.requestId)}
          className={`approval-request-card approval-request-${request.kind}`}
        >
          <div className="approval-request-icon">
            {approvalRequestIcon(request.kind)}
          </div>
          <div className="approval-request-main">
            <div className="approval-request-heading">
              <strong>{request.title}</strong>
              <span>{approvalRequestStatusLabel(request)}</span>
            </div>
            <p>{request.detail}</p>
            {request.reason ? (
              <p className="approval-request-reason">{request.reason}</p>
            ) : null}
            {request.metadata.length > 0 ? (
              <dl className="approval-request-metadata">
                {request.metadata.map((item) => (
                  <div key={`${item.label}:${item.value}`}>
                    <dt>{item.label}</dt>
                    <dd title={item.value}>{item.value}</dd>
                  </div>
                ))}
              </dl>
            ) : null}
            {request.error ? (
              <div className="approval-request-error" role="status">
                {request.error}
              </div>
            ) : null}
          </div>
          <div className="approval-request-actions">
            {request.availableDecisions.includes("accept") ? (
              <button
                type="button"
                className="approval-request-action primary"
                disabled={request.status === "submitting"}
                onClick={() => onRespond(request, "accept")}
              >
                {request.kind === "permissions" ? "Grant" : "Approve"}
              </button>
            ) : null}
            {request.availableDecisions.includes("acceptForSession") ? (
              <button
                type="button"
                className="approval-request-action"
                disabled={request.status === "submitting"}
                onClick={() => onRespond(request, "acceptForSession")}
              >
                Session
              </button>
            ) : null}
            {request.availableDecisions.includes("decline") ? (
              <button
                type="button"
                className="approval-request-action danger"
                disabled={request.status === "submitting"}
                onClick={() => onRespond(request, "decline")}
              >
                Deny
              </button>
            ) : null}
            {request.availableDecisions.includes("cancel") ? (
              <button
                type="button"
                className="approval-request-action"
                disabled={request.status === "submitting"}
                onClick={() => onRespond(request, "cancel")}
              >
                Cancel
              </button>
            ) : null}
          </div>
        </article>
      ))}
    </section>
  );
}

function approvalRequestIcon(kind: ApprovalRequest["kind"]) {
  switch (kind) {
    case "commandExecution":
      return <TerminalIcon />;
    case "fileChange":
      return <DocumentIcon />;
    case "permissions":
      return <GearIcon />;
  }
}

function approvalRequestStatusLabel(request: ApprovalRequest) {
  if (request.status === "submitting") {
    return "Submitting";
  }
  if (request.status === "failed") {
    return "Failed";
  }
  return "Awaiting approval";
}

export const MessageRow = memo(function MessageRow({
  entries,
  onOpenLocalFile,
}: MessageRowProps) {
  const firstEntry = entries[0];
  const shouldUseSingleBubble =
    entries.length > 1 &&
    entries.every(
      (entry) => entry.kind === "message" && entry.role === "agent",
    );

  return (
    <article className={`message-row message-row-${firstEntry.role}`}>
      <div className={`message-avatar ${firstEntry.role}`}>
        {firstEntry.role === "user" ? <UserIcon /> : <RobotIcon />}
      </div>
      <div className="message-main">
        <div className="message-head">
          <strong>{firstEntry.author}</strong>
          <span>{entries.at(-1)?.timestamp ?? firstEntry.timestamp}</span>
        </div>
        <div className="message-stack">
          {shouldUseSingleBubble ? (
            <div className="message-bubble message-bubble-combined">
              {entries.map((entry) => (
                <section key={entry.id} className="message-segment">
                  {renderMessageEntryContent(entry, onOpenLocalFile)}
                </section>
              ))}
            </div>
          ) : (
            entries.map((entry) => (
              <div key={entry.id} className="message-bubble">
                {renderMessageEntryContent(entry, onOpenLocalFile)}
              </div>
            ))
          )}
        </div>
      </div>
    </article>
  );
}, areMessageRowPropsEqual);

function renderMessageEntryContent(
  entry: ConversationEntry,
  onOpenLocalFile?: (path: string) => void,
) {
  return (
    <>
      <MarkdownContent
        text={entry.text}
        {...(onOpenLocalFile ? { onOpenLocalFile } : {})}
      />
      {entry.attachments.length > 0 ? (
        <div className="message-attachments">
          {entry.attachments.map((attachment) => {
            const key = `${entry.id}:${attachment.kind}:${attachment.label}`;
            if (attachment.kind === "image" && attachment.url) {
              return (
                <figure key={key} className="attachment-image-card">
                  <ZoomableImage
                    src={attachment.url}
                    alt={attachment.label}
                    className="attachment-image"
                  />
                  <figcaption>{attachment.label}</figcaption>
                </figure>
              );
            }
            if (attachment.kind === "image" && attachment.path) {
              return (
                <LocalImage
                  key={key}
                  path={attachment.path}
                  label={attachment.label}
                />
              );
            }
            return (
              <span key={key} className="attachment-chip">
                <DocumentIcon />
                <span>{attachment.label}</span>
              </span>
            );
          })}
        </div>
      ) : null}
    </>
  );
}

export const EventRow = memo(function EventRow({ entry }: EventRowProps) {
  return (
    <article className="event-row">
      <div className="event-icon">
        <ShareIcon />
      </div>
      <div className="event-pill">
        <span>{entry.text}</span>
        <time>{entry.timestamp}</time>
      </div>
    </article>
  );
}, areEventRowPropsEqual);

export const ToolRow = memo(function ToolRow({
  entries,
  isOpen,
  onToggleOpen,
  selectedEntryId,
  onSelectEntry,
}: ToolRowProps) {
  const firstEntry = entries[0];
  const hasSingleEntry = entries.length === 1;
  const doneCount = entries.filter(
    (entry) => toolStatusClass(entry.toolStatus) === "done",
  ).length;
  const summaryStatusClass = hasSingleEntry
    ? toolStatusClass(firstEntry.toolStatus)
    : toolGroupStatusClass(entries);
  const toolCategory = firstEntry.toolCategory ?? "external";
  const icon = getToolIcon(toolCategory);

  return (
    <article className={`tool-row tool-row-${toolCategory}`}>
      <div className={`event-icon tool-icon tool-icon-${toolCategory}`}>
        {icon}
      </div>
      <details
        className={`tool-card tool-card-${toolCategory}`}
        open={isOpen}
        onToggle={(event) => {
          onToggleOpen?.(event.currentTarget.open);
        }}
      >
        <summary className="tool-card-summary">
          <div className="tool-card-copy">
            <strong>
              {hasSingleEntry
                ? (firstEntry.toolName ?? firstEntry.author)
                : `${entries.length} tool calls`}
            </strong>
            <span>
              {hasSingleEntry
                ? firstEntry.text
                : `${firstEntry.toolName ?? firstEntry.author} and ${entries.length - 1} more`}
            </span>
          </div>
          <div className="tool-card-meta">
            <span
              className={`tool-status-badge ${summaryStatusClass}`}
            >
              {hasSingleEntry
                ? (firstEntry.toolStatus ?? "unknown")
                : `${doneCount}/${entries.length} done`}
            </span>
            <time>{entries.at(-1)?.timestamp ?? firstEntry.timestamp}</time>
          </div>
        </summary>
        {hasSingleEntry ? (
          isOpen && firstEntry.toolDetails ? (
            <div className="tool-card-body tool-card-item-body">
              {firstEntry.pollEventProgress ? (
                <PollEventProgress progress={firstEntry.pollEventProgress} />
              ) : null}
              <pre>{firstEntry.toolDetails}</pre>
            </div>
          ) : null
        ) : (
          <div className="tool-card-list">
            {entries.map((entry) => {
              const isSelected = selectedEntryId === entry.id;
              return (
                <section key={entry.id} className="tool-card-item">
                  <button
                    type="button"
                    className={`tool-card-item-head ${isSelected ? "selected" : ""}`}
                    onClick={() =>
                      onSelectEntry?.(isSelected ? null : entry.id)
                    }
                  >
                    <div className="tool-card-copy">
                      <strong>{entry.toolName ?? entry.author}</strong>
                      <span>{entry.text}</span>
                    </div>
                    <div className="tool-card-meta">
                      <span
                        className={`tool-status-badge ${toolStatusClass(entry.toolStatus)}`}
                      >
                        {entry.toolStatus ?? "unknown"}
                      </span>
                      <time>{entry.timestamp}</time>
                    </div>
                  </button>
                  {isSelected && entry.toolDetails ? (
                    <div className="tool-card-body tool-card-item-body">
                      {entry.pollEventProgress ? (
                        <PollEventProgress progress={entry.pollEventProgress} />
                      ) : null}
                      <pre>{entry.toolDetails}</pre>
                    </div>
                  ) : null}
                </section>
              );
            })}
          </div>
        )}
      </details>
    </article>
  );
}, areToolRowPropsEqual);

function PollEventProgress({
  progress,
}: {
  progress: NonNullable<ConversationEntry["pollEventProgress"]>;
}) {
  const [nowMs, setNowMs] = useState(() => Date.now());

  useEffect(() => {
    setNowMs(Date.now());
    const timer = window.setInterval(() => {
      setNowMs(Date.now());
    }, 1000);
    return () => window.clearInterval(timer);
  }, []);

  const elapsedMs = Math.max(0, nowMs - progress.startedAtMs);
  const remainingMs = Math.max(0, progress.currentTimeoutMs - elapsedMs);
  const percent =
    progress.currentTimeoutMs > 0
      ? Math.min(
          100,
          Math.max(0, (elapsedMs / progress.currentTimeoutMs) * 100),
        )
      : 0;

  return (
    <div className="poll-event-progress">
      <div className="poll-event-progress-labels">
        <span>Elapsed {formatProgressDuration(elapsedMs)}</span>
        <span>Remaining {formatProgressDuration(remainingMs)}</span>
      </div>
      <div
        className="poll-event-progress-track"
        role="progressbar"
        aria-label="poll_event wait progress"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(percent)}
      >
        <span style={{ width: `${percent}%` }} />
      </div>
    </div>
  );
}

function formatProgressDuration(totalMilliseconds: number) {
  if (!Number.isFinite(totalMilliseconds) || totalMilliseconds <= 0) {
    return "0s";
  }
  if (totalMilliseconds < 1000) {
    return `${Math.round(totalMilliseconds)}ms`;
  }
  const roundedSeconds = Math.round(totalMilliseconds / 1000);
  if (roundedSeconds >= 60) {
    const minutes = Math.floor(roundedSeconds / 60);
    const remainingSeconds = roundedSeconds % 60;
    return remainingSeconds === 0
      ? `${minutes}m`
      : `${minutes}m ${remainingSeconds}s`;
  }
  return `${roundedSeconds}s`;
}

export const CompactRow = memo(function CompactRow({
  entry,
  isExpanded = false,
  isLoading = false,
  loadError = null,
  onToggleExpanded,
  onOpenLocalFile,
}: CompactRowProps) {
  const replacementCount = entry.replacementHistoryCount;
  const replacementLabel =
    replacementCount === null || replacementCount === undefined
      ? "replacement history unavailable"
      : `${replacementCount} replacement item${replacementCount === 1 ? "" : "s"}`;
  const archivedCells = entry.archivedCells ?? [];
  const replacementHistoryCells = entry.replacementHistoryCells ?? [];
  const archivedEntryCount = entry.archivedEntryCount ?? 0;
  return (
    <section className="compact-row" aria-label="Context compacted">
      <div className="event-icon compact-icon">
        <ShareIcon />
      </div>
      <div className="compact-card">
        <button
          type="button"
          className={`compact-summary compact-toggle ${isExpanded ? "expanded" : ""}`}
          aria-expanded={isExpanded}
          onClick={onToggleExpanded}
        >
          <div className="compact-copy">
            <strong>Context compacted</strong>
            <span>{entry.text}</span>
            {entry.replacementHistoryStatus === "missing" ? (
              <em>Replacement history is unavailable for this compact event.</em>
            ) : entry.replacementHistoryStatus === "empty" ? (
              <em>No replacement history was provided after compacting.</em>
            ) : (
              <em>Open this compact round to load the archived conversation and compacted context.</em>
            )}
          </div>
          <div className="compact-meta">
            <span>{replacementLabel}</span>
            <time>{entry.timestamp}</time>
          </div>
        </button>
        {isExpanded ? (
          <div className="compact-body">
            {isLoading ? (
              <div className="compact-body-note">Loading compact history…</div>
            ) : loadError ? (
              <div className="compact-body-note compact-body-error">
                {loadError}
              </div>
            ) : (
              <>
                <div className="compact-group-section">
                  <div className="compact-group-header">
                    <strong>Previous conversation</strong>
                    <span>
                      {archivedEntryCount} archived item
                      {archivedEntryCount === 1 ? "" : "s"}
                    </span>
                  </div>
                  {archivedCells.length > 0 ? (
                    <div className="compact-group-cells">
                      {archivedCells.map((cell) => (
                        <div
                          key={cell.id}
                          className={`archive-cell archive-cell-${cell.kind}`}
                        >
                          {renderNestedConversationCell(cell, onOpenLocalFile)}
                        </div>
                      ))}
                    </div>
                  ) : (
                    <div className="compact-body-note">
                      No archived items were available for this compact round.
                    </div>
                  )}
                </div>
                <div className="compact-group-section">
                  <div className="compact-group-header">
                    <strong>Compacted context</strong>
                    <span>{replacementLabel}</span>
                  </div>
                  {entry.replacementHistoryStatus === "missing" ? (
                    <div className="compact-body-note">
                      Replacement history is unavailable for this compact
                      round.
                    </div>
                  ) : replacementHistoryCells.length > 0 ? (
                    <div className="compact-group-cells">
                      {replacementHistoryCells.map((cell) => (
                        <div
                          key={cell.id}
                          className={`archive-cell archive-cell-${cell.kind}`}
                        >
                          {renderNestedConversationCell(cell, onOpenLocalFile)}
                        </div>
                      ))}
                    </div>
                  ) : (
                    <div className="compact-body-note">
                      No replacement history was provided after compacting.
                    </div>
                  )}
                </div>
              </>
            )}
          </div>
        ) : null}
      </div>
    </section>
  );
}, areCompactRowPropsEqual);

export const ArchivedHistoryRow = memo(function ArchivedHistoryRow({
  entry,
  onOpenLocalFile,
}: ArchivedHistoryRowProps) {
  const archivedCells = entry.archivedCells ?? [];
  const archivedEntryCount = entry.archivedEntryCount ?? 0;

  return (
    <section className="archive-row" aria-label="Previous conversation">
      <div className="event-icon archive-icon">
        <DocumentIcon />
      </div>
      <details className="archive-card">
        <summary className="archive-summary">
          <div className="archive-copy">
            <strong>Previous conversation</strong>
            <span>{entry.text}</span>
          </div>
          <div className="archive-meta">
            <span>
              {archivedEntryCount} archived item
              {archivedEntryCount === 1 ? "" : "s"}
            </span>
            <time>{entry.timestamp}</time>
          </div>
        </summary>
        <div className="archive-body">
          {archivedCells.map((cell) => (
            <div
              key={cell.id}
              className={`archive-cell archive-cell-${cell.kind}`}
            >
              {renderArchivedConversationCell(cell, onOpenLocalFile)}
            </div>
          ))}
        </div>
      </details>
    </section>
  );
}, areArchivedHistoryRowPropsEqual);

function renderArchivedConversationCell(
  cell: ConversationCell,
  onOpenLocalFile?: (target: string) => void,
) {
  return renderNestedConversationCell(cell, onOpenLocalFile);
}

function renderNestedConversationCell(
  cell: ConversationCell,
  onOpenLocalFile?: (target: string) => void,
) {
  if (cell.kind === "message") {
    return (
      <MessageRow entries={cell.entries} onOpenLocalFile={onOpenLocalFile} />
    );
  }
  if (cell.kind === "tool") {
    return (
      <div className="archive-tool-stack">
        {cell.entries.map((entry) => (
          <ToolRow key={entry.id} entries={[entry]} isOpen />
        ))}
      </div>
    );
  }
  if (cell.kind === "event") {
    return <EventRow entry={cell.entries[0]} />;
  }
  if (cell.kind === "compact") {
    return (
      <CompactRow
        entry={cell.entries[0]}
        isExpanded
        onOpenLocalFile={onOpenLocalFile}
      />
    );
  }
  if (cell.kind === "archive") {
    return (
      <ArchivedHistoryRow
        entry={cell.entries[0]}
        onOpenLocalFile={onOpenLocalFile}
      />
    );
  }
  return (
    <div className="compact-nested-item">
      <strong>Archived item</strong>
      <span>{cell.entries[0]?.text ?? "Archived conversation item."}</span>
    </div>
  );
}

function areMessageRowPropsEqual(
  previous: Readonly<MessageRowProps>,
  next: Readonly<MessageRowProps>,
) {
  return previous.entries === next.entries;
}

function areEventRowPropsEqual(
  previous: Readonly<EventRowProps>,
  next: Readonly<EventRowProps>,
) {
  return previous.entry === next.entry;
}

function areToolRowPropsEqual(
  previous: Readonly<ToolRowProps>,
  next: Readonly<ToolRowProps>,
) {
  return (
    previous.entries === next.entries &&
    previous.isOpen === next.isOpen &&
    previous.selectedEntryId === next.selectedEntryId
  );
}

function areCompactRowPropsEqual(
  previous: Readonly<CompactRowProps>,
  next: Readonly<CompactRowProps>,
) {
  return (
    previous.entry === next.entry &&
    previous.isExpanded === next.isExpanded &&
    previous.isLoading === next.isLoading &&
    previous.loadError === next.loadError &&
    previous.onOpenLocalFile === next.onOpenLocalFile
  );
}

function areArchivedHistoryRowPropsEqual(
  previous: Readonly<ArchivedHistoryRowProps>,
  next: Readonly<ArchivedHistoryRowProps>,
) {
  return previous.entry === next.entry;
}

function getToolIcon(category: NonNullable<ConversationEntry["toolCategory"]>) {
  switch (category) {
    case "command":
      return <TerminalIcon />;
    case "eventDrivenSubscription":
    case "eventDrivenEvent":
      return <GearIcon />;
    case "multiAgent":
    case "childCompletion":
    case "subagentNotification":
      return <BranchIcon />;
    case "context":
      return <ShareIcon />;
    case "workflow":
      return <BranchIcon />;
    case "external":
    default:
      return <CodeIcon />;
  }
}
