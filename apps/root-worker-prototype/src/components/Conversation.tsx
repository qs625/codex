import { MarkdownContent } from "../lib/markdown";
import { threadStatusClass } from "../lib/thread";
import type { ConversationEntry } from "../types";
import { CodeIcon, DocumentIcon, RobotIcon, ShareIcon, UserIcon } from "./icons";

export function MessageRow({ entries }: { entries: ConversationEntry[] }) {
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
              <MarkdownContent text={entry.text} />
              {entry.attachments.length > 0 ? (
                <div className="message-attachments">
                  {entry.attachments.map((attachment) =>
                    attachment.kind === "image" && attachment.url ? (
                      <figure
                        key={`${entry.id}:${attachment.kind}:${attachment.label}`}
                        className="attachment-image-card"
                      >
                        <img
                          src={attachment.url}
                          alt={attachment.label}
                          className="attachment-image"
                        />
                        <figcaption>{attachment.label}</figcaption>
                      </figure>
                    ) : (
                      <span
                        key={`${entry.id}:${attachment.kind}:${attachment.label}`}
                        className="attachment-chip"
                      >
                        <DocumentIcon />
                        <span>{attachment.label}</span>
                      </span>
                    ),
                  )}
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
