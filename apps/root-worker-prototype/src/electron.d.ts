export {};

type BrowserPanelTabState = {
  id: string;
  url: string | null;
  title: string | null;
  loading: boolean;
  canGoBack: boolean;
  canGoForward: boolean;
  error: string | null;
};

type BrowserPanelState = Omit<BrowserPanelTabState, "id"> & {
  activeTabId: string | null;
  tabs: BrowserPanelTabState[];
};

type TerminalPanelTabState = {
  id: string;
  sessionId: string;
  generation: string;
  origin: "user" | "model";
  threadId: string | null;
  commandItemId: string | null;
  processId: string;
  title: string;
  cwd: string;
  status: "starting" | "running" | "exited" | "lost";
  replayBase64: string;
  replayTruncated: boolean;
  replayThroughSequence: number;
  size: { rows: number; cols: number } | null;
  lastSequence: number | null;
  hasSequenceGap: boolean;
  backgroundActivity: boolean;
  canResize: boolean;
  canWrite: boolean;
  canTerminate: boolean;
  readOnlyOutput?: boolean;
  exitCode: number | null;
  error?: string | null;
};

type TerminalPanelState = {
  activeTabId: string | null;
  tabs: TerminalPanelTabState[];
  detachedCount: number;
  error: string | null;
};

type TerminalPanelEvent =
  | { type: "snapshot"; state: TerminalPanelState }
  | {
      type: "delta";
      tabId: string;
      deltaBase64: string;
      tab: Omit<TerminalPanelTabState, "replayBase64">;
    };

type ComputerUsePoint = {
  x: number;
  y: number;
};

type ComputerUseAction =
  | ({ type: "move" | "click" } & ComputerUsePoint)
  | { type: "type"; text: string }
  | { type: "key"; key: string; modifiers?: string[] }
  | { type: "drag"; from: ComputerUsePoint; to: ComputerUsePoint };

type ComputerUsePolicy = {
  kind:
    | "read-only"
    | "low-risk"
    | "needs-confirmation"
    | "needs-permission"
    | "needs-observation"
    | "target-mismatch"
    | "disabled";
  allowed: boolean;
  reason: string | null;
};

type ComputerUseState = {
  id: string;
  status: "idle" | "observing" | "active" | "acting" | "error" | "stopped";
  createdAtMs: number;
  updatedAtMs: number;
  target: {
    app: string | null;
    window: unknown | null;
  };
  sequence: number;
  observation: {
    sequence: number;
    reason: string;
    observedAtMs: number;
    cursor: ComputerUsePoint | null;
    systemCursor: ComputerUsePoint | null;
    activeApp: unknown | null;
    frontmostApp: unknown | null;
    targetApp: unknown | null;
    targetVisibility: "frontmost" | "background" | "unknown";
    limitations: Array<{ code: string; message: string }>;
    accessibilityTrusted: boolean;
    screenshot: {
      path: string;
      mimeType: string;
      byteSize: number;
      dataUrl: string;
    } | null;
    error: string | null;
  } | null;
  frontmostApp: unknown | null;
  targetApp: unknown | null;
  targetVisibility: "frontmost" | "background" | "unknown";
  systemCursor: ComputerUsePoint | null;
  agentCursor: ComputerUsePoint | null;
  cursor: ComputerUsePoint | null;
  pointerPath: Array<ComputerUsePoint & { atMs: number; source: string }>;
  limitations: Array<{ code: string; message: string }>;
  overlay: {
    mode: "target-bound";
    visible: boolean;
    reason: string | null;
  };
  trace: Array<{
    id: string;
    sequence: number;
    action: ComputerUseAction;
    policy: ComputerUsePolicy;
    status: "running" | "completed" | "failed" | "blocked";
    error: string | null;
    startedAtMs: number;
    completedAtMs: number | null;
    observationSequence: number | null;
    agentCursorPath?: Array<ComputerUsePoint & { atMs: number }>;
    evidence?: {
      systemCursorRestored?: boolean;
      systemCursorBefore?: ComputerUsePoint | null;
      systemCursorAfter?: ComputerUsePoint | null;
    };
  }>;
  pendingAction: ComputerUseAction | null;
  policy: ComputerUsePolicy | null;
  error: { phase: string; message: string; atMs: number } | null;
};

declare global {
  interface Window {
    codexDesktop: {
      health: () => Promise<{
        ok: boolean;
        appServer: {
          connected: boolean;
          pid: number | null;
          mobileConnection?: AndroidConnectionInfo;
        };
        workspace: string;
      }>;
      showSystemNotification: (payload: {
        title: string;
        body?: string | null;
      }) => Promise<{ ok: boolean; reason?: string }>;
      relaunchApp: (payload?: {
        reason?: string | null;
      }) => Promise<{
        ok: boolean;
        relaunching: boolean;
        alreadyRequested?: boolean;
        reason?: string | null;
      }>;
      bootstrap: () => Promise<{
        workspace: string;
        threads: unknown[];
        autoResume?: {
          resumedThreadIds: string[];
          skippedThreadIds: string[];
          failedThreadIds: string[];
          focusThreadId: string | null;
          errors: Array<{ threadId: string; message: string }>;
        };
        expectedRestart?: {
          recoveredThreadIds: string[];
          failedThreadIds: string[];
          expectedRequestIds: string[];
          expectedThreadIds: string[];
          recoveryOccurrenceId: string | null;
          focusThreadId: string | null;
        };
        appServer: {
          connected: boolean;
          pid: number | null;
          mobileConnection?: AndroidConnectionInfo;
        };
      }>;
      listThreads: (cwd?: string) => Promise<{ data: unknown[] }>;
      listModels: () => Promise<unknown>;
      readConfig: (payload?: {
        includeLayers?: boolean;
        cwd?: string | null;
      }) => Promise<unknown>;
      writeConfigValue: (payload: {
        keyPath: string;
        value: unknown;
        mergeStrategy: "replace" | "upsert";
        filePath?: string | null;
        expectedVersion?: string | null;
      }) => Promise<unknown>;
      batchWriteConfig: (payload: {
        edits: Array<{
          keyPath: string;
          value: unknown;
          mergeStrategy: "replace" | "upsert";
        }>;
        filePath?: string | null;
        expectedVersion?: string | null;
        reloadUserConfig?: boolean;
      }) => Promise<unknown>;
      readAccount: (payload?: {
        refreshToken?: boolean;
      }) => Promise<unknown>;
      getAndroidConnectionInfo: () => Promise<AndroidConnectionInfo>;
      startAccountLogin: (payload:
        | { type: "apiKey"; apiKey: string }
        | { type: "chatgpt"; codexStreamlinedLogin?: boolean }
        | { type: "chatgptDeviceCode" }) => Promise<unknown>;
      cancelAccountLogin: (payload: {
        loginId: string;
      }) => Promise<unknown>;
      listAgentTypes: (cwd?: string) => Promise<{
        data: Array<{
          name: string;
          description?: string | null;
          builtIn?: boolean;
        }>;
      }>;
      listThreadProviders: (cwd?: string) => Promise<{
        data: Array<{
          id: string;
          displayName: string;
          kind: "native" | "externalCli";
          description: string;
          agentTypes: Array<{
            name: string;
            description?: string | null;
            builtIn?: boolean;
          }>;
          modelSelection: {
            mode: "catalog" | "providerDefault" | "none";
            modelProviders: string[];
          };
          capabilities: {
            startThread: boolean;
            sendInput: boolean;
            closeThread: boolean;
            listChildren: boolean;
            restoreThread: boolean;
            restoreSnapshot: boolean;
            eventStream: boolean;
            spawnChild: boolean;
            compact: boolean;
            workflow: boolean;
            pollEvent: boolean;
            commandSession: boolean;
            permissions: boolean;
            dynamicTools: boolean;
          };
        }>;
      }>;
      selectProjectDirectory: (
        defaultPath?: string,
      ) => Promise<{ path: string | null }>;
      listSkills: (cwd?: string) => Promise<{
        skills: unknown[];
        errors: string[];
      }>;
      listWorkflows: (cwd?: string) => Promise<{
        workflows: unknown[];
        diagnostics: unknown[];
      }>;
      createThread: (payload: {
        threadMode?: "chat" | "project";
        cwd?: string;
        name?: string;
        taskName?: string | null;
        threadProvider?: string | null;
        agentType?: string | null;
        model?: string | null;
        modelProvider?: string | null;
        reasoningEffort?: string | null;
        serviceTier?: string | null;
      }) => Promise<{ thread: unknown }>;
      getSelfProject: () => Promise<{
        project: null | {
          id: "/self";
          path: "/self";
          workspace: string;
          hidden: boolean;
          system: boolean;
        };
      }>;
      startSelfCommand: (payload: { text: string }) => Promise<{
        project: {
          id: "/self";
          path: "/self";
          workspace: string;
          hidden: boolean;
          system: boolean;
        };
        materializedSelfThreadId?: string | null;
        thread: unknown;
        turn: unknown;
      }>;
      archiveThread: (threadId: string) => Promise<{ ok: boolean }>;
      readThread: (
        threadId: string,
        includeTurns?: boolean,
      ) => Promise<{ thread: unknown }>;
      readCompactHistory: (threadId: string) => Promise<{ thread: unknown }>;
      setThreadRunConfig: (payload: {
        threadId: string;
        model: string;
        modelProvider: string | null;
        reasoningEffort: string;
      }) => Promise<{ ok: boolean }>;
      subscribeThread: (
        threadId: string,
      ) => Promise<{ thread?: unknown | null }>;
      unsubscribeThread: (
        threadId: string,
      ) => Promise<{ status: "unsubscribed" | "notSubscribed" }>;
      getThreadGoal: (threadId: string) => Promise<{ goal: unknown | null }>;
      setThreadGoal: (payload: {
        threadId: string;
        objective?: string;
        status?: "active" | "paused" | "budgetLimited" | "complete";
      }) => Promise<{ goal: unknown }>;
      clearThreadGoal: (threadId: string) => Promise<{ cleared: boolean }>;
      listLocalDirectory: (target: string) => Promise<{
        path: string;
        entries: Array<{
          path: string;
          name: string;
          kind: "file" | "directory";
        }>;
      }>;
      readLocalImage: (target: string) => Promise<{
        path: string;
        name: string;
        mimeType: string;
        byteSize: number;
        bytes: ArrayBuffer;
      }>;
      readGitSnapshot: (cwd: string, options?: { ref?: string | null }) => Promise<{
        available: boolean;
        root: string | null;
        treeRoot: string | null;
        branch: string | null;
        selectedRef: string | null;
        refs: Array<{
          name: string;
          fullName: string;
          head: boolean;
        }>;
        graph: Array<
          | {
              type: "commit";
              graph: string;
              hash: string;
              shortHash: string;
              parents: string[];
              refs: string[];
              subject: string;
              author: string;
              relativeTime: string;
            }
          | {
              type: "connector";
              graph: string;
            }
        >;
        changes: Array<{
          path: string;
          originalPath: string | null;
          stagedStatus: string | null;
          unstagedStatus: string | null;
          staged: boolean;
          unstaged: boolean;
        }>;
        error: string | null;
      }>;
      readGitCommitFiles: (cwd: string, hash: string) => Promise<{
        available: boolean;
        files: Array<{
          path: string;
          originalPath: string | null;
          status: string;
          score: string | null;
        }>;
        error: string | null;
      }>;
      readGitCommitFileDiff: (
        cwd: string,
        options: {
          hash: string;
          path: string;
          originalPath?: string | null;
          status?: string | null;
        },
      ) => Promise<{
        available: boolean;
        root: string | null;
        path: string | null;
        originalPath: string | null;
        staged: boolean;
        status: string | null;
        language: string;
        oldLabel: string | null;
        newLabel: string | null;
        oldContent: string;
        newContent: string;
        unifiedDiff: string;
        error: string | null;
        binary: boolean;
        modeLabel?: string | null;
        commit?: string | null;
        parent?: string | null;
      }>;
      readGitFileDiff: (
        cwd: string,
        options: {
          path: string;
          originalPath?: string | null;
          staged?: boolean;
        },
      ) => Promise<{
        available: boolean;
        root: string | null;
        path: string | null;
        originalPath: string | null;
        staged: boolean;
        status: string | null;
        language: string;
        oldLabel: string | null;
        newLabel: string | null;
        oldContent: string;
        newContent: string;
        unifiedDiff: string;
        error: string | null;
        binary: boolean;
      }>;
      readGitStatusSnapshot: (cwd: string) => Promise<{
        available: boolean;
        root: string | null;
        treeRoot: string | null;
        changes: Array<{
          path: string;
          originalPath: string | null;
          stagedStatus: string | null;
          unstagedStatus: string | null;
          staged: boolean;
          unstaged: boolean;
        }>;
        error: string | null;
      }>;
      readLocalFile: (target: string) => Promise<{
        path: string;
        displayPath: string;
        content: string;
        language: string;
        line: number | null;
        column: number | null;
        lsp: {
          enabled: boolean;
          languageId: string | null;
          lspStatus: {
            phase:
              | "plain"
              | "unavailable"
              | "starting"
              | "indexing"
              | "ready"
              | "error";
            detail: string | null;
          };
          serverLabel: string | null;
          workspaceRoot: string | null;
          reason: string | null;
        };
        image?: {
          path: string;
          mimeType: string;
          name: string;
          byteSize: number;
        } | null;
        pdf?: {
          path: string;
          mimeType: string;
          name: string;
          byteSize: number;
          url: string;
        } | null;
      }>;
      writeLocalFile: (
        target: string,
        content: string,
      ) => Promise<{
        ok: true;
        path: string;
        byteSize: number;
      }>;
      lspDefinition: (payload: {
        path: string;
        line: number;
        column: number;
      }) => Promise<{
        enabled: boolean;
        locations: Array<{
          path: string;
          line: number | null;
          column: number | null;
        }>;
        reason: string | null;
      }>;
      lspStatus: (filePath: string) => Promise<{
        enabled: boolean;
        lspStatus: {
          phase:
            | "plain"
            | "unavailable"
            | "starting"
            | "indexing"
            | "ready"
            | "error";
          detail: string | null;
        };
        reason: string | null;
        workspaceRoot: string | null;
      }>;
      openLink: (target: string) => Promise<{ ok: boolean }>;
      showBrowserView: (bounds: {
        x: number;
        y: number;
        width: number;
        height: number;
        sequence?: number;
      }) => Promise<BrowserPanelState>;
      hideBrowserView: () => Promise<BrowserPanelState>;
      setBrowserViewBounds: (bounds: {
        x: number;
        y: number;
        width: number;
        height: number;
        sequence?: number;
      }) => Promise<BrowserPanelState>;
      navigateBrowserView: (target: string) => Promise<BrowserPanelState>;
      createBrowserTab: (target?: string | null) => Promise<BrowserPanelState>;
      selectBrowserTab: (tabId: string) => Promise<BrowserPanelState>;
      closeBrowserTab: (tabId: string) => Promise<BrowserPanelState>;
      browserGoBack: () => Promise<BrowserPanelState>;
      browserGoForward: () => Promise<BrowserPanelState>;
      reloadBrowserView: () => Promise<BrowserPanelState>;
      stopBrowserView: () => Promise<BrowserPanelState>;
      getTerminalState: (threadId?: string | null) => Promise<TerminalPanelState>;
      createTerminal: (payload: {
        cwd?: string | null;
        size?: { rows: number; cols: number };
      }) => Promise<TerminalPanelState>;
      selectTerminalTab: (tabId: string) => Promise<TerminalPanelState>;
      focusTerminalCommand: (command: {
        threadId: string;
        commandItemId: string;
        processId?: string | null;
        command?: string | null;
        cwd?: string | null;
        status?: string | null;
      }) => Promise<{ state: TerminalPanelState; tabId: string }>;
      closeTerminalTab: (tabId: string) => Promise<TerminalPanelState>;
      reattachTerminalTabs: () => Promise<TerminalPanelState>;
      writeTerminal: (payload: {
        tabId: string;
        deltaBase64: string;
      }) => Promise<{ ok: true }>;
      resizeTerminal: (payload: {
        tabId: string;
        size: { rows: number; cols: number };
      }) => Promise<{ ok: true }>;
      updateTerminalPreferredSize: (payload: {
        threadId: string;
        size: { rows: number; cols: number };
      }) => Promise<{ ok: true }>;
      terminateTerminal: (tabId: string) => Promise<{ ok: true }>;
      startComputerUse: (payload?: {
        app?: string | null;
      }) => Promise<ComputerUseState>;
      observeComputerUse: () => Promise<ComputerUseState>;
      actComputerUse: (action: ComputerUseAction) => Promise<ComputerUseState>;
      stopComputerUse: () => Promise<ComputerUseState>;
      getComputerUseState: () => Promise<ComputerUseState>;
      subscribeBrowserState: (
        listener: (state: BrowserPanelState) => void,
      ) => () => void;
      subscribeTerminalState: (
        listener: (event: TerminalPanelEvent) => void,
      ) => () => void;
      sendMessage: (payload: {
        threadId: string;
        model?: string | null;
        effort?: string | null;
        text: string;
        skills?: Array<{
          name: string;
          path: string;
        }>;
        images?: Array<{
          name: string;
          mimeType: string;
          bytes: ArrayBuffer;
        }>;
        expectedTurnId?: string | null;
      }) => Promise<unknown>;
      interruptTurn: (payload: {
        threadId: string;
        turnId: string;
      }) => Promise<unknown>;
      respondServerRequest: (payload: {
        requestId: string | number;
        result: unknown;
      }) => Promise<{ ok: boolean }>;
      rejectServerRequest: (payload: {
        requestId: string | number;
        message: string;
        code?: number;
      }) => Promise<{ ok: boolean }>;
      requestMicrophoneAccess: () => Promise<{
        granted: boolean;
        status: string;
        platform: string;
      }>;
      startRealtime: (payload: {
        threadId: string;
        outputModality?: "text" | "audio";
        prompt?: string | null;
        realtimeSessionId?: string | null;
        transport?:
          | { type: "websocket" }
          | {
              type: "webrtc";
              sdp: string;
            };
        voice?: string | null;
      }) => Promise<unknown>;
      stopRealtime: (payload: { threadId: string }) => Promise<unknown>;
      subscribe: (
        listener: (event: {
          type: "notification" | "request" | "status";
          notification?: { method: string; params?: unknown };
          request?: { id: string | number; method: string; params?: unknown };
          status?: {
            connected: boolean;
            pid?: number | null;
            reason?: string;
            mobileConnection?: AndroidConnectionInfo;
            lifecycle?: {
              type:
                | "rendererReload"
                | "installedArtifactUpdate"
                | "clientRelaunch";
              phase:
                | "received"
                | "executing"
                | "preparing"
                | "selected"
                | "shuttingDownAppServer"
                | "exiting"
                | "building"
                | "updated"
                | "relaunching"
                | "reloading"
                | "reloaded"
                | "fullRelaunchFallback"
                | "completed"
                | "failed";
              requestId?: string;
              activationId?: string | null;
              releaseId?: string | null;
              reason?: string | null;
            };
            runtimeRestart?: {
              requestId?: string | null;
              requestedByThreadId?: string | null;
              phase?: string | null;
              reason?: string | null;
              createdAtMs?: number | null;
              updatedAtMs?: number | null;
              coalescedInto?: string | null;
            };
            relaunch?: {
              ok: boolean;
              relaunching: boolean;
              alreadyRequested?: boolean;
              busy?: boolean;
              conflict?: boolean;
              reason?: string | null;
            };
          };
        }) => void,
      ) => () => void;
    };
  }
}

export type AndroidConnectionInfo =
  | {
      enabled: true;
      bindEndpoint: string;
      endpoint: string;
      token: string;
      auth: "capability-token";
    }
  | {
      enabled: false;
      reason: string;
    };
