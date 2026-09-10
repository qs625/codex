import { useEffect, useMemo, useRef, useState } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";

import { PlusIcon, StopIcon, XIcon } from "./icons";
import type { Thread } from "../types";

type TerminalPanelState = Awaited<
  ReturnType<Window["codexDesktop"]["getTerminalState"]>
>;

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

export function TerminalPanel({ thread }: { thread: Thread | null }) {
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const activeTabIdRef = useRef<string | null>(null);
  const lastSizeRef = useRef<{ rows: number; cols: number } | null>(null);
  const [state, setState] = useState<TerminalPanelState>(EMPTY_STATE);
  const [localError, setLocalError] = useState<string | null>(null);
  const activeTab = useMemo(
    () =>
      state.tabs.find((tab) => tab.id === state.activeTabId) ??
      state.tabs[0] ??
      null,
    [state],
  );
  activeTabIdRef.current = activeTab?.id ?? null;

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
    void window.codexDesktop
      .getTerminalState(thread?.id ?? null)
      .then((nextState) => {
        if (!disposed) {
          setState(nextState);
          setLocalError(null);
        }
      })
      .catch((error) => {
        if (!disposed) {
          setLocalError(toTerminalError(error));
        }
      });
    return () => {
      disposed = true;
      unsubscribe();
    };
  }, [thread?.id]);

  useEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport || !activeTab) {
      terminalRef.current?.dispose();
      terminalRef.current = null;
      return undefined;
    }

    const terminal = new Terminal({
      allowProposedApi: false,
      convertEol: false,
      cursorBlink: isInteractive(activeTab.status),
      cursorStyle: "bar",
      fontFamily:
        '"SFMono-Regular", "Cascadia Code", "Liberation Mono", Menlo, monospace',
      fontSize: 12,
      lineHeight: 1.18,
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

    const sendSize = () => {
      try {
        fitAddon.fit();
      } catch {
        return;
      }
      const next = { rows: terminal.rows, cols: terminal.cols };
      const previous = lastSizeRef.current;
      if (
        activeTab.canResize &&
        isInteractive(activeTab.status) &&
        (previous?.rows !== next.rows || previous.cols !== next.cols)
      ) {
        lastSizeRef.current = next;
        void window.codexDesktop
          .resizeTerminal({ tabId: activeTab.id, size: next })
          .catch((error) => setLocalError(toTerminalError(error)));
      }
    };
    const resizeObserver = new ResizeObserver(sendSize);
    resizeObserver.observe(viewport);
    queueMicrotask(() => {
      sendSize();
      terminal.focus();
    });
    const dataSubscription = terminal.onData((data) => {
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
    const binarySubscription = terminal.onBinary((data) => {
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

    return () => {
      dataSubscription.dispose();
      binarySubscription.dispose();
      resizeObserver.disconnect();
      terminal.dispose();
      terminalRef.current = null;
      lastSizeRef.current = null;
    };
  }, [
    activeTab?.id,
    activeTab?.generation,
    activeTab?.replayThroughSequence,
    activeTab?.status,
  ]);

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

  return (
    <div className="preview-panel terminal-panel">
      <header className="panel-content-header terminal-header">
        <div className="panel-content-copy">
          <span className="panel-eyebrow">Terminal</span>
          <h2 title={activeTab?.title}>{activeTab?.title ?? "Terminal"}</h2>
          <p title={activeTab?.cwd}>
            {activeTab
              ? `${activeTab.origin === "model" ? "Model PTY" : "Shell"} · ${activeTab.cwd}`
              : "Create a shell or attach to a live model PTY."}
          </p>
        </div>
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
      </header>

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
        {activeTab ? (
          <div ref={viewportRef} className="terminal-viewport" />
        ) : (
          <div className="terminal-empty">
            <span>$</span>
            <p>Open a sandboxed shell or wait for a model PTY to become attachable.</p>
            <button type="button" onClick={createTerminal}>
              New shell
            </button>
          </div>
        )}
      </div>
    </div>
  );
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
