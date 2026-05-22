export type SessionSource =
  | "cli"
  | "vscode"
  | "exec"
  | "appServer"
  | "unknown"
  | { custom: string }
  | { subAgent: SubAgentSource };

export type SubAgentSource =
  | "review"
  | "compact"
  | "memory_consolidation"
  | { other: string }
  | {
      thread_spawn: {
        parent_thread_id: string;
        depth: number;
        agent_path: string | null;
        agent_nickname: string | null;
        agent_role: string | null;
      };
    };

export type ThreadItem =
  | {
      type: "userMessage";
      id: string;
      content: Array<{ type: string; text?: string; image_url?: string; name?: string }>;
    }
  | {
      type: "agentMessage";
      id: string;
      text: string;
      phase: string | null;
      memoryCitation: unknown | null;
    }
  | {
      type: "plan";
      id: string;
      text: string;
    }
  | {
      type: "reasoning";
      id: string;
      summary: string[];
      content: string[];
    }
  | {
      type: "commandExecution";
      id: string;
      command: string;
      cwd: string;
      status: string;
      aggregatedOutput: string | null;
      exitCode: number | null;
      durationMs: number | null;
    }
  | {
      type: "fileChange";
      id: string;
      changes: Array<{ path: string; kind: string }>;
      status: string;
    }
  | {
      type: "collabAgentToolCall";
      id: string;
      tool: string;
      status: string;
      senderThreadId: string;
      receiverThreadIds: string[];
      prompt: string | null;
      model: string | null;
      reasoningEffort: string | null;
      agentsStates: Record<string, { status: string; message?: string | null }>;
    }
  | {
      type: "dynamicToolCall";
      id: string;
      namespace: string | null;
      tool: string;
      arguments: unknown;
      status: string;
      contentItems: unknown[] | null;
      success: boolean | null;
      durationMs: number | null;
    }
  | {
      type: "mcpToolCall";
      id: string;
      server: string;
      tool: string;
      status: string;
      arguments: unknown;
      result: unknown | null;
      error: unknown | null;
      durationMs: number | null;
    }
  | {
      type: "webSearch";
      id: string;
      query: string;
      action: string | null;
    }
  | {
      type: "enteredReviewMode" | "exitedReviewMode";
      id: string;
      review: string;
    }
  | {
      type: "contextCompaction";
      id: string;
    };

export type Turn = {
  id: string;
  items: ThreadItem[];
  itemsView: string;
  status: string;
  error: { message?: string } | null;
  startedAt: number | null;
  completedAt: number | null;
  durationMs: number | null;
};

export type Thread = {
  id: string;
  sessionId: string;
  forkedFromId: string | null;
  preview: string;
  ephemeral: boolean;
  modelProvider: string;
  model: string | null;
  reasoningEffort: string | null;
  createdAt: number;
  updatedAt: number;
  status: string;
  path: string | null;
  cwd: string;
  cliVersion: string;
  source: SessionSource;
  threadSource: string | null;
  agentNickname: string | null;
  agentRole: string | null;
  gitInfo: unknown | null;
  name: string | null;
  turns: Turn[];
};

export type BootstrapResponse = {
  workspace: string;
  threads: Thread[];
  appServer: {
    connected: boolean;
    pid: number | null;
  };
};

export type NotificationEnvelope = {
  type: "notification" | "status" | "ready";
  notification?: {
    method: string;
    params?: unknown;
  };
  status?: {
    connected: boolean;
    pid?: number | null;
    reason?: string;
  };
};

export type TreeNode = {
  key: string;
  label: string;
  path: string;
  thread: Thread | null;
  threadId: string;
  isPlaceholder: boolean;
  children: TreeNode[];
};

export type TaskFilter = "all" | "todo" | "doing" | "blocked" | "done";

export type TodoCardItem = {
  id: string;
  title: string;
  ownerPath: string;
  status: Exclude<TaskFilter, "all">;
  statusLabel: string;
  updatedLabel: string;
  summary: string;
  threadId: string;
};

export type ConversationEntry = {
  id: string;
  kind: "message" | "event" | "tool";
  author: string;
  role: "user" | "agent" | "system";
  text: string;
  timestamp: string;
  attachments: Array<{
    kind: "image" | "file";
    label: string;
    url?: string;
  }>;
  toolName?: string;
  toolStatus?: string;
  toolDetails?: string;
};

export type ConversationCell = {
  id: string;
  kind: "message" | "event" | "tool";
  entries: ConversationEntry[];
};

export type TreeMenuState = {
  threadId: string;
  x: number;
  y: number;
};

export type ComposerImage = {
  id: string;
  name: string;
  dataUrl: string;
};

export type RightPanelView = "todo" | "preview";

export type FilePreview = {
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
};

export type FileLocation = {
  path: string;
  line: number | null;
  column: number | null;
};
