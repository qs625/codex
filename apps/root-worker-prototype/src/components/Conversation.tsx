import { useEffect, useState } from "react";
import { createPortal } from "react-dom";

import { MarkdownContent } from "../lib/markdown";
import { threadStatusClass } from "../lib/thread";
import type { ConversationEntry } from "../types";
import { CodeIcon, DocumentIcon, RobotIcon, ShareIcon, UserIcon } from "./icons";

const localImageCache = new Map<string, string>();

function ZoomableImage({
  src,
  alt,
  className,
}: {
  src: string;
  alt: string;
  className?: string;
}) {
  const [zoomed, setZoomed] = useState(false);

  useEffect(() => {
    if (!zoomed) {
      return;
    }
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setZoomed(false);
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    document.body.classList.add("is-image-lightbox-open");
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
      document.body.classList.remove("is-image-lightbox-open");
    };
  }, [zoomed]);

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
      {zoomed
        ? createPortal(
            <div
              className="image-lightbox"
              role="dialog"
              aria-modal="true"
              aria-label={alt}
              onClick={() => setZoomed(false)}
            >
              <button
                type="button"
                className="image-lightbox-close"
                aria-label="Close image preview"
                onClick={(event) => {
                  event.stopPropagation();
                  setZoomed(false);
                }}
              >
                ×
              </button>
              <img
                src={src}
                alt={alt}
                className="image-lightbox-image"
                onClick={(event) => event.stopPropagation()}
              />
            </div>,
            document.body,
          )
        : null}
    </>
  );
}

function LocalImage({ path, label }: { path: string; label: string }) {
  const cached = localImageCache.get(path) ?? null;
  const [dataUrl, setDataUrl] = useState<string | null>(cached);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (localImageCache.has(path)) {
      setDataUrl(localImageCache.get(path) ?? null);
      setError(null);
      return;
    }

    let cancelled = false;
    setDataUrl(null);
    setError(null);

    void window.codexDesktop
      .readLocalImage(path)
      .then((result) => {
        if (cancelled) {
          return;
        }
        localImageCache.set(path, result.dataUrl);
        setDataUrl(result.dataUrl);
      })
      .catch((loadError: unknown) => {
        if (cancelled) {
          return;
        }
        setError(loadError instanceof Error ? loadError.message : String(loadError));
      });

    return () => {
      cancelled = true;
    };
  }, [path]);

  if (error) {
    return (
      <span className="attachment-chip" title={`${path}\n${error}`}>
        <DocumentIcon />
        <span>{label}</span>
      </span>
    );
  }

  return (
    <figure className="attachment-image-card">
      {dataUrl ? (
        <ZoomableImage src={dataUrl} alt={label} className="attachment-image" />
      ) : (
        <div
          className="attachment-image attachment-image-loading"
          role="img"
          aria-label={`Loading ${label}`}
        />
      )}
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

export function MessageRow({
  entries,
  onOpenLocalFile,
}: {
  entries: ConversationEntry[];
  onOpenLocalFile?: (target: string) => void;
}) {
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
              <MarkdownContent text={entry.text} onOpenLocalFile={onOpenLocalFile} />
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
}

export function EventRow({ entry }: { entry: ConversationEntry }) {
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
}

export function ToolRow({ entries }: { entries: ConversationEntry[] }) {
  const firstEntry = entries[0];
  const doneCount = entries.filter(
    (entry) => threadStatusClass(entry.toolStatus ?? "todo") === "done",
  ).length;

  return (
    <article className="tool-row">
      <div className="event-icon">
        <CodeIcon />
      </div>
      <details className="tool-card">
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
}
