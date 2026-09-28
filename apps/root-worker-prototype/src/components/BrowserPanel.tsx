import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import {
  ArrowLeftIcon,
  ArrowRightIcon,
  BrowserIcon,
  OpenIcon,
  PlusIcon,
  RefreshIcon,
  StopIcon,
  XIcon,
} from "./icons";
import { normalizeBrowserUrl } from "../lib/browserUrl";
import {
  writeWorkspaceObjectDragData,
  type WorkspaceObjectDragPayload,
} from "../lib/workspaceObjectDrag";

type BrowserViewBounds = {
  x: number;
  y: number;
  width: number;
  height: number;
  sequence?: number;
};

export type BrowserPanelTabState = {
  id: string;
  url: string | null;
  title: string | null;
  loading: boolean;
  canGoBack: boolean;
  canGoForward: boolean;
  error: string | null;
};

export type BrowserWorkspaceTabDescriptor = Extract<
  WorkspaceObjectDragPayload,
  { kind: "browser" }
>;

type BrowserPanelActiveState = Omit<BrowserPanelTabState, "id">;

type BrowserPanelState = BrowserPanelActiveState & {
  activeTabId: string | null;
  tabs: BrowserPanelTabState[];
};

type BrowserPanelApi = Pick<
  Window["codexDesktop"],
  | "browserGoBack"
  | "browserGoForward"
  | "closeBrowserTab"
  | "createBrowserTab"
  | "hideBrowserView"
  | "navigateBrowserView"
  | "openLink"
  | "reloadBrowserView"
  | "selectBrowserTab"
  | "setBrowserViewBounds"
  | "showBrowserView"
  | "stopBrowserView"
  | "subscribeBrowserState"
>;

let browserPanelSurfaceCounter = 0;

function nextBrowserPanelSurfaceId() {
  browserPanelSurfaceCounter += 1;
  return `browser-surface-${browserPanelSurfaceCounter}`;
}

const EMPTY_BROWSER_STATE: BrowserPanelState = {
  url: null,
  title: null,
  loading: false,
  canGoBack: false,
  canGoForward: false,
  error: null,
  activeTabId: null,
  tabs: [],
};

export function resolveBrowserPanelTabSelection({
  tabs,
  activeTabId,
  activeBrowserTabId = null,
  managerSelectedBrowserTabId = null,
  isManagerVariant,
  detachedBrowserTabIds = [],
}: {
  tabs: BrowserPanelTabState[];
  activeTabId: string | null;
  activeBrowserTabId?: string | null;
  managerSelectedBrowserTabId?: string | null;
  isManagerVariant: boolean;
  detachedBrowserTabIds?: string[];
}) {
  const detachedBrowserTabIdSet = new Set(detachedBrowserTabIds);
  const managerVisibleTabs =
    isManagerVariant && detachedBrowserTabIdSet.size > 0
      ? tabs.filter((tab) => !detachedBrowserTabIdSet.has(tab.id))
      : tabs;
  const renderedTabs = isManagerVariant ? managerVisibleTabs : tabs;
  const selectedBrowserTabId = isManagerVariant
    ? (managerSelectedBrowserTabId ?? activeTabId)
    : activeBrowserTabId
      ? activeBrowserTabId
      : activeTabId;
  const hasExplicitWorkspaceSelection =
    !isManagerVariant && activeBrowserTabId != null;
  const activeTab =
    renderedTabs.find((tab) => tab.id === selectedBrowserTabId) ??
    (hasExplicitWorkspaceSelection ? null : (renderedTabs[0] ?? null));
  const managerHasDetachedTabs =
    isManagerVariant && detachedBrowserTabIdSet.size > 0;
  const managerActiveTabDetached =
    isManagerVariant &&
    activeTabId != null &&
    detachedBrowserTabIdSet.has(activeTabId);
  return {
    renderedTabs,
    activeTab,
    managerHasDetachedTabs,
    managerActiveTabDetached,
  };
}

export function BrowserPanel({
  active = true,
  variant = "manager",
  nativeOverlayActive,
  resizing,
  navigationRequest,
  onNavigationRequestHandled,
  onOpenBrowserTabInWorkspace,
  onBrowserTabIdsChange,
  activeBrowserTabId,
  focusBrowserTabRequest,
  detachedBrowserTabIds = [],
}: {
  active?: boolean;
  variant?: "manager" | "workspace";
  nativeOverlayActive: boolean;
  resizing: boolean;
  navigationRequest: { url: string; token: number } | null;
  onNavigationRequestHandled?: (token: number) => void;
  onOpenBrowserTabInWorkspace?: (tab: BrowserWorkspaceTabDescriptor) => void;
  onBrowserTabIdsChange?: (tabIds: string[]) => void;
  activeBrowserTabId?: string | null;
  focusBrowserTabRequest?: { tabId: string; token: number } | null;
  detachedBrowserTabIds?: string[];
}) {
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const boundsSequenceRef = useRef(0);
  const passiveBoundsCorrectionRef = useRef<(() => void) | null>(null);
  const addressInputFocusedRef = useRef(false);
  const lastAddressTabIdRef = useRef<string | null>(null);
  const browserSurfaceRef = useRef({
    activeBrowserTabId,
    detachedBrowserTabIds,
    isManagerVariant: variant === "manager",
    managerSelectedBrowserTabId: null as string | null,
  });
  const browserSurfaceIdRef = useRef(nextBrowserPanelSurfaceId());
  const lastBrowserTabFocusRequestTokenRef = useRef(0);
  const onBrowserTabIdsChangeRef = useRef(onBrowserTabIdsChange);
  const [address, setAddress] = useState("");
  const [managerSelectedBrowserTabId, setManagerSelectedBrowserTabId] =
    useState<string | null>(null);
  const [state, setState] = useState<BrowserPanelState>(EMPTY_BROWSER_STATE);
  const [localError, setLocalError] = useState<string | null>(null);
  const hasBrowserApi = currentBrowserPanelApi() !== null;
  const isManagerVariant = variant === "manager";
  onBrowserTabIdsChangeRef.current = onBrowserTabIdsChange;
  browserSurfaceRef.current = {
    activeBrowserTabId,
    detachedBrowserTabIds,
    isManagerVariant,
    managerSelectedBrowserTabId,
  };
  const tabs = useMemo(
    () =>
      state.tabs.length > 0 || isManagerVariant
        ? state.tabs
        : browserTabsFromActiveState(state),
    [
      isManagerVariant,
      state.canGoBack,
      state.canGoForward,
      state.error,
      state.loading,
      state.tabs,
      state.title,
      state.url,
    ],
  );
  const {
    renderedTabs,
    activeTab,
    managerHasDetachedTabs,
    managerActiveTabDetached,
  } = useMemo(
    () =>
      resolveBrowserPanelTabSelection({
        tabs,
        activeTabId: state.activeTabId,
        activeBrowserTabId,
        managerSelectedBrowserTabId,
        isManagerVariant,
        detachedBrowserTabIds,
      }),
    [
      activeBrowserTabId,
      detachedBrowserTabIds,
      isManagerVariant,
      managerSelectedBrowserTabId,
      state.activeTabId,
      tabs,
    ],
  );
  const managerNativeViewBlocked =
    managerActiveTabDetached || (managerHasDetachedTabs && activeTab == null);
  const workspaceSelectionMissing =
    !isManagerVariant && activeBrowserTabId != null && activeTab == null;
  const displayUrl =
    activeTab?.url ??
    (!isManagerVariant && !workspaceSelectionMissing ? state.url : "") ??
    "";
  const error = activeTab
    ? (localError ?? activeTab.error ?? state.error)
    : localError;
  const panelChromeLabels = useMemo(
    () => resolveBrowserPanelChromeLabels(activeTab),
    [activeTab],
  );
  const syncAddressFromTab = (
    tab: BrowserPanelTabState | null,
    { force = false }: { force?: boolean } = {},
  ) => {
    const tabId = tab?.id ?? null;
    const tabChanged = tabId !== lastAddressTabIdRef.current;
    lastAddressTabIdRef.current = tabId;
    if (!force && addressInputFocusedRef.current && !tabChanged) {
      return;
    }
    setAddress(tab?.url ?? "");
  };
  const handleAddressInputBlur = (
    event: React.FocusEvent<HTMLInputElement>,
  ) => {
    addressInputFocusedRef.current = false;
    const nextFocusedElement = event.relatedTarget;
    if (
      typeof Node !== "undefined" &&
      nextFocusedElement instanceof Node &&
      event.currentTarget.form?.contains(nextFocusedElement)
    ) {
      return;
    }
    syncAddressFromTab(activeTab);
  };

  const applyBrowserState = (nextState: BrowserPanelState) => {
    const normalizedState = normalizeBrowserPanelState(nextState);
    setState(normalizedState);
    onBrowserTabIdsChangeRef.current?.(
      normalizedState.tabs.map((tab) => tab.id),
    );
    const normalizedActiveTab =
      normalizedState.tabs.find(
        (tab) => tab.id === normalizedState.activeTabId,
      ) ??
      normalizedState.tabs[0] ??
      null;
    const surface = browserSurfaceRef.current;
    const surfaceDetachedTabIds = new Set(surface.detachedBrowserTabIds);
    const surfaceActiveTab =
      !surface.isManagerVariant && surface.activeBrowserTabId
        ? (normalizedState.tabs.find(
            (tab) => tab.id === surface.activeBrowserTabId,
          ) ?? null)
        : surface.isManagerVariant
          ? ((surface.managerSelectedBrowserTabId
              ? (normalizedState.tabs.find(
                  (tab) =>
                    tab.id === surface.managerSelectedBrowserTabId &&
                    !surfaceDetachedTabIds.has(tab.id),
                ) ?? null)
              : null) ??
            (normalizedActiveTab &&
            !surfaceDetachedTabIds.has(normalizedActiveTab.id)
              ? normalizedActiveTab
              : (normalizedState.tabs.find(
                  (tab) => !surfaceDetachedTabIds.has(tab.id),
                ) ?? null)))
          : normalizedActiveTab;
    syncAddressFromTab(surfaceActiveTab);
    if (shouldClearBrowserLocalError(normalizedState, normalizedActiveTab)) {
      setLocalError(null);
    }
    passiveBoundsCorrectionRef.current?.();
  };
  const showNativeBrowserView = async (browserApi: BrowserPanelApi) => {
    const viewport = viewportRef.current;
    if (!viewport || !activeTab) {
      return;
    }
    const bounds = {
      ...browserBoundsFromElement(
        viewport,
        nextBrowserBoundsSequence(boundsSequenceRef),
      ),
      surfaceId: browserSurfaceIdRef.current,
      tabId: activeTab.id,
    };
    const nextState = await browserApi.showBrowserView(bounds);
    applyBrowserState(nextState);
  };
  const selectBrowserTabForSurfaceIfNeeded = async (
    browserApi: BrowserPanelApi,
  ) => {
    if (!isManagerVariant) {
      return;
    }
    const targetTabId = activeTab?.id ?? null;
    if (!targetTabId || state.activeTabId === targetTabId) {
      return;
    }
    const nextState = await browserApi.selectBrowserTab(targetTabId);
    applyBrowserState(nextState);
  };

  useEffect(() => {
    const browserApi = currentBrowserPanelApi();
    const unsubscribe = browserApi?.subscribeBrowserState((nextState) => {
      applyBrowserState(nextState);
    });

    return () => {
      unsubscribe?.();
    };
  }, []);

  useEffect(() => {
    if (!isManagerVariant) {
      return;
    }
    syncAddressFromTab(activeTab);
  }, [activeTab?.id, activeTab?.url, isManagerVariant]);

  useEffect(() => {
    if (
      !isManagerVariant ||
      !managerSelectedBrowserTabId ||
      !renderedTabs.some((tab) => tab.id === state.activeTabId)
    ) {
      return;
    }
    setManagerSelectedBrowserTabId(null);
  }, [
    isManagerVariant,
    managerSelectedBrowserTabId,
    renderedTabs,
    state.activeTabId,
  ]);

  useEffect(() => {
    if (!navigationRequest) {
      return;
    }
    onNavigationRequestHandled?.(navigationRequest.token);
    const normalized = normalizeBrowserUrl(navigationRequest.url);
    if (!normalized.ok) {
      setLocalError(normalized.reason);
      return;
    }
    setAddress(normalized.url);
    lastAddressTabIdRef.current = activeTab?.id ?? null;
    setLocalError(null);
    if (isManagerVariant && managerHasDetachedTabs && !activeTab) {
      return;
    }
    const browserApi = currentBrowserPanelApi();
    if (!browserApi) {
      setLocalError("In-app browser is unavailable in this environment.");
      return;
    }
    void (async () => {
      await selectBrowserTabForSurfaceIfNeeded(browserApi);
      await showNativeBrowserView(browserApi);
      setAddress(normalized.url);
      const nextState = await browserApi.navigateBrowserView({
        target: normalized.url,
        surfaceId: browserSurfaceIdRef.current,
        tabId: activeTab?.id ?? null,
      });
      applyBrowserState(nextState);
    })().catch((navigationError) =>
      setLocalError(toBrowserError(navigationError)),
    );
  }, [
    activeTab,
    isManagerVariant,
    managerHasDetachedTabs,
    navigationRequest,
    onNavigationRequestHandled,
  ]);

  useEffect(() => {
    if (
      !active ||
      !isManagerVariant ||
      !focusBrowserTabRequest ||
      focusBrowserTabRequest.token <= lastBrowserTabFocusRequestTokenRef.current
    ) {
      return;
    }
    if (!renderedTabs.some((tab) => tab.id === focusBrowserTabRequest.tabId)) {
      return;
    }
    if (focusBrowserTabRequest.tabId === state.activeTabId) {
      lastBrowserTabFocusRequestTokenRef.current = focusBrowserTabRequest.token;
      return;
    }
    const browserApi = currentBrowserPanelApi();
    if (!browserApi) {
      return;
    }
    lastBrowserTabFocusRequestTokenRef.current = focusBrowserTabRequest.token;
    runCommand(
      (api) => api.selectBrowserTab(focusBrowserTabRequest.tabId),
      "Could not switch tabs.",
    );
  }, [
    active,
    focusBrowserTabRequest,
    isManagerVariant,
    renderedTabs,
    state.activeTabId,
  ]);

  useEffect(() => {
    if (
      !active ||
      !isManagerVariant ||
      !activeTab?.id ||
      activeTab.id === state.activeTabId
    ) {
      return;
    }
    const browserApi = currentBrowserPanelApi();
    if (!browserApi) {
      return;
    }
    runCommand(
      (api) => api.selectBrowserTab(activeTab.id),
      "Could not switch tabs.",
    );
  }, [active, activeTab?.id, isManagerVariant, state.activeTabId]);

  useEffect(() => {
    const viewport = viewportRef.current;
    const browserApi = currentBrowserPanelApi();
    if (!viewport || !browserApi) {
      return undefined;
    }
    const surfaceId = browserSurfaceIdRef.current;

    let boundsUpdateFrame: number | null = null;
    let lastSentBounds: BrowserViewBounds | null = null;
    const shouldHideNativeView =
      !active ||
      !activeTab ||
      nativeOverlayActive ||
      resizing ||
      managerNativeViewBlocked;
    const hideBrowserViewIfOwned = () => {
      void browserApi
        .hideBrowserView({ surfaceId })
        .then((nextState) => applyBrowserState(nextState))
        .catch((error) => setLocalError(toBrowserError(error)));
    };
    const measureBounds = () => ({
      ...browserBoundsFromElement(
        viewport,
        nextBrowserBoundsSequence(boundsSequenceRef),
      ),
      surfaceId,
      tabId: activeTab?.id ?? null,
    });
    const sendBounds = () => {
      const bounds = measureBounds();
      if (browserBoundsMatch(lastSentBounds, bounds)) {
        return;
      }
      lastSentBounds = bounds;
      void browserApi
        .setBrowserViewBounds(bounds)
        .catch((error) => setLocalError(toBrowserError(error)));
    };
    const scheduleBoundsUpdate = () => {
      if (boundsUpdateFrame !== null) {
        return;
      }
      boundsUpdateFrame = window.requestAnimationFrame(() => {
        boundsUpdateFrame = null;
        if (shouldHideNativeView) {
          return;
        }
        sendBounds();
      });
    };
    passiveBoundsCorrectionRef.current = scheduleBoundsUpdate;

    if (shouldHideNativeView) {
      hideBrowserViewIfOwned();
    } else {
      const bounds = measureBounds();
      void browserApi
        .showBrowserView(bounds)
        .then((nextState) => {
          applyBrowserState(nextState);
        })
        .catch((error) => {
          setLocalError(toBrowserError(error));
        });
    }

    scheduleBoundsUpdate();
    const resizeObserver = new ResizeObserver(scheduleBoundsUpdate);
    resizeObserver.observe(viewport);
    window.addEventListener("resize", scheduleBoundsUpdate);

    return () => {
      if (boundsUpdateFrame !== null) {
        window.cancelAnimationFrame(boundsUpdateFrame);
      }
      if (passiveBoundsCorrectionRef.current === scheduleBoundsUpdate) {
        passiveBoundsCorrectionRef.current = null;
      }
      resizeObserver.disconnect();
      window.removeEventListener("resize", scheduleBoundsUpdate);
      void browserApi.hideBrowserView({ surfaceId });
    };
  }, [
    active,
    activeTab?.id,
    managerNativeViewBlocked,
    nativeOverlayActive,
    resizing,
  ]);

  const navigate = () => {
    if (workspaceSelectionMissing) {
      return;
    }
    const normalized = normalizeBrowserUrl(address);
    if (!normalized.ok) {
      setLocalError(normalized.reason);
      return;
    }
    setAddress(normalized.url);
    lastAddressTabIdRef.current = activeTab?.id ?? null;
    setLocalError(null);
    if (isManagerVariant && managerHasDetachedTabs && !activeTab) {
      return;
    }
    const browserApi = currentBrowserPanelApi();
    if (!browserApi) {
      setLocalError("In-app browser is unavailable in this environment.");
      return;
    }
    void (async () => {
      await selectBrowserTabForSurfaceIfNeeded(browserApi);
      await showNativeBrowserView(browserApi);
      const nextState = await browserApi.navigateBrowserView({
        target: normalized.url,
        surfaceId: browserSurfaceIdRef.current,
        tabId: activeTab?.id ?? null,
      });
      applyBrowserState(nextState);
    })().catch((navigationError) =>
      setLocalError(toBrowserError(navigationError)),
    );
  };

  const runCommand = (
    command: (browserApi: BrowserPanelApi) => Promise<BrowserPanelState>,
    fallbackError: string,
  ) => {
    if (workspaceSelectionMissing) {
      return;
    }
    const browserApi = currentBrowserPanelApi();
    if (!browserApi) {
      setLocalError("In-app browser is unavailable in this environment.");
      return;
    }
    setLocalError(null);
    void (async () => {
      await selectBrowserTabForSurfaceIfNeeded(browserApi);
      return command(browserApi);
    })()
      .then(applyBrowserState)
      .catch((commandError) =>
        setLocalError(toBrowserError(commandError) || fallbackError),
      );
  };
  const activeBrowserCommandTarget = () => ({
    surfaceId: browserSurfaceIdRef.current,
    tabId: activeTab?.id ?? null,
  });

  const createTab = () => {
    const browserApi = currentBrowserPanelApi();
    if (!browserApi) {
      setLocalError("In-app browser is unavailable in this environment.");
      return;
    }
    setLocalError(null);
    runCommand((api) => api.createBrowserTab(), "Could not create a tab.");
  };

  const selectTab = (tabId: string) => {
    if (tabId === state.activeTabId) {
      return;
    }
    runCommand(
      (browserApi) => browserApi.selectBrowserTab(tabId),
      "Could not switch tabs.",
    );
  };

  const closeTab = (tabId: string) => {
    runCommand(
      (browserApi) => browserApi.closeBrowserTab(tabId),
      "Could not close the tab.",
    );
  };

  const browserTabDragPayload = useCallback(
    (tab: BrowserPanelTabState): BrowserWorkspaceTabDescriptor => ({
      kind: "browser",
      browserTabId: tab.id,
      title: browserTabLabel(tab),
      url: tab.url,
    }),
    [],
  );

  return (
    <div
      className={`preview-panel browser-panel ${
        isManagerVariant ? "browser-panel-manager" : "browser-panel-workspace"
      }`}
    >
      {isManagerVariant ? (
        <>
          <header className="panel-content-header browser-header">
            <div className="panel-content-copy">
              <h2>{panelChromeLabels.headerTitle}</h2>
            </div>
            <button
              type="button"
              className="panel-inline-action browser-open-external"
              aria-label="Open browser page externally"
              title="Open externally"
              disabled={!displayUrl || !hasBrowserApi}
              onClick={() => {
                if (!displayUrl) {
                  return;
                }
                runCommand(
                  (browserApi) =>
                    browserApi.openLink(displayUrl).then(() => state),
                  "Could not open the page externally.",
                );
              }}
            >
              <OpenIcon />
            </button>
          </header>

          <div
            className="browser-tab-strip"
            role="tablist"
            aria-label="Browser tabs"
          >
            <div className="browser-tabs">
              {renderedTabs.map((tab) => {
                const isActive = tab.id === activeTab?.id;
                return (
                  <div
                    key={tab.id}
                    className={`browser-tab-shell ${isActive ? "active" : ""}`}
                  >
                    <button
                      type="button"
                      className="browser-tab"
                      draggable={onOpenBrowserTabInWorkspace != null}
                      role="tab"
                      aria-selected={isActive}
                      title={browserTabLabel(tab)}
                      onClick={() => selectTab(tab.id)}
                      onDoubleClick={() =>
                        onOpenBrowserTabInWorkspace?.(
                          browserTabDragPayload(tab),
                        )
                      }
                      onDragStart={(event) =>
                        writeWorkspaceObjectDragData(
                          event.dataTransfer,
                          browserTabDragPayload(tab),
                        )
                      }
                    >
                      <span
                        className={`browser-tab-dot ${tab.loading ? "loading" : ""}`}
                      />
                      <span className="browser-tab-title">
                        {browserTabLabel(tab)}
                      </span>
                    </button>
                    <button
                      type="button"
                      className="browser-tab-close"
                      aria-label={`Close ${browserTabLabel(tab)}`}
                      title="Close tab"
                      onClick={(event) => {
                        event.preventDefault();
                        event.stopPropagation();
                        closeTab(tab.id);
                      }}
                      onKeyDown={(event) => {
                        if (event.key === "Enter" || event.key === " ") {
                          event.preventDefault();
                          event.stopPropagation();
                          closeTab(tab.id);
                        }
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
              aria-label="New browser tab"
              title="New tab"
              disabled={!hasBrowserApi}
              onClick={createTab}
            >
              <PlusIcon />
            </button>
          </div>

          {activeTab ? (
            <>
              <form
                className="browser-toolbar"
                onSubmit={(event) => {
                  event.preventDefault();
                  navigate();
                }}
              >
                <button
                  type="button"
                  className="browser-icon-button"
                  aria-label="Go back"
                  title="Back"
                  disabled={!activeTab.canGoBack || !hasBrowserApi}
                  onClick={() =>
                    runCommand(
                      (browserApi) =>
                        browserApi.browserGoBack(activeBrowserCommandTarget()),
                      "Could not go back.",
                    )
                  }
                >
                  <ArrowLeftIcon />
                </button>
                <button
                  type="button"
                  className="browser-icon-button"
                  aria-label="Go forward"
                  title="Forward"
                  disabled={!activeTab.canGoForward || !hasBrowserApi}
                  onClick={() =>
                    runCommand(
                      (browserApi) =>
                        browserApi.browserGoForward(
                          activeBrowserCommandTarget(),
                        ),
                      "Could not go forward.",
                    )
                  }
                >
                  <ArrowRightIcon />
                </button>
                <button
                  type="button"
                  className="browser-icon-button"
                  aria-label={activeTab.loading ? "Stop loading" : "Reload"}
                  title={activeTab.loading ? "Stop" : "Reload"}
                  disabled={!hasBrowserApi}
                  onClick={() =>
                    runCommand(
                      (browserApi) =>
                        activeTab.loading
                          ? browserApi.stopBrowserView(
                              activeBrowserCommandTarget(),
                            )
                          : browserApi.reloadBrowserView(
                              activeBrowserCommandTarget(),
                            ),
                      "Could not update the page.",
                    )
                  }
                >
                  {activeTab.loading ? <StopIcon /> : <RefreshIcon />}
                </button>
                <input
                  aria-label="Browser URL"
                  value={address}
                  placeholder="https://example.com or localhost:5173"
                  onFocus={() => {
                    addressInputFocusedRef.current = true;
                  }}
                  onBlur={handleAddressInputBlur}
                  onChange={(event) => setAddress(event.target.value)}
                />
                <button
                  type="submit"
                  className="browser-go-button"
                  disabled={!hasBrowserApi}
                >
                  Go
                </button>
              </form>

              <div className="browser-status-row" role="status">
                <span
                  className={`browser-status-dot ${activeTab.loading ? "loading" : "idle"}`}
                />
                <span title={error ?? displayUrl}>
                  {error ?? (displayUrl || "Ready")}
                </span>
              </div>
            </>
          ) : null}
        </>
      ) : null}
      {!isManagerVariant && !workspaceSelectionMissing ? (
        <form
          className="browser-toolbar browser-toolbar-workspace"
          onSubmit={(event) => {
            event.preventDefault();
            navigate();
          }}
        >
          <button
            type="button"
            className="browser-icon-button"
            aria-label="Go back"
            title="Back"
            disabled={!activeTab?.canGoBack || !hasBrowserApi}
            onClick={() =>
              runCommand(
                (browserApi) =>
                  browserApi.browserGoBack(activeBrowserCommandTarget()),
                "Could not go back.",
              )
            }
          >
            <ArrowLeftIcon />
          </button>
          <button
            type="button"
            className="browser-icon-button"
            aria-label="Go forward"
            title="Forward"
            disabled={!activeTab?.canGoForward || !hasBrowserApi}
            onClick={() =>
              runCommand(
                (browserApi) =>
                  browserApi.browserGoForward(activeBrowserCommandTarget()),
                "Could not go forward.",
              )
            }
          >
            <ArrowRightIcon />
          </button>
          <button
            type="button"
            className="browser-icon-button"
            aria-label={activeTab?.loading ? "Stop loading" : "Reload"}
            title={activeTab?.loading ? "Stop" : "Reload"}
            disabled={!hasBrowserApi}
            onClick={() =>
              runCommand(
                (browserApi) =>
                  activeTab?.loading
                    ? browserApi.stopBrowserView(activeBrowserCommandTarget())
                    : browserApi.reloadBrowserView(
                        activeBrowserCommandTarget(),
                      ),
                "Could not update the page.",
              )
            }
          >
            {activeTab?.loading ? <StopIcon /> : <RefreshIcon />}
          </button>
          <input
            aria-label="Workspace browser URL"
            value={address}
            placeholder={displayUrl || "https://example.com or localhost:5173"}
            onFocus={() => {
              addressInputFocusedRef.current = true;
            }}
            onBlur={handleAddressInputBlur}
            onChange={(event) => setAddress(event.target.value)}
          />
          <button
            type="submit"
            className="browser-go-button"
            disabled={!hasBrowserApi}
          >
            Go
          </button>
        </form>
      ) : null}

      <div ref={viewportRef} className="browser-native-viewport">
        {managerNativeViewBlocked ? (
          <div className="browser-empty">
            <BrowserIcon />
            <span>Browser content is open in workspace.</span>
          </div>
        ) : workspaceSelectionMissing ? (
          <div className="browser-empty">
            <BrowserIcon />
            <span>Browser tab closed.</span>
          </div>
        ) : !displayUrl && isManagerVariant ? (
          <div className="browser-empty">
            <BrowserIcon />
            <span>Open a page in the right panel.</span>
          </div>
        ) : null}
      </div>
    </div>
  );
}

export function normalizeBrowserPanelState(
  state: BrowserPanelState | BrowserPanelActiveState,
): BrowserPanelState {
  const candidateState = state as Partial<BrowserPanelState> &
    BrowserPanelActiveState;
  const tabs =
    Array.isArray(candidateState.tabs) && candidateState.tabs.length > 0
      ? candidateState.tabs
      : browserTabsFromActiveState(candidateState);
  const activeTabId =
    typeof candidateState.activeTabId === "string"
      ? candidateState.activeTabId
      : null;
  const activeTab =
    tabs.find((tab) => tab.id === activeTabId) ?? tabs[0] ?? null;
  return {
    url: activeTab?.url ?? candidateState.url,
    title: activeTab?.title ?? candidateState.title,
    loading: activeTab?.loading ?? candidateState.loading,
    canGoBack: activeTab?.canGoBack ?? candidateState.canGoBack,
    canGoForward: activeTab?.canGoForward ?? candidateState.canGoForward,
    error: activeTab?.error ?? candidateState.error,
    activeTabId: activeTab?.id ?? null,
    tabs,
  };
}

export function shouldClearBrowserLocalError(
  state: BrowserPanelState,
  activeTab: BrowserPanelTabState | null,
) {
  return !state.error && !activeTab?.error;
}

function browserTabsFromActiveState(state: BrowserPanelActiveState) {
  return [
    {
      id: "browser-tab-active",
      url: state.url,
      title: state.title,
      loading: state.loading,
      canGoBack: state.canGoBack,
      canGoForward: state.canGoForward,
      error: state.error,
    },
  ];
}

export function browserTabLabel(tab: BrowserPanelTabState) {
  if (tab.title?.trim()) {
    return tab.title.trim();
  }
  if (!tab.url) {
    return "New tab";
  }
  try {
    const parsed = new URL(tab.url);
    return parsed.host || tab.url;
  } catch {
    return tab.url;
  }
}

export function resolveBrowserPanelChromeLabels(
  activeTab: BrowserPanelTabState | null,
) {
  return {
    headerTitle: "Browser",
    activeTabTitle: activeTab ? browserTabLabel(activeTab) : "New tab",
  };
}

export function browserBoundsFromElement(
  element: HTMLElement,
  sequence?: number,
): BrowserViewBounds {
  const rect = element.getBoundingClientRect();
  return {
    x: Math.max(0, Math.round(rect.left)),
    y: Math.max(0, Math.round(rect.top)),
    width: Math.max(0, Math.round(rect.width)),
    height: Math.max(0, Math.round(rect.height)),
    ...(sequence ? { sequence } : {}),
  };
}

export function nextBrowserBoundsSequence(ref: { current: number }): number {
  const sequence = Math.max(ref.current + 1, Date.now());
  ref.current = sequence;
  return sequence;
}

export function browserBoundsMatch(
  previous: BrowserViewBounds | null,
  next: BrowserViewBounds,
): boolean {
  return (
    previous !== null &&
    previous.x === next.x &&
    previous.y === next.y &&
    previous.width === next.width &&
    previous.height === next.height
  );
}

export function currentBrowserPanelApi(): BrowserPanelApi | null {
  if (typeof window === "undefined") {
    return null;
  }
  const browserApi = window.codexDesktop;
  if (
    !browserApi?.browserGoBack ||
    !browserApi.browserGoForward ||
    !browserApi.closeBrowserTab ||
    !browserApi.createBrowserTab ||
    !browserApi.hideBrowserView ||
    !browserApi.navigateBrowserView ||
    !browserApi.openLink ||
    !browserApi.reloadBrowserView ||
    !browserApi.selectBrowserTab ||
    !browserApi.setBrowserViewBounds ||
    !browserApi.showBrowserView ||
    !browserApi.stopBrowserView ||
    !browserApi.subscribeBrowserState
  ) {
    return null;
  }
  return browserApi;
}

function toBrowserError(error: unknown) {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  return "Browser action failed.";
}
