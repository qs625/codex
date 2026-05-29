import { memo, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import { MarkdownContent } from "../lib/markdown";
import { threadStatusClass } from "../lib/thread";
import type { ConversationEntry } from "../types";
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

function clampScale(value: number) {
  return Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, value));
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

export const MessageRow = memo(function MessageRow({
  entries,
  onOpenLocalFile,
}: MessageRowProps) {
  const firstEntry = entries[0];

  return (
    <article className="message-row">
      <div className={`message-avatar ${firstEntry.role}`}>
        {firstEntry.role === "user" ? <UserIcon /> : <RobotIcon />}
      </div>
      <div className="message-main">
        <div className="message-head">
          <strong>{firstEntry.author}</strong>
          <span>{entries.at(-1)?.timestamp ?? firstEntry.timestamp}</span>
        </div>
        <div className="message-stack">
          {entries.map((entry) => (
            <div key={entry.id} className="message-bubble">
              <MarkdownContent
                text={entry.text}
                onOpenLocalFile={onOpenLocalFile}
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
            </div>
          ))}
        </div>
      </div>
    </article>
  );
}, areMessageRowPropsEqual);

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
}: ToolRowProps) {
  const firstEntry = entries[0];
  const doneCount = entries.filter(
    (entry) => threadStatusClass(entry.toolStatus ?? "todo") === "done",
  ).length;
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
              {entries.length === 1
                ? (firstEntry.toolName ?? firstEntry.author)
                : `${entries.length} tool calls`}
            </strong>
            <span>
              {entries.length === 1
                ? firstEntry.text
                : `${firstEntry.toolName ?? firstEntry.author} and ${entries.length - 1} more`}
            </span>
          </div>
          <div className="tool-card-meta">
            <span
              className={`tool-status-badge ${threadStatusClass(firstEntry.toolStatus ?? "todo")}`}
            >
              {entries.length === 1
                ? (firstEntry.toolStatus ?? "unknown")
                : `${doneCount}/${entries.length} done`}
            </span>
            <time>{entries.at(-1)?.timestamp ?? firstEntry.timestamp}</time>
          </div>
        </summary>
        <div className="tool-card-list">
          {entries.map((entry) => (
            <section key={entry.id} className="tool-card-item">
              <div className="tool-card-item-head">
                <div className="tool-card-copy">
                  <strong>{entry.toolName ?? entry.author}</strong>
                  <span>{entry.text}</span>
                </div>
                <div className="tool-card-meta">
                  <span
                    className={`tool-status-badge ${threadStatusClass(entry.toolStatus ?? "todo")}`}
                  >
                    {entry.toolStatus ?? "unknown"}
                  </span>
                  <time>{entry.timestamp}</time>
                </div>
              </div>
              {entry.toolDetails ? (
                <div className="tool-card-body">
                  <pre>{entry.toolDetails}</pre>
                </div>
              ) : null}
            </section>
          ))}
        </div>
      </details>
    </article>
  );
}, areToolRowPropsEqual);

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
  return previous.entries === next.entries && previous.isOpen === next.isOpen;
}

function getToolIcon(category: NonNullable<ConversationEntry["toolCategory"]>) {
  switch (category) {
    case "command":
      return <TerminalIcon />;
    case "eventDriven":
      return <GearIcon />;
    case "multiAgent":
      return <BranchIcon />;
    case "context":
      return <ShareIcon />;
    case "external":
    default:
      return <CodeIcon />;
  }
}
