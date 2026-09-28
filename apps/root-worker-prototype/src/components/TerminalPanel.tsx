import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { FitAddon as XTermFitAddon } from "@xterm/addon-fit";
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
  shouldApplyTerminalViewportFocusRequest,
  type PendingTerminalViewportFocusRequest,
  type TerminalCommandFocusRequest,
} from "../lib/terminalCommandFocus";
import { selectRunningActiveCommandItems } from "../lib/activeCommands";
import {
  writeWorkspaceObjectDragData,
  type WorkspaceObjectDragPayload,
} from "../lib/workspaceObjectDrag";
import type { Thread } from "../types";

type TerminalPanelState = Awaited<
  ReturnType<Window["codexDesktop"]["getTerminalState"]>
>;
type TerminalSize = { rows: number; cols: number };

/*
 * Design brief: a compact operational surface that extends the Browser panel's
 * scrollable tabs and warm-stone chrome. The emulator owns scrolling inside a
 * soft code viewport; teal marks live activity, amber marks focus, and red marks
 * failed/lost sessions. Narrow layouts keep controls terse and ellipsized.
 */

const TERMINAL_THEME = {
  background: "#fbfaf8",
  foreground: "#292524",
  cursor: "#0f766e",
  cursorAccent: "#fbfaf8",
  selectionBackground: "#99f6e466",
  black: "#292524",
  red: "#dc2626",
  green: "#15803d",
  yellow: "#a16207",
  blue: "#2563eb",
  magenta: "#9333ea",
  cyan: "#0f766e",
  white: "#57534e",
  brightBlack: "#78716c",
  brightRed: "#b91c1c",
  brightGreen: "#166534",
  brightYellow: "#854d0e",
  brightBlue: "#1d4ed8",
  brightMagenta: "#7e22ce",
  brightCyan: "#0e7490",
  brightWhite: "#292524",
};

const EMPTY_STATE: TerminalPanelState = {
  activeTabId: null,
  tabs: [],
  detachedCount: 0,
  error: null,
};

export function TerminalPanel({
  variant = "manager",
  thread,
  focusCommandRequest,
  focusPanelRequestToken,
  onOpenTerminalTabInWorkspace,
  activeTerminalTabId,
  focusTerminalTabRequest,
  detachedTerminalTabIds = [],
}: {
  variant?: "manager" | "workspace";
  thread: Thread | null;
  focusCommandRequest?: TerminalCommandFocusRequest | null;
  focusPanelRequestToken?: number;
  onOpenTerminalTabInWorkspace?: (
    tab: Extract<WorkspaceObjectDragPayload, { kind: "terminal" }>,
  ) => void;
  activeTerminalTabId?: string | null;
  focusTerminalTabRequest?: { tabId: string; token: number } | null;
  detachedTerminalTabIds?: string[];
}) {
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<XTermTerminal | null>(null);
  const fitAddonRef = useRef<XTermFitAddon | null>(null);
  const syncTerminalSizeRef = useRef<(() => void) | null>(null);
  const activeTabIdRef = useRef<string | null>(null);
  const activeTabRuntimeRef = useRef<{
    id: string;
    status: TerminalPanelTabState["status"];
    canResize: boolean;
    canWrite: boolean;
  } | null>(null);
  const terminalStatusMarkerRef = useRef<{
    tabId: string;
    generation: string;
    status: "exited" | "lost";
    exitCode: number | null;
  } | null>(null);
  const terminalFocusRequestTokenRef = useRef(0);
  const pendingTerminalFocusRequestRef =
    useRef<PendingTerminalViewportFocusRequest | null>(null);
  const lastAppliedTerminalFocusTokenRef = useRef(0);
  const lastTerminalTabFocusRequestTokenRef = useRef(0);
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
  const [terminalStateLoaded, setTerminalStateLoaded] = useState(false);
  const [displayPreferences, setDisplayPreferences] = useState(
    readTerminalDisplayPreferences,
  );
  const [showDisplaySettings, setShowDisplaySettings] = useState(false);
  const [terminalFocusRequestToken, setTerminalFocusRequestToken] = useState(0);
  const isManagerVariant = variant === "manager";
  const detachedTerminalTabIdSet = useMemo(
    () => new Set(detachedTerminalTabIds),
    [detachedTerminalTabIds],
  );
  const visibleTabs = useMemo(
    () =>
      isManagerVariant && detachedTerminalTabIdSet.size > 0
        ? state.tabs.filter((tab) => !detachedTerminalTabIdSet.has(tab.id))
        : state.tabs,
    [detachedTerminalTabIdSet, isManagerVariant, state.tabs],
  );
  const managerHasDetachedTabs =
    isManagerVariant && detachedTerminalTabIdSet.size > 0;
  const activeTab = useMemo(
    () =>
      visibleTabs.find((tab) => tab.id === state.activeTabId) ??
      visibleTabs[0] ??
      null,
    [state.activeTabId, visibleTabs],
  );
  activeTabIdRef.current = activeTab?.id ?? null;
  activeTabRuntimeRef.current = activeTab
    ? {
        id: activeTab.id,
        status: activeTab.status,
        canResize: activeTab.canResize,
        canWrite: activeTab.canWrite,
      }
    : null;
  terminalFocusRequestTokenRef.current = terminalFocusRequestToken;

  const requestTerminalViewportFocus = useCallback(
    (tabId: string | null = null) => {
      const token = terminalFocusRequestTokenRef.current + 1;
      terminalFocusRequestTokenRef.current = token;
      pendingTerminalFocusRequestRef.current = { token, tabId };
      setTerminalFocusRequestToken(token);
    },
    [],
  );

  const applyPendingTerminalFocus = useCallback(() => {
    const request = pendingTerminalFocusRequestRef.current;
    const terminal = terminalRef.current;
    if (
      !shouldApplyTerminalViewportFocusRequest({
        request,
        lastAppliedToken: lastAppliedTerminalFocusTokenRef.current,
        activeTabId: activeTabIdRef.current,
        terminalAvailable: Boolean(terminal),
      }) ||
      !terminal
    ) {
      return;
    }
    pendingTerminalFocusRequestRef.current = null;
    lastAppliedTerminalFocusTokenRef.current = request.token;
    terminal.focus();
  }, []);

  const publishPreferredTerminalSize = useCallback(
    (next: TerminalSize) => {
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
    },
    [thread?.id],
  );

  useEffect(() => {
    let disposed = false;
    setTerminalStateLoaded(false);
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
        if (
          !disposed &&
          terminalStateRequestSeqRef.current.isCurrent(requestSeq)
        ) {
          setState(nextState);
          setLocalError(null);
          setTerminalStateLoaded(true);
        }
      })
      .catch((error) => {
        if (
          !disposed &&
          terminalStateRequestSeqRef.current.isCurrent(requestSeq)
        ) {
          setLocalError(toTerminalError(error));
        }
      });
    return () => {
      disposed = true;
      unsubscribe();
    };
  }, [thread?.id]);

  useEffect(() => {
    if (
      !isTerminalCommandFocusRequestForThread(focusCommandRequest, thread?.id)
    ) {
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
          requestTerminalViewportFocus(nextState.activeTabId);
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
    requestTerminalViewportFocus,
    thread?.id,
  ]);

  useEffect(() => {
    if (focusPanelRequestToken == null || focusPanelRequestToken <= 0) {
      return;
    }
    requestTerminalViewportFocus();
  }, [focusPanelRequestToken, requestTerminalViewportFocus]);

  useEffect(() => {
    if (
      !activeTerminalTabId ||
      activeTerminalTabId === state.activeTabId ||
      !state.tabs.some((tab) => tab.id === activeTerminalTabId)
    ) {
      return;
    }
    void window.codexDesktop
      .selectTerminalTab(activeTerminalTabId)
      .then((nextState) => {
        setState(nextState);
        requestTerminalViewportFocus(activeTerminalTabId);
        setLocalError(null);
      })
      .catch((error) => setLocalError(toTerminalError(error)));
  }, [
    activeTerminalTabId,
    requestTerminalViewportFocus,
    state.activeTabId,
    state.tabs,
  ]);

  useEffect(() => {
    if (
      !isManagerVariant ||
      !focusTerminalTabRequest ||
      focusTerminalTabRequest.token <=
        lastTerminalTabFocusRequestTokenRef.current
    ) {
      return;
    }
    if (!visibleTabs.some((tab) => tab.id === focusTerminalTabRequest.tabId)) {
      return;
    }
    if (focusTerminalTabRequest.tabId === state.activeTabId) {
      lastTerminalTabFocusRequestTokenRef.current =
        focusTerminalTabRequest.token;
      return;
    }
    lastTerminalTabFocusRequestTokenRef.current = focusTerminalTabRequest.token;
    void window.codexDesktop
      .selectTerminalTab(focusTerminalTabRequest.tabId)
      .then((nextState) => {
        setState(nextState);
        requestTerminalViewportFocus(focusTerminalTabRequest.tabId);
        setLocalError(null);
      })
      .catch((error) => setLocalError(toTerminalError(error)));
  }, [
    focusTerminalTabRequest,
    isManagerVariant,
    requestTerminalViewportFocus,
    state.activeTabId,
    state.tabs,
    visibleTabs,
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
    const mountedTabId = activeTab.id;

    void Promise.all([import("@xterm/xterm"), import("@xterm/addon-fit")])
      .then(([{ Terminal }, { FitAddon }]) => {
        if (disposed) {
          return;
        }
        terminal = new Terminal({
          allowProposedApi: false,
          convertEol: shouldConvertTerminalEol(activeTab),
          cursorBlink: isInteractive(activeTab.status),
          cursorStyle: "bar",
          fontFamily: terminalFontFamilyValue(displayPreferences.fontFamily),
          fontSize: displayPreferences.fontSize,
          lineHeight: displayPreferences.lineHeight,
          scrollback: 10_000,
          theme: TERMINAL_THEME,
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
        const resizeToRecordedReplaySize = () => {
          const size = normalizedTerminalSize(activeTab.size);
          if (!terminal || !size) {
            return false;
          }
          terminal.resize(size.cols, size.rows);
          lastSizeRef.current = size;
          return true;
        };
        sendSize = () => {
          const next = fitTerminal();
          if (!next) {
            return;
          }
          const previous = lastSizeRef.current;
          lastSizeRef.current = next;
          publishPreferredTerminalSize(next);
          const currentTab = activeTabRuntimeRef.current;
          if (
            currentTab?.id === mountedTabId &&
            currentTab.canResize &&
            isInteractive(currentTab.status) &&
            (previous?.rows !== next.rows || previous.cols !== next.cols)
          ) {
            void window.codexDesktop
              .resizeTerminal({ tabId: mountedTabId, size: next })
              .catch((error) => setLocalError(toTerminalError(error)));
          }
        };
        syncTerminalSizeRef.current = sendSize;
        dataSubscription = terminal.onData((data) => {
          const currentTab = activeTabRuntimeRef.current;
          if (
            currentTab?.id !== mountedTabId ||
            !currentTab.canWrite ||
            !isInteractive(currentTab.status)
          ) {
            return;
          }
          void window.codexDesktop
            .writeTerminal({
              tabId: mountedTabId,
              deltaBase64: encodeUtf8(data),
            })
            .catch((error) => setLocalError(toTerminalError(error)));
        });
        binarySubscription = terminal.onBinary((data) => {
          const currentTab = activeTabRuntimeRef.current;
          if (
            currentTab?.id !== mountedTabId ||
            !currentTab.canWrite ||
            !isInteractive(currentTab.status)
          ) {
            return;
          }
          void window.codexDesktop
            .writeTerminal({
              tabId: mountedTabId,
              deltaBase64: encodeBinary(data),
            })
            .catch((error) => setLocalError(toTerminalError(error)));
        });
        const shouldReplayAtRecordedSize =
          Boolean(activeTab.replayBase64) && resizeToRecordedReplaySize();
        if (!shouldReplayAtRecordedSize) {
          sendSize();
        }
        if (activeTab.replayTruncated || activeTab.hasSequenceGap) {
          terminal.writeln(
            "\r\n\u001b[33m[Earlier terminal output is unavailable.]\u001b[0m",
          );
        }
        if (activeTab.replayBase64) {
          terminal.write(decodeBase64(activeTab.replayBase64));
        }
        writeTerminalStatusMarker(terminal, activeTab, terminalStatusMarkerRef);
        if (shouldReplayAtRecordedSize) {
          sendSize();
        }

        resizeObserver = new ResizeObserver(sendSize);
        resizeObserver.observe(viewport);
        queueMicrotask(() => {
          if (disposed) {
            return;
          }
          sendSize?.();
          applyPendingTerminalFocus();
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
      terminalStatusMarkerRef.current = null;
    };
  }, [
    activeTab?.id,
    activeTab?.generation,
    activeTab?.canResize,
    activeTab?.canWrite,
    activeTab?.readOnlyOutput,
    activeTab?.replayBase64,
    activeTab?.replayTruncated,
    activeTab?.hasSequenceGap,
    applyPendingTerminalFocus,
    publishPreferredTerminalSize,
    thread?.id,
  ]);

  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal || !activeTab) {
      terminalStatusMarkerRef.current = null;
      return;
    }

    terminal.options.cursorBlink = isInteractive(activeTab.status);
    writeTerminalStatusMarker(terminal, activeTab, terminalStatusMarkerRef);
  }, [
    activeTab?.exitCode,
    activeTab?.generation,
    activeTab?.id,
    activeTab?.status,
  ]);

  useEffect(() => {
    queueMicrotask(applyPendingTerminalFocus);
  }, [applyPendingTerminalFocus, terminalFocusRequestToken]);

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
    void window.codexDesktop
      .createTerminal({
        cwd: thread?.cwd ?? null,
        size: lastSizeRef.current ?? { rows: 24, cols: 80 },
      })
      .then((nextState) => {
        setState(nextState);
        requestTerminalViewportFocus(nextState.activeTabId);
        setLocalError(null);
      })
      .catch((error) => setLocalError(toTerminalError(error)));
  };

  useEffect(() => {
    if (
      isManagerVariant ||
      !terminalStateLoaded ||
      activeTerminalTabId != null ||
      visibleTabs.length > 0 ||
      localError ||
      state.error
    ) {
      return;
    }
    createTerminal();
  }, [
    activeTerminalTabId,
    isManagerVariant,
    localError,
    state.error,
    terminalStateLoaded,
    visibleTabs.length,
  ]);

  const liveCommands = selectRunningActiveCommandItems(thread);

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
        requestTerminalViewportFocus(nextState.activeTabId);
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

  const terminalTabDragPayload = (
    tab: TerminalPanelState["tabs"][number],
  ): Extract<WorkspaceObjectDragPayload, { kind: "terminal" }> => ({
    kind: "terminal",
    terminalTabId: tab.id,
    sessionId: tab.sessionId,
    threadId: tab.threadId,
    title: tab.title,
    cwd: tab.cwd,
    commandItemId: tab.commandItemId,
    command: tab.title,
    status: tab.status,
  });

  return (
    <div
      className={`preview-panel terminal-panel ${
        isManagerVariant ? "terminal-panel-manager" : "terminal-panel-workspace"
      }`}
    >
      {isManagerVariant ? (
        <>
          <header className="panel-content-header terminal-header">
            <div className="panel-content-copy">
              <h2 title={activeTab?.title}>{activeTab?.title ?? "Terminal"}</h2>
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
              <div
                className="terminal-display-settings"
                aria-label="Terminal display settings"
              >
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
                        fontFamily: event.target
                          .value as typeof displayPreferences.fontFamily,
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
              <span className="terminal-live-commands-label">
                Live Commands
              </span>
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

          <div
            className="browser-tab-strip terminal-tab-strip"
            role="tablist"
            aria-label="Terminal tabs"
          >
            <div className="browser-tabs">
              {visibleTabs.map((tab) => {
                const isActive = tab.id === activeTab?.id;
                return (
                  <div
                    key={tab.id}
                    className={`browser-tab-shell terminal-tab-shell ${isActive ? "active" : ""} ${tab.status}`}
                  >
                    <button
                      type="button"
                      className="browser-tab"
                      draggable={onOpenTerminalTabInWorkspace != null}
                      role="tab"
                      aria-selected={isActive}
                      title={tab.title}
                      onClick={() => {
                        void window.codexDesktop
                          .selectTerminalTab(tab.id)
                          .then((nextState) => {
                            setState(nextState);
                            requestTerminalViewportFocus(tab.id);
                            setLocalError(null);
                          })
                          .catch((error) =>
                            setLocalError(toTerminalError(error)),
                          );
                      }}
                      onDoubleClick={() =>
                        onOpenTerminalTabInWorkspace?.(
                          terminalTabDragPayload(tab),
                        )
                      }
                      onDragStart={(event) =>
                        writeWorkspaceObjectDragData(
                          event.dataTransfer,
                          terminalTabDragPayload(tab),
                        )
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
                        applyState(
                          window.codexDesktop.closeTerminalTab(tab.id),
                        );
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
                onClick={() =>
                  applyState(window.codexDesktop.reattachTerminalTabs())
                }
              >
                Reattach {state.detachedCount}
              </button>
            ) : null}
          </div>
        </>
      ) : null}

      {localError || state.error ? (
        <div className="terminal-status-row" role="status">
          <span className="terminal-status-dot lost" />
          <span>{localError ?? state.error}</span>
        </div>
      ) : null}

      <div className="terminal-viewport-shell">
        <div
          ref={viewportRef}
          className={`terminal-viewport ${activeTab ? "" : "idle"}`}
        />
        {!activeTab ? (
          <div className="terminal-empty">
            <span>$</span>
            <p>
              {managerHasDetachedTabs
                ? "Terminal session is open in workspace."
                : isManagerVariant
                  ? "Open a sandboxed shell or wait for a model PTY to become attachable."
                  : "Terminal session is not available."}
            </p>
            {isManagerVariant ? (
              <button type="button" onClick={createTerminal}>
                New shell
              </button>
            ) : null}
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
  const cellHeight =
    displayPreferences.fontSize * displayPreferences.lineHeight;
  if (cellWidth <= 0 || cellHeight <= 0) {
    return null;
  }
  return {
    rows: Math.max(1, Math.floor(contentHeight / cellHeight)),
    cols: Math.max(1, Math.floor(contentWidth / cellWidth)),
  };
}

function normalizedTerminalSize(size: TerminalSize | null | undefined) {
  if (!size) {
    return null;
  }
  const rows = Math.round(Number(size.rows));
  const cols = Math.round(Number(size.cols));
  if (
    !Number.isSafeInteger(rows) ||
    !Number.isSafeInteger(cols) ||
    rows <= 0 ||
    cols <= 0
  ) {
    return null;
  }
  return { rows, cols };
}

function shouldConvertTerminalEol(tab: TerminalPanelTabState) {
  return tab.readOnlyOutput === true || (!tab.canWrite && !tab.canResize);
}

function writeTerminalStatusMarker(
  terminal: Pick<XTermTerminal, "writeln">,
  tab: TerminalPanelTabState,
  markerRef: {
    current: {
      tabId: string;
      generation: string;
      status: "exited" | "lost";
      exitCode: number | null;
    } | null;
  },
) {
  if (tab.status !== "lost" && tab.status !== "exited") {
    markerRef.current = null;
    return;
  }

  const previous = markerRef.current;
  if (
    previous?.tabId === tab.id &&
    previous.generation === tab.generation &&
    previous.status === tab.status &&
    previous.exitCode === tab.exitCode
  ) {
    return;
  }

  markerRef.current = {
    tabId: tab.id,
    generation: tab.generation,
    status: tab.status,
    exitCode: tab.exitCode,
  };

  if (tab.status === "lost") {
    terminal.writeln(
      "\r\n\u001b[31m[Session disconnected from the runtime.]\u001b[0m",
    );
    return;
  }

  terminal.writeln(
    `\r\n\u001b[90m[Process exited${tab.exitCode == null ? "" : ` with code ${tab.exitCode}`}.]\u001b[0m`,
  );
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
