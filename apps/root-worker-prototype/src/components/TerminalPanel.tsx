import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { FitAddon } from "@xterm/addon-fit";
import type { Terminal as XTermTerminal } from "@xterm/xterm";

import { GearIcon, PlusIcon, StopIcon, XIcon } from "./icons";
import {
  TERMINAL_FONT_FAMILIES,
  readTerminalDisplayPreferences,
  resetTerminalDisplayPreferences,
  storeTerminalDisplayPreferences,
  terminalFontFamilyValue,
  updateTerminalDisplayPreferences,
} from "../lib/terminalDisplayPreferences";
import {
  createTerminalStateRequestSequencer,
  isTerminalCommandFocusRequestForThread,
  type TerminalCommandFocusRequest,
} from "../lib/terminalCommandFocus";
import type { Thread } from "../types";

type TerminalPanelState = Awaited<
  ReturnType<Window["codexDesktop"]["getTerminalState"]>
>;
type TerminalSize = { rows: number; cols: number };

/*
 * Design brief: a compact operational surface that extends the Browser panel's
 * scrollable tabs and warm-stone chrome. The emulator owns scrolling inside a
 * deep-charcoal viewport; teal marks live activity, amber marks focus, and red
 * marks failed/lost sessions. Narrow layouts keep controls terse and ellipsized.
 */

const EMPTY_STATE: TerminalPanelState = {
  activeTabId: null,
  tabs: [],
  detachedCount: 0,
  error: null,
};

export function TerminalPanel({
  thread,
  focusCommandRequest,
}: {
  thread: Thread | null;
  focusCommandRequest?: TerminalCommandFocusRequest | null;
}) {
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<XTermTerminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const syncTerminalSizeRef = useRef<(() => void) | null>(null);
  const activeTabIdRef = useRef<string | null>(null);
  const lastSizeRef = useRef<{ rows: number; cols: number } | null>(null);
  const lastPreferredSizeRef = useRef<{
    threadId: string;
    rows: number;
    cols: number;
  } | null>(null);
  const terminalStateRequestSeqRef = useRef(
    createTerminalStateRequestSequencer(),
  );
  const [state, setState] = useState<TerminalPanelState>(EMPTY_STATE);
  const [localError, setLocalError] = useState<string | null>(null);
  const [displayPreferences, setDisplayPreferences] = useState(
    readTerminalDisplayPreferences,
  );
  const [showDisplaySettings, setShowDisplaySettings] = useState(false);
  const activeTab = useMemo(
    () =>
      state.tabs.find((tab) => tab.id === state.activeTabId) ??
      state.tabs[0] ??
      null,
    [state],
  );
  activeTabIdRef.current = activeTab?.id ?? null;

  const publishPreferredTerminalSize = useCallback((next: TerminalSize) => {
    const threadId = thread?.id ?? null;
    const previousPreferred = lastPreferredSizeRef.current;
    if (
      threadId &&
      (previousPreferred?.threadId !== threadId ||
        previousPreferred.rows !== next.rows ||
        previousPreferred.cols !== next.cols)
    ) {
      lastPreferredSizeRef.current = { threadId, ...next };
      void window.codexDesktop
        .updateTerminalPreferredSize({ threadId, size: next })
        .catch(() => {
          if (lastPreferredSizeRef.current?.threadId === threadId) {
            lastPreferredSizeRef.current = previousPreferred;
          }
        });
    }
  }, [thread?.id]);

  useEffect(() => {
    let disposed = false;
    const unsubscribe = window.codexDesktop.subscribeTerminalState((event) => {
      if (disposed) {
        return;
      }
      if (event.type === "snapshot") {
        setState(event.state);
        return;
      }
      setState((current) => ({
        ...current,
        tabs: current.tabs.map((tab) =>
          tab.id === event.tabId ? { ...tab, ...event.tab } : tab,
        ),
      }));
      if (event.tabId === activeTabIdRef.current) {
        terminalRef.current?.write(decodeBase64(event.deltaBase64));
      }
    });
    const requestSeq = terminalStateRequestSeqRef.current.begin();
    void window.codexDesktop
      .getTerminalState(thread?.id ?? null)
      .then((nextState) => {
        if (!disposed && terminalStateRequestSeqRef.current.isCurrent(requestSeq)) {
          setState(nextState);
          setLocalError(null);
        }
      })
      .catch((error) => {
        if (!disposed && terminalStateRequestSeqRef.current.isCurrent(requestSeq)) {
          setLocalError(toTerminalError(error));
        }
      });
    return () => {
      disposed = true;
      unsubscribe();
    };
  }, [thread?.id]);

  useEffect(() => {
    if (!isTerminalCommandFocusRequestForThread(focusCommandRequest, thread?.id)) {
      return;
    }
    const requestSeq = terminalStateRequestSeqRef.current.begin();
    void window.codexDesktop
      .focusTerminalCommand({
        threadId: focusCommandRequest.threadId,
        commandItemId: focusCommandRequest.commandItemId,
        processId: focusCommandRequest.processId,
        command: focusCommandRequest.command,
        cwd: focusCommandRequest.cwd,
        status: focusCommandRequest.status,
      })
      .then(({ state: nextState }) => {
        if (terminalStateRequestSeqRef.current.isCurrent(requestSeq)) {
          setState(nextState);
          setLocalError(null);
        }
      })
      .catch((error) => {
        if (terminalStateRequestSeqRef.current.isCurrent(requestSeq)) {
          setLocalError(toTerminalError(error));
        }
      });
  }, [
    focusCommandRequest?.commandItemId,
    focusCommandRequest?.command,
    focusCommandRequest?.cwd,
    focusCommandRequest?.processId,
    focusCommandRequest?.status,
    focusCommandRequest?.threadId,
    focusCommandRequest?.token,
    thread?.id,
  ]);

  useEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport || !activeTab) {
      terminalRef.current?.dispose();
      terminalRef.current = null;
      return undefined;
    }

    let disposed = false;
    let terminal: XTermTerminal | null = null;
    let sendSize: (() => void) | null = null;
    let resizeObserver: ResizeObserver | null = null;
    let dataSubscription: { dispose: () => void } | null = null;
    let binarySubscription: { dispose: () => void } | null = null;

    void import("@xterm/xterm")
      .then(({ Terminal }) => {
        if (disposed) {
          return;
        }
        terminal = new Terminal({
          allowProposedApi: false,
          convertEol: false,
          cursorBlink: isInteractive(activeTab.status),
          cursorStyle: "bar",
          fontFamily: terminalFontFamilyValue(displayPreferences.fontFamily),
          fontSize: displayPreferences.fontSize,
          lineHeight: displayPreferences.lineHeight,
          scrollback: 10_000,
          theme: {
            background: "#111827",
            foreground: "#e7e5e4",
            cursor: "#f59e0b",
            cursorAccent: "#111827",
            selectionBackground: "#0f766e66",
            black: "#1c1917",
            red: "#f87171",
            green: "#4ade80",
            yellow: "#fbbf24",
            blue: "#60a5fa",
            magenta: "#c084fc",
            cyan: "#2dd4bf",
            white: "#e7e5e4",
            brightBlack: "#78716c",
          },
        });
        const fitAddon = new FitAddon();
        terminal.loadAddon(fitAddon);
        terminal.open(viewport);
        terminalRef.current = terminal;
        fitAddonRef.current = fitAddon;
        const fitTerminal = () => {
          if (!terminal) {
            return null;
          }
          try {
            fitAddon.fit();
          } catch {
            return null;
          }
          const next = { rows: terminal.rows, cols: terminal.cols };
          return next;
        };
        sendSize = () => {
          const next = fitTerminal();
          if (!next) {
            return;
          }
          const previous = lastSizeRef.current;
          lastSizeRef.current = next;
          publishPreferredTerminalSize(next);
          if (
            activeTab.canResize &&
            isInteractive(activeTab.status) &&
            (previous?.rows !== next.rows || previous.cols !== next.cols)
          ) {
            void window.codexDesktop
              .resizeTerminal({ tabId: activeTab.id, size: next })
              .catch((error) => setLocalError(toTerminalError(error)));
          }
        };
        syncTerminalSizeRef.current = sendSize;
        dataSubscription = terminal.onData((data) => {
          if (!activeTab.canWrite || !isInteractive(activeTab.status)) {
            return;
          }
          void window.codexDesktop
            .writeTerminal({
              tabId: activeTab.id,
              deltaBase64: encodeUtf8(data),
            })
            .catch((error) => setLocalError(toTerminalError(error)));
        });
        binarySubscription = terminal.onBinary((data) => {
          if (!activeTab.canWrite || !isInteractive(activeTab.status)) {
            return;
          }
          void window.codexDesktop
            .writeTerminal({
              tabId: activeTab.id,
              deltaBase64: encodeBinary(data),
            })
            .catch((error) => setLocalError(toTerminalError(error)));
        });
        sendSize();
        if (activeTab.replayTruncated || activeTab.hasSequenceGap) {
          terminal.writeln(
            "\r\n\u001b[33m[Earlier terminal output is unavailable.]\u001b[0m",
          );
        }
        if (activeTab.replayBase64) {
          terminal.write(decodeBase64(activeTab.replayBase64));
        }
        if (activeTab.status === "lost") {
          terminal.writeln(
            "\r\n\u001b[31m[Session disconnected from the runtime.]\u001b[0m",
          );
        }
        if (activeTab.status === "exited") {
          terminal.writeln(
            `\r\n\u001b[90m[Process exited${activeTab.exitCode == null ? "" : ` with code ${activeTab.exitCode}`}.]\u001b[0m`,
          );
        }

        resizeObserver = new ResizeObserver(sendSize);
        resizeObserver.observe(viewport);
        queueMicrotask(() => {
          if (disposed) {
            return;
          }
          sendSize?.();
          terminal?.focus();
        });
      })
      .catch((error) => {
        if (!disposed) {
          setLocalError(toTerminalError(error));
        }
      });

    return () => {
      disposed = true;
      dataSubscription?.dispose();
      binarySubscription?.dispose();
      resizeObserver?.disconnect();
      terminal?.dispose();
      if (terminalRef.current === terminal) {
        terminalRef.current = null;
      }
      fitAddonRef.current = null;
      if (syncTerminalSizeRef.current === sendSize) {
        syncTerminalSizeRef.current = null;
      }
      lastSizeRef.current = null;
    };
  }, [
    activeTab?.id,
    activeTab?.generation,
    activeTab?.replayThroughSequence,
    activeTab?.status,
    publishPreferredTerminalSize,
    thread?.id,
  ]);

  useEffect(() => {
    if (activeTab) {
      return undefined;
    }
    const viewport = viewportRef.current;
    if (!viewport) {
      return undefined;
    }

    const sendPreferredSize = () => {
      const next = measureTerminalViewportSize(viewport, displayPreferences);
      if (!next) {
        return;
      }
      lastSizeRef.current = next;
      publishPreferredTerminalSize(next);
    };
    sendPreferredSize();
    const resizeObserver = new ResizeObserver(sendPreferredSize);
    resizeObserver.observe(viewport);
    queueMicrotask(sendPreferredSize);
    return () => {
      resizeObserver.disconnect();
    };
  }, [activeTab, displayPreferences, publishPreferredTerminalSize, thread?.id]);

  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal) {
      return;
    }
    terminal.options.fontFamily = terminalFontFamilyValue(
      displayPreferences.fontFamily,
    );
    terminal.options.fontSize = displayPreferences.fontSize;
    terminal.options.lineHeight = displayPreferences.lineHeight;
    try {
      fitAddonRef.current?.fit();
      syncTerminalSizeRef.current?.();
    } catch {
      // A detached viewport cannot be fitted until it is attached again.
    }
  }, [displayPreferences]);

  const applyState = (promise: Promise<TerminalPanelState>) => {
    void promise
      .then((nextState) => {
        setState(nextState);
        setLocalError(null);
      })
      .catch((error) => setLocalError(toTerminalError(error)));
  };

  const createTerminal = () => {
    applyState(
      window.codexDesktop.createTerminal({
        cwd: thread?.cwd ?? null,
        size: lastSizeRef.current ?? { rows: 24, cols: 80 },
      }),
    );
  };

  const liveCommands = (thread?.activeCommandItems ?? []).filter(
    (item): item is Extract<typeof item, { type: "commandExecution" }> =>
      item.type === "commandExecution" &&
      ["running", "inprogress"].includes(
        item.status.trim().toLowerCase().replace(/[_-]/g, ""),
      ),
  );

  const focusLiveCommand = (command: (typeof liveCommands)[number]) => {
    if (!thread) {
      return;
    }
    void window.codexDesktop
      .focusTerminalCommand({
        threadId: thread.id,
        commandItemId: command.id,
        processId: command.processId,
        command: command.command,
        cwd: command.cwd,
        status: command.status,
      })
      .then(({ state: nextState }) => {
        setState(nextState);
        setLocalError(null);
      })
      .catch((error) => setLocalError(toTerminalError(error)));
  };

  const updateDisplayPreferences = (
    patch: Parameters<typeof updateTerminalDisplayPreferences>[1],
  ) => {
    setDisplayPreferences((current) => {
      const next = updateTerminalDisplayPreferences(current, patch);
      storeTerminalDisplayPreferences(next);
      return next;
    });
  };

  const resetDisplayPreferences = () => {
    setDisplayPreferences(resetTerminalDisplayPreferences());
  };

  return (
    <div className="preview-panel terminal-panel">
      <header className="panel-content-header terminal-header">
        <div className="panel-content-copy">
          <span className="panel-eyebrow">Terminal</span>
          <h2 title={activeTab?.title}>{activeTab?.title ?? "Terminal"}</h2>
          <p title={activeTab?.cwd}>
            {activeTab
              ? `${activeTab.readOnlyOutput ? "Model output" : activeTab.origin === "model" ? "Model PTY" : "Shell"} · ${activeTab.cwd}`
              : "Create a shell or attach to a live model PTY."}
          </p>
        </div>
        <div className="terminal-header-actions">
          <button
            type="button"
            className="panel-inline-action terminal-display-settings-button"
            aria-expanded={showDisplaySettings}
            aria-label="Terminal display settings"
            title="Terminal display settings"
            onClick={() => setShowDisplaySettings((current) => !current)}
          >
            <GearIcon />
          </button>
          <button
            type="button"
            className="panel-inline-action terminal-terminate"
            aria-label="Terminate active terminal"
            title="Terminate process"
            disabled={
              !activeTab?.canTerminate || !isInteractive(activeTab.status)
            }
            onClick={() => {
              if (
                activeTab &&
                window.confirm(`Terminate “${activeTab.title}”?`)
              ) {
                void window.codexDesktop
                  .terminateTerminal(activeTab.id)
                  .catch((error) => setLocalError(toTerminalError(error)));
              }
            }}
          >
            <StopIcon />
          </button>
        </div>
        {showDisplaySettings ? (
          <div className="terminal-display-settings" aria-label="Terminal display settings">
            <div className="terminal-display-settings-heading">
              <span>Display</span>
              <button type="button" onClick={resetDisplayPreferences}>
                Reset
              </button>
            </div>
            <label className="settings-inline-field">
              <span>Font family</span>
              <select
                value={displayPreferences.fontFamily}
                onChange={(event) =>
                  updateDisplayPreferences({
                    fontFamily: event.target.value as typeof displayPreferences.fontFamily,
                  })
                }
              >
                {TERMINAL_FONT_FAMILIES.map((option) => (
                  <option key={option.id} value={option.id}>
                    {option.label}
                  </option>
                ))}
              </select>
            </label>
            <div className="terminal-display-settings-grid">
              <label className="settings-inline-field">
                <span>Font size</span>
                <input
                  type="number"
                  min="10"
                  max="22"
                  step="1"
                  value={displayPreferences.fontSize}
                  onChange={(event) =>
                    updateDisplayPreferences({
                      fontSize: Number(event.target.value),
                    })
                  }
                />
              </label>
              <label className="settings-inline-field">
                <span>Line height</span>
                <input
                  type="number"
                  min="1"
                  max="2"
                  step="0.05"
                  value={displayPreferences.lineHeight}
                  onChange={(event) =>
                    updateDisplayPreferences({
                      lineHeight: Number(event.target.value),
                    })
                  }
                />
              </label>
            </div>
          </div>
        ) : null}
      </header>

      {liveCommands.length > 0 ? (
        <div className="terminal-live-commands" aria-label="Live Commands">
          <span className="terminal-live-commands-label">Live Commands</span>
          <div className="terminal-live-command-list">
            {liveCommands.map((command) => (
              <button
                key={command.id}
                type="button"
                className="terminal-live-command"
                title={command.command}
                onClick={() => focusLiveCommand(command)}
              >
                <span className="terminal-status-dot running" />
                <span>{command.command}</span>
              </button>
            ))}
          </div>
        </div>
      ) : null}

      <div className="browser-tab-strip terminal-tab-strip" role="tablist" aria-label="Terminal tabs">
        <div className="browser-tabs">
          {state.tabs.map((tab) => {
            const isActive = tab.id === activeTab?.id;
            return (
              <div
                key={tab.id}
                className={`browser-tab-shell terminal-tab-shell ${isActive ? "active" : ""} ${tab.status}`}
              >
                <button
                  type="button"
                  className="browser-tab"
                  role="tab"
                  aria-selected={isActive}
                  title={tab.title}
                  onClick={() =>
                    applyState(window.codexDesktop.selectTerminalTab(tab.id))
                  }
                >
                  <span
                    className={`browser-tab-dot terminal-tab-dot ${tab.status} ${tab.backgroundActivity ? "activity" : ""}`}
                  />
                  <span className="browser-tab-title">{tab.title}</span>
                </button>
                <button
                  type="button"
                  className="browser-tab-close"
                  aria-label={`Detach ${tab.title}`}
                  title="Close tab (process keeps running)"
                  onClick={(event) => {
                    event.stopPropagation();
                    applyState(window.codexDesktop.closeTerminalTab(tab.id));
                  }}
                >
                  <XIcon />
                </button>
              </div>
            );
          })}
        </div>
        <button
          type="button"
          className="browser-icon-button browser-new-tab-button"
          aria-label="New shell terminal"
          title="New shell"
          onClick={createTerminal}
        >
          <PlusIcon />
        </button>
        {state.detachedCount > 0 ? (
          <button
            type="button"
            className="browser-icon-button terminal-reattach-button"
            onClick={() => applyState(window.codexDesktop.reattachTerminalTabs())}
          >
            Reattach {state.detachedCount}
          </button>
        ) : null}
      </div>

      <div className="terminal-status-row" role="status">
        <span className={`terminal-status-dot ${activeTab?.status ?? "idle"}`} />
        <span>
          {localError ??
            state.error ??
            (activeTab
              ? `${activeTab.status}${activeTab.canResize ? "" : " · fixed size"}`
              : "No terminal tabs")}
        </span>
      </div>

      <div className="terminal-viewport-shell">
        <div
          ref={viewportRef}
          className={`terminal-viewport ${activeTab ? "" : "idle"}`}
        />
        {!activeTab ? (
          <div className="terminal-empty">
            <span>$</span>
            <p>Open a sandboxed shell or wait for a model PTY to become attachable.</p>
            <button type="button" onClick={createTerminal}>
              New shell
            </button>
          </div>
        ) : null}
      </div>
    </div>
  );
}

function measureTerminalViewportSize(
  viewport: HTMLElement,
  displayPreferences: ReturnType<typeof readTerminalDisplayPreferences>,
): TerminalSize | null {
  const style = window.getComputedStyle(viewport);
  const contentWidth =
    viewport.clientWidth -
    numericCssPixels(style.paddingLeft) -
    numericCssPixels(style.paddingRight);
  const contentHeight =
    viewport.clientHeight -
    numericCssPixels(style.paddingTop) -
    numericCssPixels(style.paddingBottom);
  if (contentWidth <= 0 || contentHeight <= 0) {
    return null;
  }

  const measure = document.createElement("span");
  measure.textContent = "W".repeat(32);
  measure.style.position = "absolute";
  measure.style.visibility = "hidden";
  measure.style.pointerEvents = "none";
  measure.style.whiteSpace = "pre";
  measure.style.fontFamily = terminalFontFamilyValue(
    displayPreferences.fontFamily,
  );
  measure.style.fontSize = `${displayPreferences.fontSize}px`;
  measure.style.lineHeight = `${displayPreferences.lineHeight}`;
  viewport.appendChild(measure);
  const bounds = measure.getBoundingClientRect();
  measure.remove();

  const cellWidth = bounds.width / 32;
  const cellHeight = displayPreferences.fontSize * displayPreferences.lineHeight;
  if (cellWidth <= 0 || cellHeight <= 0) {
    return null;
  }
  return {
    rows: Math.max(1, Math.floor(contentHeight / cellHeight)),
    cols: Math.max(1, Math.floor(contentWidth / cellWidth)),
  };
}

function numericCssPixels(value: string): number {
  const parsed = Number.parseFloat(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

function decodeBase64(value: string): Uint8Array {
  const binary = window.atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

function encodeUtf8(value: string): string {
  const bytes = new TextEncoder().encode(value);
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return window.btoa(binary);
}

function encodeBinary(value: string): string {
  let binary = "";
  for (let index = 0; index < value.length; index += 1) {
    binary += String.fromCharCode(value.charCodeAt(index) & 0xff);
  }
  return window.btoa(binary);
}

function isInteractive(
  status: "starting" | "running" | "exited" | "lost",
): boolean {
  return status === "running";
}

function toTerminalError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
