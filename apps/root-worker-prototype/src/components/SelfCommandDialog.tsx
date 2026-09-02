import { type FormEvent, type KeyboardEvent, useEffect, useRef } from "react";

export type SelfCommandProject = {
  id: "/self";
  path: "/self";
  workspace: string;
  hidden: boolean;
  system: boolean;
};

export function isSelfCommandShortcut(event: {
  ctrlKey: boolean;
  key: string;
  metaKey: boolean;
}) {
  return (event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "p";
}

export function normalizeSelfCommandText(value: string) {
  return value.trim();
}

export function isSelfCommandSubmitDisabled({
  isSubmitting,
  project,
  text,
}: {
  isSubmitting: boolean;
  project: SelfCommandProject | null;
  text: string;
}) {
  return isSubmitting || !project || normalizeSelfCommandText(text).length === 0;
}

export function SelfCommandDialog({
  error,
  isOpen,
  isSubmitting,
  onClose,
  onSubmit,
  onTextChange,
  project,
  text,
  unavailableMessage,
}: {
  error: string | null;
  isOpen: boolean;
  isSubmitting: boolean;
  onClose: () => void;
  onSubmit: () => void;
  onTextChange: (value: string) => void;
  project: SelfCommandProject | null;
  text: string;
  unavailableMessage: string | null;
}) {
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const submitDisabled = isSelfCommandSubmitDisabled({
    isSubmitting,
    project,
    text,
  });

  useEffect(() => {
    if (!isOpen) {
      return;
    }
    window.setTimeout(() => textareaRef.current?.focus(), 0);
  }, [isOpen]);

  if (!isOpen) {
    return null;
  }

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!submitDisabled) {
      onSubmit();
    }
  }

  function handleKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
      return;
    }
    if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
      event.preventDefault();
      if (!submitDisabled) {
        onSubmit();
      }
    }
  }

  return (
    <div className="self-command-layer" onMouseDown={onClose}>
      <form
        aria-labelledby="self-command-title"
        aria-modal="true"
        className="self-command-shell"
        onMouseDown={(event) => event.stopPropagation()}
        onSubmit={handleSubmit}
        role="dialog"
      >
        <header className="self-command-header">
          <div>
            <span className="self-command-eyebrow">/self</span>
            <h2 id="self-command-title">Morpheus self command</h2>
          </div>
          <button
            aria-label="Close self command"
            className="self-command-close"
            onClick={onClose}
            type="button"
          >
            x
          </button>
        </header>
        <div className="self-command-target">
          {project ? project.workspace : unavailableMessage}
        </div>
        <textarea
          aria-label="Self command input"
          className="self-command-input"
          disabled={!project || isSubmitting}
          onChange={(event) => onTextChange(event.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="Ask /self to change Morpheus..."
          ref={textareaRef}
          rows={5}
          value={text}
        />
        {error ? <div className="self-command-error">{error}</div> : null}
        <footer className="self-command-actions">
          <button
            className="self-command-secondary"
            onClick={onClose}
            type="button"
          >
            Cancel
          </button>
          <button
            className="self-command-primary"
            disabled={submitDisabled}
            type="submit"
          >
            {isSubmitting ? "Starting..." : "Start /self"}
          </button>
        </footer>
      </form>
    </div>
  );
}
