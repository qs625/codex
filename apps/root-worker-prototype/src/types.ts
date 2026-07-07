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

export type ThreadItemTimestamps = {
  startedAtMs?: number | null;
  completedAtMs?: number | null;
};

export type ThreadItem = ThreadItemTimestamps &
  (
    | {
      type: "userMessage";
      id: string;
      content: Array<{
        type: string;
        text?: string;
        image_url?: string;
        name?: string;
        path?: string;
        text_elements?: unknown[];
      }>;
    }
  | {
      type: "agentMessage";
      id: string;
      text: string;
      phase: string | null;
      memoryCitation: unknown | null;
    }
  | {
      type: "injectedContext";
      id: string;
      title: string;
      preview: string;
      sections: Array<{
        label: string;
        text: string;
      }>;
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
      initialWaitMs?: number | null;
      notifyOn?: "output" | "exit" | string | null;
      aggregatedOutput: string | null;
      exitCode: number | null;
      durationMs: number | null;
    }
  | {
      type: "commandExecutionNotification";
      id: string;
      commandItemId: string;
      kind: "output" | "exit" | string;
      message: string;
      output: string | null;
      exitCode: number | null;
      createdAtMs: number;
    }
  | {
      type: "commandWait";
      id: string;
      commandId: string;
      status: "running" | "completed" | string;
      notification: "output" | "exit" | string | null;
      exitCode: number | null;
      wallTimeSeconds: number;
      waitTimeoutMs: number;
      createdAtMs: number;
    }
  | {
      type: "commandWriteStdin";
      id: string;
      commandId: string;
      bytesWritten: number;
      containsNewline: boolean;
      createdAtMs: number;
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
      senderPath: string;
      receiverThreadIds: string[];
      receiverPaths: string[];
      timeoutMs?: number | null;
      prompt: string | null;
      model: string | null;
      reasoningEffort: string | null;
      agentsStates: Record<string, { path?: string | null; status: string; message?: string | null }>;
    }
  | {
      type: "collabAgentMessage";
      id: string;
      operation: string;
      senderThreadId: string | null;
      senderPath: string;
      recipientThreadId: string | null;
      recipientPath: string;
      otherRecipientPaths: string[];
      content: string;
      triggerTurn: boolean;
    }
  | {
      type: "collabAgentStatusUpdate";
      id: string;
      senderThreadId: string | null;
      senderPath: string;
      recipientThreadId: string | null;
      recipientPath: string;
      status: {
        path?: string | null;
        status: string;
        message?: string | null;
      };
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
      type: "builtinToolCall";
      id: string;
      tool: string;
      arguments: unknown;
      status: string;
      output: unknown | null;
    }
  | {
      type: "eventDrivenToolCall";
      id: string;
      tool: string;
      arguments: unknown;
      status: string;
      output: unknown | null;
    }
  | {
      type: "eventDrivenTool";
      id: string;
      tool: string;
      title: string;
      text: string;
    }
  | {
      type: "eventCommandCall";
      id: string;
      subscriptionId: string;
      command: string;
      cwd: string | null;
      label: string | null;
      status: string;
      output: unknown | null;
    }
  | {
      type: "eventCommandEvent";
      id: string;
      subscriptionId: string;
      kind: string;
      label: string | null;
      command: string;
      cwd: string | null;
      line: string | null;
      sequence: number | null;
      exitCode: number | null;
      signal: string | null;
      message: string | null;
      truncated: boolean;
      createdAt: number;
    }
  | {
      type: "workflowRunProgress";
      id: string;
      event: ThreadWorkflowRunProgressEvent;
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
      type: "imageView";
      id: string;
      path: string | null;
    }
  | {
      type: "imageGeneration";
      id: string;
      status: string | null;
      revisedPrompt: string | null;
      result: string | null;
      savedPath: string | null;
    }
  | {
      type: "threadGoalUpdate";
      id: string;
      goal: ThreadGoal;
      action: ThreadGoalUpdateAction;
      source: ThreadGoalUpdateSource;
      previousStatus: ThreadGoalStatus | null;
    }
  | {
      type: "contextCompaction";
      id: string;
      replacementHistory?: ResponseItem[] | null;
      replacementHistoryStatus?: "missing" | "empty" | "available";
      replacementHistoryCount?: number | null;
    }
  );

export type ResponseItem = {
  type: string;
  [key: string]: unknown;
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

export type ThreadSkillKind = "explicit" | "implicit" | "all";

export type ThreadSkill = {
  name: string;
  path: string;
  kind: ThreadSkillKind;
};

export type WorkflowSource = "home" | "project";

export type WorkflowInputSpec = {
  type: string;
  description?: string | null;
};

export type WorkflowSummary = {
  id: string;
  name: string;
  description: string;
  source: WorkflowSource;
  path: string;
  entry: string;
  version?: string | null;
  whenToUse: string[];
  inputs: Record<string, WorkflowInputSpec>;
};

export type ThreadWorkflowRunProgressKind =
  | "started"
  | "resumed"
  | "completed"
  | "failed"
  | "aborted";

export type ThreadWorkflowRunProgressEvent = {
  runId: string;
  workflowId: string;
  status: unknown;
  runnerStatus: string;
  kind: ThreadWorkflowRunProgressKind;
  message: string;
  updatedAt: number;
};

export type ThreadContextUsageCategoryBreakdown = {
  compact: number;
  skillsMetadata: number;
  concreteSkills: number;
  toolsMetadata: number;
  toolCalls: number;
  userMessages: number;
  llmMessages: number;
  reasoning: number;
};

export type ThreadContextUsageSkill = {
  name: string;
  path: string;
  kind: ThreadSkillKind;
  loadCount: number;
};

export type ThreadContextUsage = {
  totalBytes: number;
  budgetUsedPercent: number | null;
  categories: ThreadContextUsageCategoryBreakdown;
  loadedSkills: {
    loadedCount: number;
    totalCount: number | null;
    skills: ThreadContextUsageSkill[];
  };
};

export type TokenUsageBreakdown = {
  totalTokens: number;
  inputTokens: number;
  cachedInputTokens: number;
  outputTokens: number;
  reasoningOutputTokens: number;
};

export type ThreadTokenUsage = {
  total: TokenUsageBreakdown;
  last: TokenUsageBreakdown;
  modelContextWindow: number | null;
};

export type ThreadUsage = {
  tokenUsage: ThreadTokenUsage | null;
  contextUsage: ThreadContextUsage | null;
};

export type ThreadActiveFlag =
  | "running"
  | "waitingOnApproval"
  | "waitingOnUserInput";

export type ThreadIdleReason = "waitCommand" | "waitChild";

export type ThreadStatus =
  | { type: "notLoaded" }
  | { type: "idle"; reason: ThreadIdleReason }
  | { type: "complete" }
  | { type: "systemError" }
  | {
      type: "active";
      activeFlags: ThreadActiveFlag[];
    };

export type ThreadGoalStatus =
  | "active"
  | "paused"
  | "budgetLimited"
  | "complete";

export type ThreadGoalUpdateAction =
  | "created"
  | "updated"
  | "paused"
  | "resumed"
  | "budgetLimited"
  | "completed";

export type ThreadGoalUpdateSource = "modelTool" | "client" | "system";

export type ThreadGoal = {
  threadId: string;
  objective: string;
  status: ThreadGoalStatus;
  tokenBudget: number | null;
  tokensUsed: number;
  timeUsedSeconds: number;
  createdAt: number;
  updatedAt: number;
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
  status: ThreadStatus;
  path: string | null;
  cwd: string;
  cliVersion: string;
  source: SessionSource;
  threadSource: string | null;
  agentNickname: string | null;
  agentRole: string | null;
  gitInfo: unknown | null;
  name: string | null;
  skills: ThreadSkill[];
  threadUsage?: ThreadUsage | null;
  tokenUsage?: ThreadTokenUsage | null;
  contextUsage?: ThreadContextUsage | null;
  latestPlan?: ThreadPlanUpdate | null;
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

export type RunReasoningEffortOption = {
  reasoningEffort: string;
  description: string;
};

export type RunModel = {
  id: string;
  model: string;
  modelProvider?: string | null;
  configured?: boolean;
  current?: boolean;
  displayName: string;
  description: string;
  hidden: boolean;
  supportedReasoningEfforts: RunReasoningEffortOption[];
  defaultReasoningEffort: string;
  contextWindow?: number | null;
  maxContextWindow?: number | null;
  autoCompactTokenLimit?: number | null;
  isDefault: boolean;
};

export type RunModelListResponse = {
  data: RunModel[];
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

export type AppServerErrorNotification = {
  threadId: string;
  turnId: string;
  willRetry: boolean;
  error: {
    message?: string;
    additionalDetails?: string | null;
  } | null;
};

export type VoiceCaptureStatus =
  | "idle"
  | "requesting"
  | "connecting"
  | "listening"
  | "stopping"
  | "error";

export type ThreadRealtimeStartedNotification = {
  threadId: string;
  realtimeSessionId: string | null;
  version: string;
};

export type ThreadRealtimeTranscriptDeltaNotification = {
  threadId: string;
  role: string;
  delta: string;
};

export type ThreadRealtimeTranscriptDoneNotification = {
  threadId: string;
  role: string;
  text: string;
};

export type ThreadRealtimeSdpNotification = {
  threadId: string;
  sdp: string;
};

export type ThreadRealtimeErrorNotification = {
  threadId: string;
  message: string;
};

export type ThreadRealtimeClosedNotification = {
  threadId: string;
  reason: string | null;
};

export type ThreadPlanStepStatus = "pending" | "inProgress" | "completed";

export type ThreadPlanStep = {
  step: string;
  status: ThreadPlanStepStatus;
};

export type ThreadPlanUpdate = {
  threadId: string;
  turnId: string;
  explanation: string | null;
  plan: ThreadPlanStep[];
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
  kind: "message" | "event" | "tool" | "compact" | "archive";
  author: string;
  role: "user" | "agent" | "system";
  text: string;
  timestamp: string;
  attachments: Array<{
    kind: "image" | "file";
    label: string;
    url?: string;
    path?: string;
  }>;
  toolName?: string;
  toolStatus?: string;
  toolDetails?: string;
  toolCategory?:
    | "command"
    | "eventDrivenSubscription"
    | "eventDrivenEvent"
    | "multiAgent"
    | "childCompletion"
    | "subagentNotification"
    | "workflow"
    | "goal"
    | "external"
    | "context";
  replacementHistoryCells?: ConversationCell[] | null;
  replacementHistoryEntries?: ConversationEntry[] | null;
  replacementHistoryStatus?: "missing" | "empty" | "available";
  replacementHistoryCount?: number | null;
  archivedCells?: ConversationCell[];
  archivedEntryCount?: number;
  isReplacementHistory?: boolean;
};

export type ConversationCell = {
  id: string;
  kind: "message" | "event" | "tool" | "compact" | "archive";
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
  mimeType: string;
  byteSize: number;
  bytes: ArrayBuffer;
  previewUrl: string;
};

export type DraftSkill = {
  name: string;
  path: string;
};

export type RightPanelView = "todo" | "preview" | "skills" | "git";

export type FilePanelView = "preview" | "tree";

export type FilePreviewImage = {
  path: string;
  mimeType: string;
  name: string;
  byteSize: number;
};

export type FilePreview = {
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
      phase: "plain" | "unavailable" | "starting" | "indexing" | "ready" | "error";
      detail: string | null;
    };
    serverLabel: string | null;
    workspaceRoot: string | null;
    reason: string | null;
  };
  image?: FilePreviewImage | null;
};

export type FileLocation = {
  path: string;
  line: number | null;
  column: number | null;
};

export type FileTreeEntry = {
  path: string;
  name: string;
  kind: "file" | "directory";
};
