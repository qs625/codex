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
      bootstrap: () => Promise<{
        workspace: string;
        threads: unknown[];
        appServer: {
          connected: boolean;
          pid: number | null;
        };
      }>;
      listThreads: (cwd?: string) => Promise<{ data: unknown[] }>;
      createThread: (payload: { cwd?: string; name?: string }) => Promise<{ thread: unknown }>;
      archiveThread: (threadId: string) => Promise<{ ok: boolean }>;
      readThread: (threadId: string, subscribe?: boolean) => Promise<{ thread: unknown }>;
      readLocalFile: (target: string) => Promise<{
        path: string;
        displayPath: string;
        content: string;
        language: string;
        line: number | null;
        lsp: {
          enabled: boolean;
          languageId: string | null;
          serverLabel: string | null;
          workspaceRoot: string | null;
          reason: string | null;
        };
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
      openLink: (target: string) => Promise<{ ok: boolean }>;
      sendMessage: (payload: {
        threadId: string;
        text: string;
        images?: Array<{
          name: string;
          dataUrl: string;
        }>;
        expectedTurnId?: string | null;
      }) => Promise<unknown>;
      subscribe: (
        listener: (event: {
          type: "notification" | "status";
          notification?: { method: string; params?: unknown };
          status?: { connected: boolean; pid?: number | null; reason?: string };
        }) => void,
      ) => () => void;
    };
  }
}
