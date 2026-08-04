export {};

declare global {
  interface Window {
    codexDesktop: {
      health: () => Promise<{
        ok: boolean;
        appServer: {
          connected: boolean;
          pid: number | null;
        };
        workspace: string;
      }>;
      showSystemNotification: (payload: {
        title: string;
        body?: string | null;
      }) => Promise<{ ok: boolean; reason?: string }>;
      bootstrap: () => Promise<{
        workspace: string;
        threads: unknown[];
        appServer: {
          connected: boolean;
          pid: number | null;
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
      }) => Promise<{
        url: string | null;
        title: string | null;
        loading: boolean;
        canGoBack: boolean;
        canGoForward: boolean;
        error: string | null;
      }>;
      hideBrowserView: () => Promise<{
        url: string | null;
        title: string | null;
        loading: boolean;
        canGoBack: boolean;
        canGoForward: boolean;
        error: string | null;
      }>;
      setBrowserViewBounds: (bounds: {
        x: number;
        y: number;
        width: number;
        height: number;
      }) => Promise<{
        url: string | null;
        title: string | null;
        loading: boolean;
        canGoBack: boolean;
        canGoForward: boolean;
        error: string | null;
      }>;
      navigateBrowserView: (target: string) => Promise<{
        url: string | null;
        title: string | null;
        loading: boolean;
        canGoBack: boolean;
        canGoForward: boolean;
        error: string | null;
      }>;
      browserGoBack: () => Promise<{
        url: string | null;
        title: string | null;
        loading: boolean;
        canGoBack: boolean;
        canGoForward: boolean;
        error: string | null;
      }>;
      browserGoForward: () => Promise<{
        url: string | null;
        title: string | null;
        loading: boolean;
        canGoBack: boolean;
        canGoForward: boolean;
        error: string | null;
      }>;
      reloadBrowserView: () => Promise<{
        url: string | null;
        title: string | null;
        loading: boolean;
        canGoBack: boolean;
        canGoForward: boolean;
        error: string | null;
      }>;
      stopBrowserView: () => Promise<{
        url: string | null;
        title: string | null;
        loading: boolean;
        canGoBack: boolean;
        canGoForward: boolean;
        error: string | null;
      }>;
      subscribeBrowserState: (
        listener: (state: {
          url: string | null;
          title: string | null;
          loading: boolean;
          canGoBack: boolean;
          canGoForward: boolean;
          error: string | null;
        }) => void,
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
          status?: { connected: boolean; pid?: number | null; reason?: string };
        }) => void,
      ) => () => void;
    };
  }
}
