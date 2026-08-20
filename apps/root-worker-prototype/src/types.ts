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
      agentsStates: Record<string, { path?: string | null; agentNickname?: string | null; agentRole?: string | null; lifecycleStatus: ThreadLifecycleStatus; message?: string | null }>;
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
      lifecycleStatus: {
        path?: string | null;
        agentNickname?: string | null;
        agentRole?: string | null;
        lifecycleStatus: ThreadLifecycleStatus;
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
      replacementHistory?: CompactReplacementHistoryItem[] | ResponseItem[] | null;
      replacementHistoryStatus?: "missing" | "empty" | "available";
      replacementHistoryCount?: number | null;
    }
  );

export type CompactReplacementHistoryItem =
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
      type: "userMessage";
      id: string;
      content: Array<{
        type: string;
        text?: string;
        image_url?: string;
        imageUrl?: string;
        url?: string;
        path?: string;
        text_elements?: unknown[];
        textElements?: unknown[];
      }>;
    }
  | {
      type: "agentMessage";
      id: string;
      text: string;
      phase?: string | null;
      memoryCitation?: unknown | null;
    };

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

export type ThreadContextUsageToolBucket = {
  input: number;
  output: number;
};

export type ThreadContextUsageToolBreakdown = {
  applyPatch: ThreadContextUsageToolBucket;
  fileOperations: ThreadContextUsageToolBucket;
  commands: ThreadContextUsageToolBucket;
  interAgent: ThreadContextUsageToolBucket;
  searchMedia: ThreadContextUsageToolBucket;
  otherTools: ThreadContextUsageToolBucket;
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
  toolBreakdown?: ThreadContextUsageToolBreakdown;
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

export type ThreadLifecycleActiveFlag =
  | "running"
  | "waitingOnApproval"
  | "waitingOnUserInput";

export type ThreadLifecycleWaitReason =
  | "child"
  | "command"
  | "eventSubscription";

export type ThreadLifecycleFinalStatus =
  | { type: "completed"; lastAgentMessage?: string | null }
  | { type: "errored"; message?: string | null }
  | { type: "interrupted" }
  | { type: "shutdown" };

export type ThreadLifecycleStatus =
  | { type: "notLoaded" }
  | { type: "initializing" }
  | { type: "waiting"; reason: ThreadLifecycleWaitReason }
  | { type: "final"; result: ThreadLifecycleFinalStatus }
  | { type: "systemError"; message?: string | null }
  | {
      type: "active";
      activeFlags: ThreadLifecycleActiveFlag[];
    };

export type JsonRpcRequestId = string | number;

export type ApprovalRequestKind = "commandExecution" | "fileChange" | "permissions";

export type ApprovalDecision =
  | "accept"
  | "acceptForSession"
  | "decline"
  | "cancel";

export type ApprovalRequest = {
  requestId: JsonRpcRequestId;
  kind: ApprovalRequestKind;
  threadId: string;
  turnId: string;
  itemId: string;
  startedAtMs: number;
  reason: string | null;
  title: string;
  detail: string;
  metadata: Array<{ label: string; value: string }>;
  permissions?: unknown;
  status: "pending" | "submitting" | "failed";
  error: string | null;
  availableDecisions: ApprovalDecision[];
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
  lifecycleStatus: ThreadLifecycleStatus;
  path: string | null;
  cwd: string;
  cliVersion: string;
  source: SessionSource;
  threadSource: string | null;
  agentNickname: string | null;
  agentRole: string | null;
  agentPath?: string | null;
  gitInfo: unknown | null;
  name: string | null;
  skills: ThreadSkill[];
  threadUsage?: ThreadUsage | null;
  tokenUsage?: ThreadTokenUsage | null;
  contextUsage?: ThreadContextUsage | null;
  latestPlan?: ThreadPlanUpdate | null;
  turns: Turn[];
  activeSubscriptionItems?: ThreadItem[];
  activeCommandItems?: ThreadItem[];
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

export type AgentTypeOption = {
  name: string;
  description?: string | null;
  builtIn?: boolean;
};

export type AgentTypeListResponse = {
  data: AgentTypeOption[];
};

export type ThreadProviderModelSelectionMode =
  | "catalog"
  | "providerDefault"
  | "none";

export type ThreadProviderDescriptor = {
  id: string;
  displayName: string;
  kind: "native" | "externalCli";
  description: string;
  agentTypes: AgentTypeOption[];
  modelSelection: {
    mode: ThreadProviderModelSelectionMode;
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
};

export type ThreadProviderListResponse = {
  data: ThreadProviderDescriptor[];
};

export type NotificationEnvelope = {
  type: "notification" | "request" | "status" | "ready";
  notification?: {
    method: string;
    params?: unknown;
  };
  request?: {
    id: JsonRpcRequestId;
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

export type SidebarStatusClass =
  | "todo"
  | "doing"
  | "blocked"
  | "done"
  | "waiting-subagent"
  | "waiting-eventtool"
  | "waiting-subscription";

export type SidebarProjectNode = {
  id: string;
  label: string;
  subtitle: string | null;
  cwd: string;
  statusClass: SidebarStatusClass;
  updatedAt: number;
  tree: TreeNode;
  descendantCount: number;
  activeCount: number;
  waitingCount: number;
  failedCount: number;
  duplicateRootThreadIds: string[];
};

export type SidebarChatGroup = {
  id: "chat";
  statusClass: SidebarStatusClass;
  updatedAt: number;
  conversations: TreeNode[];
};

export type ProjectAgentSidebar = {
  projects: SidebarProjectNode[];
  chat: SidebarChatGroup;
};

export type NewThreadDraft = {
  mode: "project" | "chat";
  projectPath: string;
  taskName: string;
  threadProvider: string | null;
  agentType: string | null;
  model: string | null;
  modelProvider: string | null;
  reasoningEffort: string | null;
  serviceTier: string | null;
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
  turnId?: string;
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
  toolOutput?: {
    label: string;
    text: string;
    isEmpty?: boolean;
  };
  pollEventProgress?: {
    startedAtMs: number;
    currentTimeoutMs: number;
  };
  toolCategory?:
    | "command"
    | "commandNotification"
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
  kind?: "agent" | "project";
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

export type RightPanelView = "preview" | "skills" | "git" | "browser" | "workflow";

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

export type JsonConfigValue =
  | null
  | boolean
  | number
  | string
  | JsonConfigValue[]
  | { [key: string]: JsonConfigValue | undefined };

export type ConfigLayerSource =
  | { type: "mdm"; domain: string; key: string }
  | { type: "system"; file: string }
  | { type: "user"; file: string; profile: string | null }
  | { type: "project"; dotCodexFolder: string }
  | { type: "sessionFlags" }
  | { type: "legacyManagedConfigTomlFromFile"; file: string }
  | { type: "legacyManagedConfigTomlFromMdm" };

export type ConfigLayerMetadata = {
  name: ConfigLayerSource;
  version: string;
};

export type ConfigLayer = {
  name: ConfigLayerSource;
  version: string;
  config: JsonConfigValue;
  disabledReason: string | null;
};

export type ConfigReadResponse = {
  config: Record<string, JsonConfigValue | undefined>;
  origins: Record<string, ConfigLayerMetadata | undefined>;
  layers: ConfigLayer[] | null;
};

export type ConfigEdit = {
  keyPath: string;
  value: JsonConfigValue;
  mergeStrategy: "replace" | "upsert";
};

export type ConfigBatchWriteParams = {
  edits: ConfigEdit[];
  filePath?: string | null;
  expectedVersion?: string | null;
  reloadUserConfig?: boolean;
};

export type ConfigWriteResponse = {
  status: "ok" | "okOverridden";
  version: string;
  filePath: string;
  overriddenMetadata?: {
    message: string;
    overridingLayer: ConfigLayerMetadata;
    effectiveValue: JsonConfigValue;
  } | null;
};

export type Account =
  | { type: "apiKey" }
  | { type: "chatgpt"; email: string; planType: string }
  | { type: "amazonBedrock" };

export type GetAccountResponse = {
  account: Account | null;
  requiresOpenaiAuth: boolean;
};

export type LoginAccountParams =
  | { type: "apiKey"; apiKey: string }
  | { type: "chatgpt"; codexStreamlinedLogin?: boolean }
  | { type: "chatgptDeviceCode" };

export type LoginAccountResponse =
  | { type: "apiKey" }
  | { type: "chatgpt"; loginId: string; authUrl: string }
  | {
      type: "chatgptDeviceCode";
      loginId: string;
      verificationUrl: string;
      userCode: string;
    };

export type CancelLoginAccountResponse = {
  status: "canceled" | "notFound";
};

export type AccountLoginCompletedNotification = {
  loginId: string | null;
  success: boolean;
  error: string | null;
};
