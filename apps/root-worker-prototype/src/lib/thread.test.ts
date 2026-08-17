import test from "node:test";
import assert from "node:assert/strict";

import { buildConversationEntries, buildConversationState } from "./conversation";
import { buildThreadAnalysis } from "./threadAnalysis";
import {
  appendAgentDelta,
  appendCommandExecutionDelta,
  applyInitializedThreadUpdate,
  applyOrQueueInitializedThreadUpdate,
  applyPendingThreadUpdates,
  buildProjectAgentSidebar,
  buildCurrentThreadTodoItems,
  formatUpdatedLabel,
  getAgentRoleLabel,
  getPresenceLabel,
  getParentThreadId,
  getRootThreadConversationTitle,
  getThreadPresenceLabel,
  getThreadItemNotificationSyntheticTurnStatus,
  getThreadItemNotificationTargetThreadIds,
  getThreadAncestorIds,
  getThreadPath,
  getThreadSubtreeIdsChildrenFirst,
  shouldNotifyProjectThreadCompleted,
  isThreadThinking,
  markThreadCommandExecutionRunning,
  mergeDefaultCollapsedProjectIds,
  mergeThreadLifecycleStatus,
  mergeThreadSnapshot,
  normalizeThreadSnapshot,
  pickInitialProjectThread,
  preserveTerminalLifecycleStatus,
  queuePendingThreadUpdate,
  threadDisplayStatusClass,
  threadStatusClass,
  treeThreadLifecycleStatusClass,
  treeThreadLifecycleStatusLabel,
  updateThreadItem,
  updateThreadTurn,
  updateThreadTurnLifecycle,
  upsertThread,
  upsertThreadMetadataPreservingTurns,
} from "./thread";
import { CHAT_COMPAT_CWD_BASENAME } from "./chatCompat";
import type { Thread, ThreadItem, TreeNode, Turn } from "../types";

function makeThread(): Thread {
  return {
    id: "thread-1",
    sessionId: "session-1",
    forkedFromId: null,
    preview: "",
    ephemeral: false,
    modelProvider: "openai",
    model: "gpt-5",
    reasoningEffort: null,
    createdAt: 1,
    updatedAt: 1,
    lifecycleStatus: { type: "final", result: { type: "completed" } },
    path: null,
    cwd: "/tmp",
    cliVersion: "test",
    source: "cli",
    threadSource: null,
    agentNickname: null,
    agentRole: null,
    gitInfo: null,
    name: null,
    skills: [],
    turns: [],
    threadUsage: {
      tokenUsage: {
        total: {
          totalTokens: 1200,
          inputTokens: 800,
          cachedInputTokens: 100,
          outputTokens: 400,
          reasoningOutputTokens: 50,
        },
        last: {
          totalTokens: 200,
          inputTokens: 120,
          cachedInputTokens: 20,
          outputTokens: 80,
          reasoningOutputTokens: 10,
        },
        modelContextWindow: 200000,
      },
      contextUsage: {
        totalBytes: 1234,
        budgetUsedPercent: 12,
        categories: {
          compact: 0,
          skillsMetadata: 0,
          concreteSkills: 0,
          toolsMetadata: 0,
          toolCalls: 0,
          userMessages: 0,
          llmMessages: 0,
          reasoning: 0,
        },
        loadedSkills: {
          loadedCount: 0,
          totalCount: 0,
          skills: [],
        },
      },
    },
    tokenUsage: {
      total: {
        totalTokens: 1200,
        inputTokens: 800,
        cachedInputTokens: 100,
        outputTokens: 400,
        reasoningOutputTokens: 50,
      },
      last: {
        totalTokens: 200,
        inputTokens: 120,
        cachedInputTokens: 20,
        outputTokens: 80,
        reasoningOutputTokens: 10,
      },
      modelContextWindow: 200000,
    },
    contextUsage: {
      totalBytes: 1234,
      budgetUsedPercent: 12,
      categories: {
        compact: 0,
        skillsMetadata: 0,
        concreteSkills: 0,
        toolsMetadata: 0,
        toolCalls: 0,
        userMessages: 0,
        llmMessages: 0,
        reasoning: 0,
      },
      loadedSkills: {
        loadedCount: 0,
        totalCount: 0,
        skills: [],
      },
    },
  };
}

function makeTurn(id: string, items: ThreadItem[]): Turn {
  return {
    id,
    items,
    itemsView: "full",
    status: "completed",
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  };
}

function makeUserMessage(id: string, text: string): ThreadItem {
  return {
    type: "userMessage",
    id,
    content: [{ type: "text", text }],
  };
}

function makeAgentMessage(id: string, text: string): ThreadItem {
  return {
    type: "agentMessage",
    id,
    text,
    phase: null,
    memoryCitation: null,
  };
}

function makeCompactItem(id: string): ThreadItem {
  return {
    type: "contextCompaction",
    id,
    replacementHistory: [
      {
        type: "agentMessage",
        id: `${id}-seed`,
        text: "compact seed",
        phase: null,
        memoryCitation: null,
      },
    ],
  };
}

function makeCollabStatusItem(id: string): ThreadItem {
  return {
    type: "collabAgentStatusUpdate",
    id,
    senderThreadId: "child-thread",
    senderPath: "/root/child",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    lifecycleStatus: collabAgentLifecycleState("completed"),
  };
}

function collabAgentLifecycleState(
  status: "completed" | "errored" | "shutdown" | "notFound",
  message: string | null = "done",
  path: string | null = "/root/worker",
): Extract<ThreadItem, { type: "collabAgentStatusUpdate" }>["lifecycleStatus"] {
  const lifecycleStatus =
    status === "completed"
      ? { type: "final" as const, result: { type: "completed" as const, lastAgentMessage: message } }
      : status === "errored"
        ? { type: "final" as const, result: { type: "errored" as const, message } }
        : status === "shutdown"
          ? { type: "final" as const, result: { type: "shutdown" as const } }
          : { type: "notLoaded" as const };

  return {
    path,
    lifecycleStatus,
    message,
  };
}

function makeInitContextItem(
  id: string,
): Extract<ThreadItem, { type: "injectedContext" }> {
  return {
    type: "injectedContext",
    id,
    title: "Init Context",
    preview: "Workspace • Instructions",
    sections: [
      { label: "Workspace", text: "/tmp/project" },
      { label: "Instructions", text: "全程使用中文" },
    ],
  };
}

function makeSidebarThread(overrides: Partial<Thread>): Thread {
  return {
    ...makeThread(),
    ...overrides,
    turns: overrides.turns ?? [],
    skills: overrides.skills ?? [],
  };
}

function makeSubagentThread(
  id: string,
  parentThreadId: string,
  agentPath: string,
  overrides: Partial<Thread> = {},
): Thread {
  return makeSidebarThread({
    id,
    sessionId: `session-${id}`,
    cwd: overrides.cwd ?? "/other/cwd/that/should/not/group",
    createdAt: overrides.createdAt ?? 10,
    updatedAt: overrides.updatedAt ?? 10,
    source: {
      subAgent: {
        thread_spawn: {
          parent_thread_id: parentThreadId,
          depth: 1,
          agent_path: agentPath,
          agent_nickname: agentPath.split("/").at(-1) ?? id,
          agent_role: overrides.agentRole ?? "worker",
        },
      },
    },
    threadSource: "subagent",
    agentRole: overrides.agentRole ?? "worker",
    ...overrides,
  });
}

function makeTreeNode(
  thread: Thread | null,
  children: TreeNode[] = [],
): TreeNode {
  return {
    key: thread?.id ?? "placeholder",
    label: thread?.name ?? thread?.id ?? "placeholder",
    path: thread?.path ?? "/root",
    thread,
    threadId: thread?.id ?? "placeholder",
    isPlaceholder: thread === null,
    children,
  };
}

test("buildProjectAgentSidebar groups parentless project chat roots by cwd", () => {
  const projectA = makeSidebarThread({
    id: "project-a",
    name: "Alpha chat",
    cwd: "/work/alpha",
    createdAt: 1,
    updatedAt: 4,
  });
  const projectB = makeSidebarThread({
    id: "project-b",
    name: "Beta chat",
    cwd: "/work/beta",
    createdAt: 2,
    updatedAt: 8,
  });
  const owner = makeSubagentThread("owner-a", "project-a", "/root/owner");

  const sidebar = buildProjectAgentSidebar([projectA, projectB, owner]);

  assert.deepEqual(
    sidebar.projects.map((project) => project.label),
    ["alpha", "beta"],
  );
  const alpha = sidebar.projects.find((project) => project.cwd === "/work/alpha");
  assert.equal(alpha?.tree.threadId, "project-a");
  assert.equal(alpha?.tree.label, "Alpha chat");
  assert.equal(alpha?.tree.children[0]?.threadId, "owner-a");
  assert.equal(alpha?.descendantCount, 1);
});

test("mergeDefaultCollapsedProjectIds collapses untouched projects by default", () => {
  const collapsedProjectIds = mergeDefaultCollapsedProjectIds(
    [],
    ["project:/work/alpha", "project:/work/beta"],
    new Set(),
  );

  assert.deepEqual(collapsedProjectIds, [
    "project:/work/alpha",
    "project:/work/beta",
  ]);
});

test("mergeDefaultCollapsedProjectIds preserves touched project state across updates", () => {
  const touchedProjectIds = new Set(["project:/work/alpha"]);
  const openedProjectIds = mergeDefaultCollapsedProjectIds(
    [],
    ["project:/work/alpha"],
    touchedProjectIds,
  );
  const collapsedProjectIds = mergeDefaultCollapsedProjectIds(
    ["project:/work/alpha"],
    ["project:/work/alpha"],
    touchedProjectIds,
  );

  assert.deepEqual(openedProjectIds, []);
  assert.deepEqual(collapsedProjectIds, ["project:/work/alpha"]);
});

test("mergeDefaultCollapsedProjectIds collapses new untouched projects", () => {
  const collapsedProjectIds = mergeDefaultCollapsedProjectIds(
    [],
    ["project:/work/alpha", "project:/work/beta"],
    new Set(["project:/work/alpha"]),
  );

  assert.deepEqual(collapsedProjectIds, ["project:/work/beta"]);
});

test("buildProjectAgentSidebar places no-cwd parentless threads in Chat", () => {
  const project = makeSidebarThread({ id: "pm", cwd: "/work/project" });
  const chatA = makeSidebarThread({
    id: "chat-a",
    name: "API question",
    cwd: "",
    updatedAt: 4,
  });
  const chatB = makeSidebarThread({
    id: "chat-b",
    name: "General Q&A",
    cwd: "   ",
    updatedAt: 7,
  });

  const sidebar = buildProjectAgentSidebar([project, chatA, chatB]);

  assert.equal(sidebar.projects.length, 1);
  assert.deepEqual(
    sidebar.chat.conversations.map((node) => node.threadId),
    ["chat-b", "chat-a"],
  );
  assert.equal(sidebar.chat.conversations[0]?.label, "General Q&A");
});

test("buildProjectAgentSidebar treats chat compat cwd as Chat", () => {
  const project = makeSidebarThread({ id: "pm", cwd: "/work/project" });
  const chat = makeSidebarThread({
    id: "chat",
    name: "查看 Claude tools",
    cwd: `/tmp/root-worker/${CHAT_COMPAT_CWD_BASENAME}`,
    updatedAt: 9,
  });

  const sidebar = buildProjectAgentSidebar([project, chat]);

  assert.equal(sidebar.projects.length, 1);
  assert.equal(sidebar.projects[0]?.cwd, "/work/project");
  assert.deepEqual(
    sidebar.chat.conversations.map((node) => node.threadId),
    ["chat"],
  );
  assert.equal(sidebar.chat.conversations[0]?.label, "查看 Claude tools");
});

test("buildProjectAgentSidebar keeps Chat conversation subagents visible", () => {
  const chat = makeSidebarThread({
    id: "chat",
    name: "General Q&A",
    cwd: "",
  });
  const helper = makeSubagentThread("chat-helper", "chat", "/root/helper", {
    cwd: "/work/project",
  });

  const sidebar = buildProjectAgentSidebar([chat, helper]);

  assert.equal(sidebar.projects.length, 0);
  assert.equal(sidebar.chat.conversations[0]?.threadId, "chat");
  assert.equal(
    sidebar.chat.conversations[0]?.children[0]?.threadId,
    "chat-helper",
  );
});

test("root thread labels distinguish project roots from no-project chats", () => {
  const project = makeSidebarThread({
    id: "project",
    name: "Project chat",
    cwd: "/work/project",
    agentPath: "/my_codex",
  });
  const chat = makeSidebarThread({
    id: "chat",
    name: "General Q&A",
    cwd: "",
  });
  const compatChat = makeSidebarThread({
    id: "compat-chat",
    name: "查看 TAE managed agent API",
    cwd: `/tmp/root-worker/${CHAT_COMPAT_CWD_BASENAME}`,
    agentPath: "/my_codex",
  });

  assert.equal(getRootThreadConversationTitle(project), "/my_codex");
  assert.equal(getAgentRoleLabel(project), "/my_codex");
  assert.equal(getRootThreadConversationTitle(chat), "General Q&A");
  assert.equal(getAgentRoleLabel(chat), "Chat");
  assert.equal(
    getRootThreadConversationTitle(compatChat),
    "查看 TAE managed agent API",
  );
  assert.equal(getAgentRoleLabel(compatChat), "Chat");
});

test("getThreadAncestorIds returns ancestors for selected Chat subagents", () => {
  const chat = makeSidebarThread({
    id: "chat",
    name: "General Q&A",
    cwd: "",
  });
  const helper = makeSubagentThread("chat-helper", "chat", "/root/helper");
  const reviewer = makeSubagentThread(
    "chat-reviewer",
    "chat-helper",
    "/root/helper/reviewer",
  );

  assert.deepEqual(getThreadAncestorIds([chat, helper, reviewer], "chat-reviewer"), [
    "chat-helper",
    "chat",
  ]);
});

test("buildProjectAgentSidebar hides duplicate parentless roots for the same project", () => {
  const olderRoot = makeSidebarThread({
    id: "older-root",
    cwd: "/work/project",
    updatedAt: 2,
  });
  const activeRoot = makeSidebarThread({
    id: "active-root",
    cwd: "/work/project/",
    updatedAt: 1,
    lifecycleStatus: { type: "active" as const, activeFlags: ["running"] },
  });

  const sidebar = buildProjectAgentSidebar([olderRoot, activeRoot]);

  assert.equal(sidebar.projects.length, 1);
  assert.equal(sidebar.projects[0]?.tree.threadId, "active-root");
  assert.deepEqual(sidebar.projects[0]?.duplicateRootThreadIds, ["older-root"]);
});

test("pickInitialProjectThread follows sidebar canonical project root selection", () => {
  const olderRoot = makeSidebarThread({
    id: "older-root",
    cwd: "/work/project",
    updatedAt: 9,
  });
  const activeRoot = makeSidebarThread({
    id: "active-root",
    cwd: "/work/project",
    updatedAt: 1,
    lifecycleStatus: { type: "active" as const, activeFlags: ["running"] },
  });

  const picked = pickInitialProjectThread([olderRoot, activeRoot]);

  assert.equal(picked?.id, "active-root");
});

test("buildProjectAgentSidebar makes subagents inherit their project root", () => {
  const root = makeSidebarThread({ id: "project-root", cwd: "/work/project" });
  const owner = makeSubagentThread("owner", "project-root", "/root/owner", {
    cwd: "/another/repo",
  });
  const reviewer = makeSubagentThread("reviewer", "owner", "/root/owner/reviewer");

  const sidebar = buildProjectAgentSidebar([root, owner, reviewer]);
  const project = sidebar.projects[0];

  assert.equal(project?.cwd, "/work/project");
  assert.equal(project?.tree.children[0]?.threadId, "owner");
  assert.equal(project?.tree.children[0]?.children[0]?.threadId, "reviewer");
  assert.equal(sidebar.projects.length, 1);
});

test("buildProjectAgentSidebar keeps live external codex cli subagents under their parent", () => {
  const root = makeSidebarThread({ id: "project-root", cwd: "/work/project" });
  const external = makeSubagentThread(
    "external-codex",
    "project-root",
    "/root/codex_external_subagent",
    {
      agentRole: "codex_cli",
      agentNickname: "codex_cli",
      lifecycleStatus: {
        type: "final" as const,
        result: { type: "completed" as const, lastAgentMessage: "ready" },
      },
    },
  );

  const sidebar = buildProjectAgentSidebar([root, external]);
  const child = sidebar.projects[0]?.tree.children[0];

  assert.equal(child?.threadId, "external-codex");
  assert.equal(child?.label, "codex_external_subagent");
  assert.deepEqual(child?.thread?.lifecycleStatus, {
    type: "final",
    result: { type: "completed", lastAgentMessage: "ready" },
  });
  assert.equal(sidebar.chat.conversations.length, 0);
});

test("getThreadSubtreeIdsChildrenFirst returns only a project subtree deepest first", () => {
  const root = makeSidebarThread({ id: "project-root", cwd: "/work/project" });
  const owner = makeSubagentThread("owner", "project-root", "/root/owner");
  const reviewer = makeSubagentThread("reviewer", "owner", "/root/owner/reviewer");
  const siblingProject = makeSidebarThread({
    id: "sibling-root",
    cwd: "/work/sibling",
  });
  const chat = makeSidebarThread({ id: "chat", cwd: "" });

  assert.deepEqual(
    getThreadSubtreeIdsChildrenFirst([
      root,
      owner,
      reviewer,
      siblingProject,
      chat,
    ], "project-root"),
    ["reviewer", "owner", "project-root"],
  );
});

test("buildProjectAgentSidebar uses root thread status and aggregates project counts", () => {
  const pm = makeSidebarThread({
    id: "pm",
    cwd: "/work/project",
    lifecycleStatus: { type: "waiting" as const, reason: "child" as const },
  });
  const activeOwner = makeSubagentThread("owner", "pm", "/root/owner", {
    lifecycleStatus: { type: "active" as const, activeFlags: ["running"] },
  });
  const waitingReviewer = makeSubagentThread(
    "reviewer",
    "pm",
    "/root/reviewer",
    {
      lifecycleStatus: { type: "waiting" as const, reason: "child" as const },
    },
  );
  const failedWorker = makeSubagentThread("worker", "pm", "/root/worker", {
    lifecycleStatus: { type: "systemError" as const },
  });

  const sidebar = buildProjectAgentSidebar([
    pm,
    activeOwner,
    waitingReviewer,
    failedWorker,
  ]);
  const project = sidebar.projects[0];

  assert.equal(project?.statusClass, "waiting-subagent");
  assert.equal(project?.activeCount, 1);
  assert.equal(project?.waitingCount, 2);
  assert.equal(project?.failedCount, 1);
});

test("mergeThreadSnapshot preserves usage fields when thread/read omits them", () => {
  const existing = makeThread();
  const next = {
    ...makeThread(),
    preview: "fresh preview",
    turns: [
      {
        id: "turn-1",
        items: [],
        itemsView: "full" as const,
        status: "completed" as const,
        error: null,
        startedAt: 1,
        completedAt: 2,
        durationMs: 1,
      },
    ],
  };
  delete next.tokenUsage;
  delete next.contextUsage;
  delete next.threadUsage;

  const merged = mergeThreadSnapshot(existing, next);

  assert.equal(merged.preview, "fresh preview");
  assert.equal(merged.turns.length, 1);
  assert.equal(merged.threadUsage?.tokenUsage?.total.totalTokens, 1200);
  assert.equal(merged.threadUsage?.contextUsage?.budgetUsedPercent, 12);
  assert.equal(merged.tokenUsage?.total.totalTokens, 1200);
  assert.equal(merged.contextUsage?.budgetUsedPercent, 12);
});

test("mergeThreadSnapshot does not downgrade completed lifecycle from stale metadata", () => {
  const existing = {
    ...makeThread(),
    lifecycleStatus: { type: "final", result: { type: "completed" } },
  } satisfies Thread;
  const staleWaiting = {
    ...makeThread(),
    lifecycleStatus: { type: "waiting", reason: "command" },
  } satisfies Thread;

  const merged = mergeThreadSnapshot(existing, staleWaiting);

  assert.deepEqual(merged.lifecycleStatus, {
    type: "final",
    result: { type: "completed" },
  });
});

test("upsertThreadMetadataPreservingTurns does not downgrade completed lifecycle", () => {
  const existing = {
    ...makeThread(),
    lifecycleStatus: { type: "final", result: { type: "completed" } },
  } satisfies Thread;
  const staleWaiting = {
    ...makeThread(),
    lifecycleStatus: { type: "waiting", reason: "command" },
  } satisfies Thread;

  const threads = upsertThreadMetadataPreservingTurns([existing], staleWaiting);

  assert.deepEqual(threads[0]?.lifecycleStatus, {
    type: "final",
    result: { type: "completed" },
  });
});

test("preserveTerminalLifecycleStatus ignores stale waiting status notifications", () => {
  const completed = {
    type: "final" as const,
    result: { type: "completed" as const },
  };
  const staleWaiting = {
    type: "waiting" as const,
    reason: "eventSubscription" as const,
  };

  assert.deepEqual(
    preserveTerminalLifecycleStatus(completed, staleWaiting),
    completed,
  );
});

test("authoritative active status notification overrides stale completed lifecycle", () => {
  const completed = {
    type: "final" as const,
    result: { type: "completed" as const },
  };
  const liveActive = {
    type: "active" as const,
    activeFlags: ["running" as const],
  };

  assert.deepEqual(
    mergeThreadLifecycleStatus(completed, liveActive, {
      authoritative: true,
    }),
    liveActive,
  );
});

test("authoritative weak status notifications do not clear strong terminal lifecycle", () => {
  const shutdown = {
    type: "final" as const,
    result: { type: "shutdown" as const },
  };
  const errored = {
    type: "final" as const,
    result: { type: "errored" as const, message: "failed" },
  };

  assert.deepEqual(
    mergeThreadLifecycleStatus(
      shutdown,
      { type: "notLoaded" as const },
      { authoritative: true },
    ),
    shutdown,
  );
  assert.deepEqual(
    mergeThreadLifecycleStatus(
      errored,
      { type: "waiting" as const, reason: "eventSubscription" as const },
      { authoritative: true },
    ),
    errored,
  );
  assert.deepEqual(
    mergeThreadLifecycleStatus(
      shutdown,
      { type: "final" as const, result: { type: "completed" as const } },
      { authoritative: true },
    ),
    shutdown,
  );
});

test("agent tree does not show waiting after completed status is preserved", () => {
  const root = makeSidebarThread({ id: "project-root", cwd: "/work/project" });
  const completedOwner = makeSubagentThread(
    "owner",
    "project-root",
    "/root/owner",
    {
      lifecycleStatus: {
        type: "final" as const,
        result: { type: "completed" as const },
      },
    },
  );
  const staleWaiting = {
    type: "waiting" as const,
    reason: "eventSubscription" as const,
  };
  const ownerAfterStaleStatus = {
    ...completedOwner,
    lifecycleStatus: preserveTerminalLifecycleStatus(
      completedOwner.lifecycleStatus,
      staleWaiting,
    ),
  };

  const sidebar = buildProjectAgentSidebar([root, ownerAfterStaleStatus]);
  const child = sidebar.projects[0]?.tree.children[0];

  assert.deepEqual(child?.thread?.lifecycleStatus, {
    type: "final",
    result: { type: "completed" },
  });
  assert.ok(child);
  const statusClass = treeThreadLifecycleStatusClass(child);
  assert.equal(treeThreadLifecycleStatusLabel(statusClass), "Inactive");
  assert.notEqual(statusClass, "waiting-subscription");
});

test("mergeThreadSnapshot does not downgrade non-completed final lifecycle", () => {
  const existing = {
    ...makeThread(),
    lifecycleStatus: { type: "final", result: { type: "interrupted" } },
  } satisfies Thread;
  const staleWaiting = {
    ...makeThread(),
    lifecycleStatus: { type: "waiting", reason: "command" },
  } satisfies Thread;

  const merged = mergeThreadSnapshot(existing, staleWaiting);

  assert.deepEqual(merged.lifecycleStatus, {
    type: "final",
    result: { type: "interrupted" },
  });
});

test("updateThreadItem creates a running turn when item notifications arrive first", () => {
  const thread = makeThread();

  const updated = updateThreadItem(
    thread,
    "turn-1",
    {
      type: "eventDrivenToolCall",
      id: "item-1",
      tool: "process_exit_subscribe",
      arguments: { session_id: 42 },
      status: "inProgress",
      output: null,
    },
    { startedAtMs: 2_000 },
  );

  assert.deepEqual(updated.turns, [
    {
      id: "turn-1",
      items: [
        {
          type: "eventDrivenToolCall",
          id: "item-1",
          tool: "process_exit_subscribe",
          arguments: { session_id: 42 },
          status: "inProgress",
          output: null,
          startedAtMs: 2_000,
        },
      ],
      itemsView: "full",
      status: "running",
      error: null,
      startedAt: 2,
      completedAt: null,
      durationMs: null,
    },
  ]);
});

test("command item notifications create a visible running command and complete the same item", () => {
  const started = updateThreadItem(
    makeThread(),
    "turn-1",
    {
      type: "commandExecution",
      id: "cmd-1",
      command: "rtk sleep 1",
      cwd: "/tmp/project",
      status: "inProgress",
      initialWaitMs: 1000,
      notifyOn: "output",
      aggregatedOutput: null,
      exitCode: null,
      durationMs: null,
    },
    { startedAtMs: 2_000 },
  );

  assert.equal(started.turns.length, 1);
  assert.equal(started.turns[0]?.status, "running");
  assert.deepEqual(
    buildConversationEntries(started).map((entry) => [
      entry.id,
      entry.kind,
      entry.toolCategory,
      entry.toolStatus,
      entry.text,
    ]),
    [["cmd-1", "tool", "command", "inProgress", "tmp/project • running"]],
  );

  const withOutput = appendCommandExecutionDelta(
    started,
    "turn-1",
    "cmd-1",
    "running\n",
  );
  assert.equal(
    (withOutput.turns[0]?.items[0] as Extract<
      ThreadItem,
      { type: "commandExecution" }
    >).aggregatedOutput,
    "running\n",
  );

  const completed = updateThreadItem(
    withOutput,
    "turn-1",
    {
      type: "commandExecution",
      id: "cmd-1",
      command: "rtk sleep 1",
      cwd: "/tmp/project",
      status: "completed",
      initialWaitMs: 1000,
      notifyOn: "output",
      aggregatedOutput: "running\n",
      exitCode: 0,
      durationMs: 1000,
    },
    { completedAtMs: 3_000 },
  );

  assert.equal(completed.turns.length, 1);
  assert.equal(completed.turns[0]?.items.length, 1);
  assert.deepEqual(completed.turns[0]?.items[0], {
    type: "commandExecution",
    id: "cmd-1",
    command: "rtk sleep 1",
    cwd: "/tmp/project",
    status: "completed",
    initialWaitMs: 1000,
    notifyOn: "output",
    aggregatedOutput: "running\n",
    exitCode: 0,
    durationMs: 1000,
    startedAtMs: 2_000,
    completedAtMs: 3_000,
  });
});

test("command output delta creates a visible placeholder when start was missed", () => {
  const withOutput = appendCommandExecutionDelta(
    makeThread(),
    "turn-1",
    "cmd-1",
    "running\n",
  );

  assert.equal(withOutput.turns.length, 1);
  assert.deepEqual(withOutput.turns[0]?.items, [
    {
      type: "commandExecution",
      id: "cmd-1",
      command: "Command output",
      cwd: "cwd pending",
      status: "inProgress",
      initialWaitMs: null,
      notifyOn: null,
      aggregatedOutput: "running\n",
      exitCode: null,
      durationMs: null,
    },
  ]);
  assert.deepEqual(
    buildConversationEntries(withOutput).map((entry) => [
      entry.id,
      entry.kind,
      entry.toolCategory,
      entry.toolStatus,
    ]),
    [["cmd-1", "tool", "command", "inProgress"]],
  );

  const started = updateThreadItem(
    withOutput,
    "turn-1",
    {
      type: "commandExecution",
      id: "cmd-1",
      command: "rtk printf running",
      cwd: "/tmp/project",
      status: "inProgress",
      initialWaitMs: 1000,
      notifyOn: "output",
      aggregatedOutput: null,
      exitCode: null,
      durationMs: null,
    },
    { startedAtMs: 2_000 },
  );

  assert.deepEqual(started.turns[0]?.items, [
    {
      type: "commandExecution",
      id: "cmd-1",
      command: "rtk printf running",
      cwd: "/tmp/project",
      status: "inProgress",
      initialWaitMs: 1000,
      notifyOn: "output",
      aggregatedOutput: "running\n",
      exitCode: null,
      durationMs: null,
      startedAtMs: 2_000,
    },
  ]);

  const completed = updateThreadItem(
    started,
    "turn-1",
    {
      type: "commandExecution",
      id: "cmd-1",
      command: "rtk printf running",
      cwd: "/tmp/project",
      status: "completed",
      initialWaitMs: 1000,
      notifyOn: "output",
      aggregatedOutput: "running\n",
      exitCode: 0,
      durationMs: 1000,
    },
    { completedAtMs: 3_000 },
  );

  assert.deepEqual(completed.turns[0]?.items, [
    {
      type: "commandExecution",
      id: "cmd-1",
      command: "rtk printf running",
      cwd: "/tmp/project",
      status: "completed",
      initialWaitMs: 1000,
      notifyOn: "output",
      aggregatedOutput: "running\n",
      exitCode: 0,
      durationMs: 1000,
      startedAtMs: 2_000,
      completedAtMs: 3_000,
    },
  ]);
});

test("updateThreadItem preserves started time when completion updates the same item", () => {
  const thread = updateThreadItem(
    makeThread(),
    "turn-1",
    {
      type: "eventDrivenToolCall",
      id: "item-1",
      tool: "process_exit_subscribe",
      arguments: { session_id: 42 },
      status: "inProgress",
      output: null,
    },
    { startedAtMs: 2_000 },
  );

  const updated = updateThreadItem(
    thread,
    "turn-1",
    {
      type: "eventDrivenToolCall",
      id: "item-1",
      tool: "process_exit_subscribe",
      arguments: { session_id: 42 },
      status: "completed",
      output: { subscription_id: "sub-1" },
    },
    { completedAtMs: 3_000 },
  );

  assert.deepEqual(updated.turns[0]?.items[0], {
    type: "eventDrivenToolCall",
    id: "item-1",
    tool: "process_exit_subscribe",
    arguments: { session_id: 42 },
    status: "completed",
    output: { subscription_id: "sub-1" },
    startedAtMs: 2_000,
    completedAtMs: 3_000,
  });
});

test("command wait item notifications create visible conversation entries", () => {
  const updated = updateThreadItem(
    makeThread(),
    "turn-1",
    {
      type: "commandWait",
      id: "wait-1",
      commandId: "7",
      status: "completed",
      notification: "exit",
      exitCode: 0,
      wallTimeSeconds: 0.25,
      waitTimeoutMs: 1_000,
      createdAtMs: 2_000,
    },
    { completedAtMs: 2_000 },
  );

  const entries = buildConversationEntries(updated);

  assert.deepEqual(
    entries.map((entry) => [entry.id, entry.kind, entry.text]),
    [
      [
        "wait-1",
        "event",
        "Waited for command 7 with timeout 1s after exit notification: completed, exit 0 in 250ms.",
      ],
    ],
  );
});

test("codex cli project external root item notifications keep assistant messages visible", () => {
  const externalProjectThread = makeSidebarThread({
    id: "external-project",
    sessionId: "session-external-project",
    modelProvider: "codex_cli",
    model: null,
    cwd: "/work/project",
    source: "appServer",
    threadSource: "user",
    agentPath: "/cp_http_api",
    agentNickname: "cp_http_api",
    agentRole: "codex_cli",
    lifecycleStatus: { type: "active", activeFlags: ["running"] },
  });
  const turnId = "turn-external-project";
  const userItem = makeUserMessage("user-1", "status?");
  const agentItem = makeAgentMessage("agent-1", "External assistant done");

  assert.deepEqual(
    getThreadItemNotificationTargetThreadIds(
      externalProjectThread.id,
      agentItem,
    ),
    [externalProjectThread.id],
  );

  const withUser = updateThreadItem(externalProjectThread, turnId, userItem, {
    completedAtMs: 2_000,
    syntheticTurnStatus: getThreadItemNotificationSyntheticTurnStatus(
      "item/completed",
      userItem,
    ),
  });
  const withAssistant = updateThreadItem(withUser, turnId, agentItem, {
    completedAtMs: 3_000,
    syntheticTurnStatus: getThreadItemNotificationSyntheticTurnStatus(
      "item/completed",
      agentItem,
    ),
  });
  const completed = updateThreadTurnLifecycle(withAssistant, {
    id: turnId,
    items: [],
    itemsView: "full",
    status: "completed",
    error: null,
    startedAt: 2,
    completedAt: 3,
    durationMs: 1000,
  });

  assert.equal(completed.turns.length, 1);
  assert.equal(completed.turns[0]?.status, "completed");
  assert.deepEqual(
    completed.turns[0]?.items.map((item) => item.type),
    ["userMessage", "agentMessage"],
  );

  const entries = buildConversationEntries(completed);

  assert.deepEqual(
    entries.map((entry) => ({
      id: entry.id,
      role: entry.role,
      text: entry.text,
    })),
    [
      { id: "user-1", role: "user", text: "status?" },
      { id: "agent-1", role: "agent", text: "External assistant done" },
    ],
  );
});

test("injected init context item notifications create visible conversation entries", () => {
  const updated = updateThreadItem(
    makeThread(),
    "turn-1",
    makeInitContextItem("ctx-1"),
    {
      completedAtMs: 2_000,
      syntheticTurnStatus: getThreadItemNotificationSyntheticTurnStatus(
        "item/completed",
        makeInitContextItem("ctx-1"),
      ),
    },
  );

  const entries = buildConversationEntries(updated);
  assert.equal(updated.turns[0]?.status, "completed");

  assert.deepEqual(
    entries.map((entry) => ({
      id: entry.id,
      kind: entry.kind,
      text: entry.text,
      toolName: entry.toolName,
      toolCategory: entry.toolCategory,
    })),
    [
      {
        id: "ctx-1",
        kind: "tool",
        text: "Workspace • Instructions",
        toolName: "Init Context",
        toolCategory: "context",
      },
    ],
  );
});

test("injected init context notification merges with existing init snapshot", () => {
  const existing = {
    ...makeThread(),
    turns: [
      {
        id: "turn-start",
        items: [makeInitContextItem("ctx-start")],
        itemsView: "full" as const,
        status: "completed" as const,
        error: null,
        startedAt: 1,
        completedAt: 2,
        durationMs: 1000,
      },
    ],
  };

  const updated = updateThreadItem(
    existing,
    "turn-notification",
    makeInitContextItem("ctx-notification"),
    {
      completedAtMs: 2_000,
      syntheticTurnStatus: getThreadItemNotificationSyntheticTurnStatus(
        "item/completed",
        makeInitContextItem("ctx-notification"),
      ),
    },
  );

  const entries = buildConversationEntries(updated);

  assert.equal(updated.turns.length, 1);
  assert.equal(updated.turns[0]?.status, "completed");
  assert.deepEqual(
    entries
      .filter((entry) => entry.toolName === "Init Context")
      .map((entry) => entry.text),
    ["Workspace • Instructions"],
  );
});

test("mergeThreadSnapshot drops equivalent init context from later snapshots", () => {
  const existing = {
    ...makeThread(),
    turns: [
      {
        id: "turn-start",
        items: [makeInitContextItem("ctx-start")],
        itemsView: "full" as const,
        status: "completed" as const,
        error: null,
        startedAt: 1,
        completedAt: 2,
        durationMs: 1000,
      },
    ],
  };
  const nextTurn = {
    id: "turn-read",
    items: [makeInitContextItem("ctx-read")],
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 1,
    completedAt: 2,
    durationMs: 1000,
  };

  const merged = mergeThreadSnapshot(existing, {
    ...makeThread(),
    turns: [nextTurn],
  });

  assert.deepEqual(merged.turns, [nextTurn]);
});

test("mergeThreadSnapshot keeps one completed init context after first user turn", () => {
  const initContextTurn: Turn = {
    id: "turn-init",
    items: [makeInitContextItem("ctx-start")],
    itemsView: "full",
    status: "completed",
    error: null,
    startedAt: 1,
    completedAt: 2,
    durationMs: 1000,
  };
  const userTurn: Turn = {
    id: "turn-user",
    items: [
      makeInitContextItem("ctx-read"),
      {
        type: "userMessage",
        id: "user-1",
        content: [{ type: "text", text: "hello" }],
      },
    ],
    itemsView: "full",
    status: "completed",
    error: null,
    startedAt: 3,
    completedAt: 4,
    durationMs: 1000,
  };

  const merged = mergeThreadSnapshot(
    {
      ...makeThread(),
      turns: [initContextTurn],
    },
    {
      ...makeThread(),
      turns: [userTurn],
    },
  );
  const initContextEntries = buildConversationEntries(merged).filter(
    (entry) => entry.toolName === "Init Context",
  );

  assert.equal(initContextEntries.length, 1);
  assert.deepEqual(
    merged.turns
      .flatMap((turn) =>
        turn.items
          .filter((item) => item.type === "injectedContext")
          .map(() => turn.status),
      ),
    ["completed"],
  );
});

test("mergeThreadSnapshot preserves distinct non-init injected contexts", () => {
  const existingContext: ThreadItem = {
    type: "injectedContext",
    id: "ctx-existing",
    title: "Runtime Context",
    preview: "Workspace",
    sections: [{ label: "Workspace", text: "/tmp/project" }],
  };
  const nextContext: ThreadItem = {
    ...existingContext,
    id: "ctx-next",
  };
  const existing = {
    ...makeThread(),
    turns: [
      {
        id: "turn-existing",
        items: [existingContext],
        itemsView: "full" as const,
        status: "completed" as const,
        error: null,
        startedAt: 1,
        completedAt: 2,
        durationMs: 1000,
      },
    ],
  };
  const nextTurn = {
    id: "turn-next",
    items: [nextContext],
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 3,
    completedAt: 4,
    durationMs: 1000,
  };

  const merged = mergeThreadSnapshot(existing, {
    ...makeThread(),
    turns: [nextTurn],
  });

  assert.deepEqual(merged.turns, [nextTurn, existing.turns[0]]);
});

test("updateThreadTurn preserves item timestamps when a completed turn snapshot arrives", () => {
  const thread = updateThreadItem(
    makeThread(),
    "turn-1",
    {
      type: "eventDrivenToolCall",
      id: "item-1",
      tool: "process_exit_subscribe",
      arguments: { session_id: 42 },
      status: "completed",
      output: { subscription_id: "sub-1" },
    },
    { startedAtMs: 2_000, completedAtMs: 3_000 },
  );

  const updated = updateThreadTurn(thread, {
    id: "turn-1",
    items: [
      {
        type: "eventDrivenToolCall",
        id: "item-1",
        tool: "process_exit_subscribe",
        arguments: { session_id: 42 },
        status: "completed",
        output: { subscription_id: "sub-1" },
      },
    ],
    itemsView: "full",
    status: "completed",
    error: null,
    startedAt: 2,
    completedAt: 4,
    durationMs: 2_000,
  });

  assert.deepEqual(updated.turns[0]?.items[0], {
    type: "eventDrivenToolCall",
    id: "item-1",
    tool: "process_exit_subscribe",
    arguments: { session_id: 42 },
    status: "completed",
    output: { subscription_id: "sub-1" },
    startedAtMs: 2_000,
    completedAtMs: 3_000,
  });
});

test("mergeThreadSnapshot hydrates restored usage fields from thread/read", () => {
  const existing = {
    ...makeThread(),
    threadUsage: undefined,
    tokenUsage: undefined,
    contextUsage: undefined,
  };
  const merged = mergeThreadSnapshot(existing, makeThread());

  assert.equal(merged.threadUsage?.tokenUsage?.total.totalTokens, 1200);
  assert.equal(merged.threadUsage?.contextUsage?.budgetUsedPercent, 12);
  assert.equal(merged.tokenUsage?.total.totalTokens, 1200);
  assert.equal(merged.contextUsage?.budgetUsedPercent, 12);
});

test("mergeThreadSnapshot preserves restored usage when a later snapshot sends null fields", () => {
  const existing = makeThread();
  const next = {
    ...makeThread(),
    tokenUsage: null,
    contextUsage: null,
    threadUsage: {
      tokenUsage: null,
      contextUsage: null,
    },
  };

  const merged = mergeThreadSnapshot(existing, next);

  assert.equal(merged.threadUsage?.tokenUsage?.total.totalTokens, 1200);
  assert.equal(merged.threadUsage?.contextUsage?.budgetUsedPercent, 12);
  assert.equal(merged.tokenUsage?.total.totalTokens, 1200);
  assert.equal(merged.contextUsage?.budgetUsedPercent, 12);
});

test("mergeThreadSnapshot preserves same-content items with different ids", () => {
  const thread = {
    ...makeThread(),
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "agentMessage" as const,
            id: "item-1",
            text: "same response",
            phase: null,
            memoryCitation: null,
          },
          {
            type: "agentMessage" as const,
            id: "item-2",
            text: "same response",
            phase: null,
            memoryCitation: null,
          },
        ],
        itemsView: "full" as const,
        status: "completed" as const,
        error: null,
        startedAt: 10,
        completedAt: 12,
        durationMs: 2000,
      },
    ],
  };

  const merged = mergeThreadSnapshot(null, thread);

  assert.deepEqual(merged.turns, thread.turns);
});

test("mergeThreadSnapshot preserves prefix-compatible agent messages in the same snapshot", () => {
  const thread = {
    ...makeThread(),
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "agentMessage" as const,
            id: "item-1",
            text: "same",
            phase: null,
            memoryCitation: null,
          },
          {
            type: "agentMessage" as const,
            id: "item-2",
            text: "same response",
            phase: null,
            memoryCitation: null,
          },
        ],
        itemsView: "full" as const,
        status: "completed" as const,
        error: null,
        startedAt: 10,
        completedAt: 12,
        durationMs: 2000,
      },
    ],
  };

  const merged = mergeThreadSnapshot(null, thread);

  assert.deepEqual(merged.turns, thread.turns);
});

test("upsertThread normalizes the first inserted snapshot", () => {
  const thread = {
    ...makeThread(),
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "agentMessage" as const,
            id: "item-1",
            text: "same response",
            phase: null,
            memoryCitation: null,
          },
          {
            type: "agentMessage" as const,
            id: "item-2",
            text: "same response",
            phase: null,
            memoryCitation: null,
          },
        ],
        itemsView: "full" as const,
        status: "completed" as const,
        error: null,
        startedAt: 10,
        completedAt: 12,
        durationMs: 2000,
      },
    ],
  };

  const threads = upsertThread([], thread);

  assert.deepEqual(threads, [thread]);
});

test("mergeThreadSnapshot preserves live-derived turns with different item ids within next snapshot", () => {
  const liveTurn = {
    id: "live-turn",
    items: [
      {
        type: "agentMessage" as const,
        id: "live-item",
        text: "same response",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "notLoaded" as const,
    status: "completed" as const,
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  };
  const readTurn = {
    ...liveTurn,
    id: "read-turn",
    items: [
      {
        ...liveTurn.items[0],
        id: "read-item",
      },
    ],
    itemsView: "full" as const,
  };

  const merged = mergeThreadSnapshot(null, {
    ...makeThread(),
    turns: [liveTurn, readTurn],
  });

  assert.deepEqual(merged.turns, [liveTurn, readTurn]);
});

test("mergeThreadSnapshot preserves completed full agent turns with matching content in one snapshot", () => {
  const firstTurn = {
    id: "first-turn",
    items: [
      {
        type: "agentMessage" as const,
        id: "first-item",
        text: "same response",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  };
  const secondTurn = {
    ...firstTurn,
    id: "second-turn",
    items: [
      {
        ...firstTurn.items[0],
        id: "second-item",
      },
    ],
  };

  const merged = mergeThreadSnapshot(null, {
    ...makeThread(),
    turns: [firstTurn, secondTurn],
  });

  assert.deepEqual(merged.turns, [firstTurn, secondTurn]);
});

test("mergeThreadSnapshot preserves duplicate dynamic tool calls in one snapshot", () => {
  const item = {
    type: "dynamicToolCall" as const,
    id: "item-1",
    namespace: "functions",
    tool: "read",
    arguments: { path: "/tmp/file" },
    status: "completed" as const,
    contentItems: [{ text: "same output" }],
    success: true,
    durationMs: 10,
  };
  const firstTurn = {
    id: "first-turn",
    items: [item],
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  };
  const secondTurn = {
    ...firstTurn,
    id: "second-turn",
    items: [
      {
        ...item,
        id: "item-2",
      },
    ],
  };

  const merged = mergeThreadSnapshot(null, {
    ...makeThread(),
    turns: [firstTurn, secondTurn],
  });

  assert.deepEqual(merged.turns, [firstTurn, secondTurn]);
});

test("mergeThreadSnapshot preserves restored event-driven tool calls with distinct ids", () => {
  const liveTurn = {
    id: "live-turn",
    items: [
      {
        type: "eventDrivenToolCall" as const,
        id: "live-item",
        tool: "schedule_subscribe",
        arguments: {
          label: "daily digest",
          schedule: "every_day_at 09:00 Asia/Shanghai",
        },
        status: "completed" as const,
        output: { subscription_id: "sub-live" },
      },
    ],
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  };
  const readTurn = {
    ...liveTurn,
    id: "read-turn",
    items: [
      {
        ...liveTurn.items[0],
        id: "read-item",
        output: { subscription_id: "sub-read" },
      },
    ],
  };

  const merged = mergeThreadSnapshot(
    {
      ...makeThread(),
      turns: [liveTurn],
    },
    {
      ...makeThread(),
      turns: [readTurn],
    },
  );

  assert.deepEqual(merged.turns, [readTurn, liveTurn]);
});

test("upsertThread prunes items before the latest compact boundary", () => {
  const thread = upsertThread([], {
    ...makeThread(),
    turns: [
      makeTurn("turn-old", [
        makeUserMessage("old-user", "old request"),
        makeAgentMessage("old-agent", "old answer"),
      ]),
      makeTurn("turn-compact", [
        makeCompactItem("compact-1"),
        makeAgentMessage("after-compact", "continued"),
      ]),
    ],
  })[0]!;

  assert.deepEqual(
    thread.turns.flatMap((turn) => turn.items.map((item) => item.id)),
    ["compact-1", "after-compact"],
  );
});

test("upsertThread preserves active subscriptions across compact pruning", () => {
  const scheduleMonitor: ThreadItem = {
    type: "builtinToolCall",
    id: "active-subscription:sub-schedule",
    tool: "schedule_subscribe",
    arguments: {
      label: "daily digest",
      schedule: { kind: "every_interval", interval_ms: 21_600_000 },
    },
    status: "completed",
    output: {
      subscription_id: "sub-schedule",
      schedule_summary: "every 21600000 ms",
    },
  };
  const thread = upsertThread([], {
    ...makeThread(),
    turns: [
      makeTurn("turn-old", [
        makeUserMessage("old-user", "old request"),
        makeAgentMessage("old-agent", "old answer"),
      ]),
      makeTurn("active-subscriptions", [scheduleMonitor]),
      makeTurn("turn-compact", [
        makeCompactItem("compact-1"),
        makeAgentMessage("after-compact", "continued"),
      ]),
    ],
  })[0]!;
  const analysis = buildThreadAnalysis(thread, 0);

  assert.deepEqual(
    thread.turns.flatMap((turn) => turn.items.map((item) => item.id)),
    ["compact-1", "after-compact"],
  );
  assert.deepEqual(
    thread.activeSubscriptionItems?.map((item) => item.id),
    ["active-subscription:sub-schedule"],
  );
  assert.deepEqual(
    analysis.monitors.sections.find((section) => section.kind === "schedule")
      ?.monitors,
    [
      {
        id: "active-subscription:sub-schedule",
        subscriptionId: "sub-schedule",
        kind: "schedule",
        label: "daily digest",
        detail: "every_interval 6h",
        status: "Listening",
        eventCount: 0,
        latestEvent: null,
      },
    ],
  );
});

test("upsertThread does not revive pre-compact subscriptions after compact cleanup", () => {
  const scheduleMonitor: ThreadItem = {
    type: "builtinToolCall",
    id: "active-subscription:sub-schedule",
    tool: "schedule_subscribe",
    arguments: {
      label: "daily digest",
      schedule: { kind: "every_interval", interval_ms: 21_600_000 },
    },
    status: "completed",
    output: {
      subscription_id: "sub-schedule",
      schedule_summary: "every 21600000 ms",
    },
  };
  const scheduleCleanup: ThreadItem = {
    type: "builtinToolCall",
    id: "active-subscription:sub-schedule:inactive",
    tool: "schedule_unsubscribe",
    arguments: {
      subscription_id: "sub-schedule",
    },
    status: "completed",
    output: {
      subscription_id: "sub-schedule",
      unsubscribed: true,
    },
  };
  const thread = upsertThread([], {
    ...makeThread(),
    turns: [
      makeTurn("active-subscriptions", [scheduleMonitor]),
      makeTurn("turn-compact", [
        makeCompactItem("compact-1"),
        makeAgentMessage("after-compact", "continued"),
      ]),
      makeTurn("active-subscriptions", [scheduleCleanup]),
    ],
  })[0]!;
  const analysis = buildThreadAnalysis(thread, 0);

  assert.deepEqual(
    thread.turns.flatMap((turn) => turn.items.map((item) => item.id)),
    ["compact-1", "after-compact"],
  );
  assert.deepEqual(
    thread.activeSubscriptionItems?.map((item) => item.id),
    ["active-subscription:sub-schedule:inactive"],
  );
  assert.deepEqual(
    analysis.monitors.sections.find((section) => section.kind === "schedule")
      ?.monitors,
    [],
  );
});

test("upsertThread does not revive pre-compact subscriptions after empty compact snapshot", () => {
  const scheduleMonitor: ThreadItem = {
    type: "builtinToolCall",
    id: "active-subscription:sub-schedule",
    tool: "schedule_subscribe",
    arguments: {
      label: "daily digest",
      schedule: { kind: "every_interval", interval_ms: 21_600_000 },
    },
    status: "completed",
    output: {
      subscription_id: "sub-schedule",
      schedule_summary: "every 21600000 ms",
    },
  };
  const thread = upsertThread([], {
    ...makeThread(),
    turns: [
      makeTurn("active-subscriptions", [scheduleMonitor]),
      makeTurn("turn-compact", [
        makeCompactItem("compact-1"),
        makeAgentMessage("after-compact", "continued"),
      ]),
      makeTurn("active-subscriptions", []),
    ],
  })[0]!;
  const analysis = buildThreadAnalysis(thread, 0);

  assert.deepEqual(
    thread.turns.flatMap((turn) => turn.items.map((item) => item.id)),
    ["compact-1", "after-compact"],
  );
  assert.deepEqual(thread.activeSubscriptionItems, []);
  assert.deepEqual(
    analysis.monitors.sections.find((section) => section.kind === "schedule")
      ?.monitors,
    [],
  );
});

test("upsertThread prunes compact-turn items before the compact marker", () => {
  const thread = upsertThread([], {
    ...makeThread(),
    turns: [
      makeTurn("turn-compact", [
        makeAgentMessage("compact-summary", "summarizing old context"),
        makeCompactItem("compact-1"),
        makeAgentMessage("after-compact", "continued"),
      ]),
    ],
  })[0]!;

  assert.deepEqual(
    thread.turns.flatMap((turn) => turn.items.map((item) => item.id)),
    ["compact-1", "after-compact"],
  );
});

test("mergeThreadSnapshot keeps only the latest compact boundary", () => {
  const firstTurn = {
    id: "first-turn",
    items: [
      {
        type: "contextCompaction" as const,
        id: "first-item",
      },
    ],
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  };
  const secondTurn = {
    ...firstTurn,
    id: "second-turn",
    items: [
      {
        ...firstTurn.items[0],
        id: "second-item",
      },
    ],
  };

  const merged = mergeThreadSnapshot(null, {
    ...makeThread(),
    turns: [firstTurn, secondTurn],
  });

  assert.deepEqual(merged.turns, [secondTurn]);
});

test("updateThreadItem prunes live state when a compact notification arrives", () => {
  const thread = updateThreadItem(
    {
      ...makeThread(),
      turns: [
        makeTurn("turn-old", [
          makeUserMessage("old-user", "old request"),
          makeAgentMessage("old-agent", "old answer"),
        ]),
        makeTurn("turn-compact", [
          makeAgentMessage("compact-summary", "summarizing old context"),
        ]),
      ],
    },
    "turn-compact",
    makeCompactItem("compact-1"),
  );

  assert.deepEqual(
    thread.turns.flatMap((turn) => turn.items.map((item) => item.id)),
    ["compact-1"],
  );
});

test("updateThreadItem preserves active subscriptions when a compact notification arrives", () => {
  const scheduleMonitor: ThreadItem = {
    type: "builtinToolCall",
    id: "active-subscription:sub-schedule",
    tool: "schedule_subscribe",
    arguments: {
      label: "daily digest",
      schedule: { kind: "every_interval", interval_ms: 21_600_000 },
    },
    status: "completed",
    output: {
      subscription_id: "sub-schedule",
      schedule_summary: "every 21600000 ms",
    },
  };
  const thread = updateThreadItem(
    {
      ...makeThread(),
      turns: [
        makeTurn("turn-old", [
          makeUserMessage("old-user", "old request"),
          makeAgentMessage("old-agent", "old answer"),
        ]),
        makeTurn("active-subscriptions", [scheduleMonitor]),
        makeTurn("turn-compact", [
          makeAgentMessage("compact-summary", "summarizing old context"),
        ]),
      ],
    },
    "turn-compact",
    makeCompactItem("compact-1"),
  );
  const analysis = buildThreadAnalysis(thread, 0);

  assert.deepEqual(
    thread.turns.flatMap((turn) => turn.items.map((item) => item.id)),
    ["compact-1"],
  );
  assert.deepEqual(
    thread.activeSubscriptionItems?.map((item) => item.id),
    ["active-subscription:sub-schedule"],
  );
  assert.deepEqual(
    analysis.monitors.sections.find((section) => section.kind === "schedule")
      ?.monitors,
    [
      {
        id: "active-subscription:sub-schedule",
        subscriptionId: "sub-schedule",
        kind: "schedule",
        label: "daily digest",
        detail: "every_interval 6h",
        status: "Listening",
        eventCount: 0,
        latestEvent: null,
      },
    ],
  );
});

test("late same-turn item notifications cannot re-add compact-pruned items", () => {
  const compactedThread = updateThreadItem(
    {
      ...makeThread(),
      turns: [
        makeTurn("turn-compact", [
          makeAgentMessage("compact-summary", "summarizing old context"),
        ]),
      ],
    },
    "turn-compact",
    makeCompactItem("compact-1"),
  );

  const lateItemThread = updateThreadItem(
    compactedThread,
    "turn-compact",
    makeAgentMessage("compact-summary", "late summary completion"),
  );
  const lateDeltaThread = appendAgentDelta(
    compactedThread,
    "turn-compact",
    "compact-summary",
    " late delta",
  );

  assert.deepEqual(
    lateItemThread.turns.flatMap((turn) => turn.items.map((item) => item.id)),
    ["compact-1"],
  );
  assert.deepEqual(
    lateDeltaThread.turns.flatMap((turn) => turn.items.map((item) => item.id)),
    ["compact-1"],
  );
});

test("same-turn pruned items with the compact boundary timestamp do not reappear", () => {
  const compactedThread = updateThreadItem(
    {
      ...makeThread(),
      turns: [
        makeTurn("turn-compact", [
          makeAgentMessage("compact-summary", "summarizing old context"),
        ]),
      ],
    },
    "turn-compact",
    {
      ...makeCompactItem("compact-1"),
      completedAtMs: 12_000,
    },
  );

  const equalBoundaryThread = updateThreadItem(
    compactedThread,
    "turn-compact",
    makeAgentMessage("compact-summary", "late summary completion"),
    { completedAtMs: 12_000 },
  );
  const afterBoundaryThread = updateThreadItem(
    compactedThread,
    "turn-compact",
    makeAgentMessage("after-compact", "new item after compact"),
    { startedAtMs: 12_001 },
  );

  assert.deepEqual(
    equalBoundaryThread.turns.flatMap((turn) =>
      turn.items.map((item) => item.id),
    ),
    ["compact-1"],
  );
  assert.deepEqual(
    afterBoundaryThread.turns.flatMap((turn) =>
      turn.items.map((item) => item.id),
    ),
    ["compact-1", "after-compact"],
  );
});

test("compact-pruned threads do not create synthetic turns for old missing notifications", () => {
  const compactedThread = upsertThread([], {
    ...makeThread(),
    turns: [
      makeTurn("turn-old", [makeAgentMessage("old-agent", "old answer")]),
      makeTurn("turn-compact", [
        makeCompactItem("compact-1"),
        makeAgentMessage("after-compact", "continued"),
      ]),
    ],
  })[0]!;

  const lateOldTurnThread = updateThreadItem(
    compactedThread,
    "turn-old",
    makeAgentMessage("old-agent", "late old completion"),
  );

  assert.deepEqual(
    lateOldTurnThread.turns.flatMap((turn) =>
      turn.items.map((item) => item.id),
    ),
    ["compact-1", "after-compact"],
  );
});

test("late old turn lifecycle cannot reopen compact-pruned turns", () => {
  const compactedThread = upsertThread([], {
    ...makeThread(),
    turns: [
      makeTurn("turn-old", [makeAgentMessage("old-agent", "old answer")]),
      makeTurn("turn-compact", [makeCompactItem("compact-1")]),
    ],
  })[0]!;

  const lifecycleThread = updateThreadTurnLifecycle(compactedThread, {
    ...makeTurn("turn-old", []),
    status: "running",
    completedAt: null,
    durationMs: null,
  });
  const lateOldItemThread = updateThreadItem(
    lifecycleThread,
    "turn-old",
    makeAgentMessage("old-agent", "late old completion"),
  );

  assert.deepEqual(
    lateOldItemThread.turns.flatMap((turn) =>
      turn.items.map((item) => item.id),
    ),
    ["compact-1"],
  );
});

test("compacted threads can still create later turns with timestamps after the compact boundary", () => {
  const compactedThread = upsertThread([], {
    ...makeThread(),
    turns: [
      makeTurn("turn-old", [makeAgentMessage("old-agent", "old answer")]),
      makeTurn("turn-compact", [makeCompactItem("compact-1")]),
    ],
  })[0]!;

  const lifecycleThread = updateThreadTurnLifecycle(compactedThread, {
    ...makeTurn("turn-new", []),
    status: "running",
    startedAt: 20,
    completedAt: null,
    durationMs: null,
  });
  const withItemThread = updateThreadItem(
    lifecycleThread,
    "turn-new",
    makeAgentMessage("new-agent", "new answer"),
    { startedAtMs: 20_000 },
  );

  assert.deepEqual(
    withItemThread.turns.flatMap((turn) => turn.items.map((item) => item.id)),
    ["compact-1", "new-agent"],
  );
});

test("late old child completion does not attach to a later active turn after compact", () => {
  const compactedThread = upsertThread([], {
    ...makeThread(),
    turns: [
      makeTurn("turn-old", [makeAgentMessage("old-agent", "old answer")]),
      makeTurn("turn-compact", [makeCompactItem("compact-1")]),
    ],
  })[0]!;
  const activeThread = updateThreadTurnLifecycle(compactedThread, {
    ...makeTurn("turn-new", []),
    status: "running",
    startedAt: 20,
    completedAt: null,
    durationMs: null,
  });

  const lateCompletionThread = updateThreadItem(
    activeThread,
    "turn-old",
    makeCollabStatusItem("old-child-completion"),
  );

  assert.deepEqual(
    lateCompletionThread.turns.flatMap((turn) =>
      turn.items.map((item) => item.id),
    ),
    ["compact-1"],
  );
});

test("compact-pruned threads still update items retained after the compact marker", () => {
  const thread = updateThreadItem(
    {
      ...makeThread(),
      turns: [
        makeTurn("turn-compact", [
          makeCompactItem("compact-1"),
          makeAgentMessage("after-compact", "continued"),
        ]),
      ],
    },
    "turn-compact",
    makeAgentMessage("after-compact", "continued with more detail"),
  );
  const afterCompact = thread.turns[0]?.items[1];

  assert.equal(afterCompact?.type, "agentMessage");
  assert.equal(
    afterCompact?.type === "agentMessage" ? afterCompact.text : null,
    "continued with more detail",
  );
});

test("conversation and right-panel analysis only scan the pruned active segment", () => {
  const archivedItems = Array.from({ length: 1_000 }, (_, index) =>
    makeUserMessage(`old-user-${index}`, `old request ${index}`),
  );
  const thread = upsertThread([], {
    ...makeThread(),
    turns: [
      makeTurn("turn-old", [
        ...archivedItems,
        {
          type: "fileChange" as const,
          id: "old-file",
          changes: [{ path: "/tmp/archived.ts", kind: "modified" }],
          status: "completed",
        },
      ]),
      makeTurn("turn-compact", [makeCompactItem("compact-1")]),
      makeTurn("turn-active", [
        makeUserMessage("active-user", "active request"),
        {
          type: "fileChange" as const,
          id: "active-file",
          changes: [{ path: "/tmp/active.ts", kind: "modified" }],
          status: "completed",
        },
      ]),
    ],
  })[0]!;

  const conversation = buildConversationState(thread);
  const analysis = buildThreadAnalysis(thread, 0);

  assert.equal(conversation.flatItems.length, 3);
  assert.deepEqual(
    conversation.flatItems.map((item) => item.id),
    ["compact-1", "active-user", "active-file"],
  );
  assert.deepEqual(
    analysis.changedFiles.map((file) => file.path),
    ["/tmp/active.ts"],
  );
});

test("mergeThreadSnapshot preserves an in-flight turn missing from a stale snapshot", () => {
  const existing = {
    ...makeThread(),
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "collabAgentMessage" as const,
            id: "item-1",
            operation: "sendMessage",
            senderThreadId: "thread-2",
            senderPath: "/root/worker",
            recipientThreadId: "thread-1",
            recipientPath: "/root",
            otherRecipientPaths: [],
            content: "new backend message",
            triggerTurn: true,
          },
        ],
        itemsView: "full" as const,
        status: "running" as const,
        error: null,
        startedAt: 1,
        completedAt: null,
        durationMs: null,
      },
    ],
  };

  const merged = mergeThreadSnapshot(existing, {
    ...makeThread(),
    turns: [],
  });

  assert.deepEqual(merged.turns, existing.turns);
});

test("mergeThreadSnapshot preserves init context when a started snapshot omits turns", () => {
  const initContextTurn: Turn = {
    id: "turn-init",
    items: [
      {
        type: "injectedContext",
        id: "ctx-1",
        title: "Init Context",
        preview: "Developer",
        sections: [
          {
            label: "Developer",
            text: "Agent type file body: always inspect the active task.",
          },
        ],
      },
    ],
    itemsView: "full",
    status: "completed",
    error: null,
    startedAt: 1,
    completedAt: 2,
    durationMs: 1000,
  };
  const existing = {
    ...makeThread(),
    turns: [initContextTurn],
  };

  const merged = mergeThreadSnapshot(existing, {
    ...makeThread(),
    turns: [],
  });

  assert.deepEqual(buildConversationEntries(merged).map((entry) => entry.id), [
    "ctx-1",
  ]);
});

test("mergeThreadSnapshot preserves same-content in-flight items with different ids", () => {
  const restoredTurn = {
    id: "restored-turn",
    items: [
      {
        type: "collabAgentMessage" as const,
        id: "restored-item",
        operation: "sendMessage",
        senderThreadId: "thread-2",
        senderPath: "/root/worker",
        recipientThreadId: "thread-1",
        recipientPath: "/root",
        otherRecipientPaths: [],
        content: "same backend message",
        triggerTurn: true,
      },
    ],
    itemsView: "full" as const,
    status: "running" as const,
    error: null,
    startedAt: 10,
    completedAt: null,
    durationMs: null,
  };
  const readTurn = {
    ...restoredTurn,
    id: "read-turn",
    items: [
      {
        ...restoredTurn.items[0],
        id: "read-item",
      },
    ],
    status: "completed" as const,
    completedAt: 12,
    durationMs: 2000,
  };

  const merged = mergeThreadSnapshot(
    {
      ...makeThread(),
      turns: [restoredTurn],
    },
    {
      ...makeThread(),
      turns: [readTurn],
    },
  );

  assert.deepEqual(merged.turns, [readTurn, restoredTurn]);
});

test("mergeThreadSnapshot preserves same-content completed live agent turns with different ids", () => {
  const liveDelta = appendAgentDelta(
    makeThread(),
    "live-turn",
    "live-item",
    "same response",
  );
  const completedLive = updateThreadTurn(liveDelta, {
    id: "live-turn",
    items: [],
    itemsView: "notLoaded",
    status: "completed",
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  });
  const readTurn = {
    id: "read-turn",
    items: [
      {
        type: "agentMessage" as const,
        id: "read-item",
        text: "same response",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  };

  const merged = mergeThreadSnapshot(completedLive, {
    ...makeThread(),
    turns: [readTurn],
  });

  assert.deepEqual(merged.turns, [readTurn, completedLive.turns[0]]);
});

test("mergeThreadSnapshot preserves same-content live child completion with different ids", () => {
  const liveItem: ThreadItem = {
    type: "collabAgentStatusUpdate",
    id: "live-completion",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    lifecycleStatus: collabAgentLifecycleState("completed"),
  };
  const liveThread = updateThreadItem(makeThread(), "live-turn", liveItem, {
    completedAtMs: 3_000,
    syntheticTurnStatus: "completed",
  });
  const readTurn = {
    id: "read-turn",
    items: [
      {
        ...liveItem,
        id: "read-completion",
      },
    ],
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 3,
    completedAt: 3,
    durationMs: null,
  };

  const merged = mergeThreadSnapshot(liveThread, {
    ...makeThread(),
    turns: [readTurn],
  });

  assert.deepEqual(merged.turns, [readTurn, liveThread.turns[0]]);
});

test("mergeThreadSnapshot preserves completed full agent turns with matching content", () => {
  const liveTurn = {
    id: "live-turn",
    items: [
      {
        type: "agentMessage" as const,
        id: "live-item",
        text: "same response",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  };
  const readTurn = {
    ...liveTurn,
    id: "read-turn",
    items: [
      {
        ...liveTurn.items[0],
        id: "read-item",
      },
    ],
  };

  const merged = mergeThreadSnapshot(
    {
      ...makeThread(),
      turns: [liveTurn],
    },
    {
      ...makeThread(),
      turns: [readTurn],
    },
  );

  assert.deepEqual(merged.turns, [readTurn, liveTurn]);
});

test("mergeThreadSnapshot preserves every same-content item with a distinct id", () => {
  const restoredTurn = {
    id: "restored-turn",
    items: [
      {
        type: "agentMessage" as const,
        id: "restored-item-1",
        text: "same response",
        phase: null,
        memoryCitation: null,
      },
      {
        type: "agentMessage" as const,
        id: "restored-item-2",
        text: "same response",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "full" as const,
    status: "running" as const,
    error: null,
    startedAt: 10,
    completedAt: null,
    durationMs: null,
  };
  const readTurn = {
    ...restoredTurn,
    id: "read-turn",
    items: [
      {
        ...restoredTurn.items[0],
        id: "read-item",
      },
    ],
    status: "completed" as const,
    completedAt: 12,
    durationMs: 2000,
  };

  const merged = mergeThreadSnapshot(
    {
      ...makeThread(),
      turns: [restoredTurn],
    },
    {
      ...makeThread(),
      turns: [readTurn],
    },
  );

  assert.deepEqual(merged.turns, [
    readTurn,
    restoredTurn,
  ]);
});

test("mergeThreadSnapshot preserves distinct in-flight items with matching content", () => {
  const readTurn = {
    id: "read-turn",
    items: [
      {
        type: "collabAgentMessage" as const,
        id: "read-item",
        operation: "sendMessage",
        senderThreadId: "thread-2",
        senderPath: "/root/worker",
        recipientThreadId: "thread-1",
        recipientPath: "/root",
        otherRecipientPaths: [],
        content: "same backend message",
        triggerTurn: true,
      },
    ],
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  };
  const liveTurn = {
    ...readTurn,
    id: "live-turn",
    items: [
      {
        ...readTurn.items[0],
        id: "live-item",
      },
    ],
    status: "running" as const,
    startedAt: 20,
    completedAt: null,
    durationMs: null,
  };

  const merged = mergeThreadSnapshot(
    {
      ...makeThread(),
      turns: [liveTurn],
    },
    {
      ...makeThread(),
      turns: [readTurn],
    },
  );

  assert.deepEqual(merged.turns, [readTurn, liveTurn]);
});

test("updateThreadTurn preserves in-flight items when a completed turn arrives with new ids", () => {
  const runningTurn = {
    id: "running-turn",
    items: [
      {
        type: "agentMessage" as const,
        id: "running-item",
        text: "same response",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "full" as const,
    status: "running" as const,
    error: null,
    startedAt: 10,
    completedAt: null,
    durationMs: null,
  };
  const completedTurn = {
    ...runningTurn,
    id: "completed-turn",
    items: [
      {
        ...runningTurn.items[0],
        id: "completed-item",
      },
    ],
    status: "completed" as const,
    completedAt: 12,
    durationMs: 2000,
  };

  const updated = updateThreadTurn(
    {
      ...makeThread(),
      turns: [runningTurn],
    },
    completedTurn,
  );

  assert.deepEqual(updated.turns, [runningTurn, completedTurn]);
});

test("updateThreadTurn preserves placeholder delta turns when a completed turn arrives with new ids", () => {
  const placeholderThread = appendAgentDelta(
    makeThread(),
    "placeholder-turn",
    "placeholder-item",
    "same response",
  );
  const completedTurn = {
    id: "completed-turn",
    items: [
      {
        type: "agentMessage" as const,
        id: "completed-item",
        text: "same response",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  };

  const updated = updateThreadTurn(placeholderThread, completedTurn);

  assert.deepEqual(updated.turns, [placeholderThread.turns[0], completedTurn]);
});

test("updateThreadTurn preserves distinct running items when appending a same-content turn from another time", () => {
  const completedTurn = {
    id: "completed-turn",
    items: [
      {
        type: "agentMessage" as const,
        id: "completed-item",
        text: "same response",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  };
  const runningTurn = {
    ...completedTurn,
    id: "running-turn",
    items: [
      {
        ...completedTurn.items[0],
        id: "running-item",
      },
    ],
    status: "running" as const,
    startedAt: 20,
    completedAt: null,
    durationMs: null,
  };

  const updated = updateThreadTurn(
    {
      ...makeThread(),
      turns: [runningTurn],
    },
    completedTurn,
  );

  assert.deepEqual(updated.turns, [runningTurn, completedTurn]);
});

test("appendAgentDelta preserves later same-content assistant turns", () => {
  const completedTurn = {
    id: "completed-turn",
    items: [
      {
        type: "agentMessage" as const,
        id: "completed-item",
        text: "same response",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "full" as const,
    status: "completed" as const,
    error: null,
    startedAt: 10,
    completedAt: 12,
    durationMs: 2000,
  };
  const thread = {
    ...makeThread(),
    turns: [completedTurn],
  };

  const updated = appendAgentDelta(
    thread,
    "later-turn",
    "later-item",
    "same response",
  );

  assert.deepEqual(updated.turns, [
    completedTurn,
    {
      id: "later-turn",
      items: [
        {
          type: "agentMessage",
          id: "later-item",
          text: "same response",
          phase: null,
          memoryCitation: null,
        },
      ],
      itemsView: "full",
      status: "running",
      error: null,
      startedAt: null,
      completedAt: null,
      durationMs: null,
    },
  ]);
});

test("mergeThreadSnapshot keeps the more complete in-flight agent message text", () => {
  const existingTurn = {
    id: "turn-1",
    items: [
      {
        type: "agentMessage" as const,
        id: "item-1",
        text: "hello world",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "full" as const,
    status: "running" as const,
    error: null,
    startedAt: 1,
    completedAt: null,
    durationMs: null,
  };
  const readTurn = {
    ...existingTurn,
    items: [
      {
        ...existingTurn.items[0],
        text: "hello",
      },
    ],
  };
  const existing = {
    ...makeThread(),
    turns: [existingTurn],
  };

  const merged = mergeThreadSnapshot(existing, {
    ...makeThread(),
    turns: [readTurn],
  });

  assert.equal(merged.turns.length, 1);
  assert.equal(merged.turns[0]?.id, "turn-1");
  assert.deepEqual(merged.turns[0]?.items, existingTurn.items);
});

test("updateThreadItem preserves same-turn event-driven items with different ids", () => {
  const thread = {
    ...makeThread(),
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "eventDrivenTool" as const,
            id: "snapshot-item",
            tool: "fs_subscribe",
            title: "File watch triggered",
            text: "build.log changed",
            completedAtMs: 100,
          },
        ],
        itemsView: "full" as const,
        status: "running" as const,
        error: null,
        startedAt: 1,
        completedAt: null,
        durationMs: null,
      },
    ],
  };

  const updated = updateThreadItem(thread, "turn-1", {
    type: "eventDrivenTool",
    id: "live-item",
    tool: "fs_subscribe",
    title: "File watch triggered",
    text: "build.log changed",
    completedAtMs: 200,
  });

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "eventDrivenTool",
      id: "snapshot-item",
      tool: "fs_subscribe",
      title: "File watch triggered",
      text: "build.log changed",
      completedAtMs: 100,
    },
    {
      type: "eventDrivenTool",
      id: "live-item",
      tool: "fs_subscribe",
      title: "File watch triggered",
      text: "build.log changed",
      completedAtMs: 200,
    },
  ]);
});

test("updateThreadItem preserves same-turn agent delta placeholder with completed item", () => {
  const thread = appendAgentDelta(
    makeThread(),
    "turn-1",
    "delta-item",
    "same response",
  );

  const updated = updateThreadItem(
    thread,
    "turn-1",
    {
      type: "agentMessage",
      id: "completed-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
    },
    { completedAtMs: 2_000 },
  );

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "agentMessage",
      id: "delta-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
    },
    {
      type: "agentMessage",
      id: "completed-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
      completedAtMs: 2_000,
    },
  ]);
});

test("updateThreadItem preserves same-turn agent delta text when completed item has a new id", () => {
  const thread = appendAgentDelta(makeThread(), "turn-1", "delta-item", "same");

  const updated = updateThreadItem(
    thread,
    "turn-1",
    {
      type: "agentMessage",
      id: "completed-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
    },
    { completedAtMs: 2_000 },
  );

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "agentMessage",
      id: "delta-item",
      text: "same",
      phase: null,
      memoryCitation: null,
    },
    {
      type: "agentMessage",
      id: "completed-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
      completedAtMs: 2_000,
    },
  ]);
});

test("updateThreadItem does not overwrite an existing agent message with a prefix-compatible item", () => {
  const thread = updateThreadItem(makeThread(), "turn-1", {
    type: "agentMessage",
    id: "first-item",
    text: "same",
    phase: null,
    memoryCitation: null,
  });

  const updated = updateThreadItem(
    thread,
    "turn-1",
    {
      type: "agentMessage",
      id: "second-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
    },
    { completedAtMs: 3_000 },
  );

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "agentMessage",
      id: "first-item",
      text: "same",
      phase: null,
      memoryCitation: null,
    },
    {
      type: "agentMessage",
      id: "second-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
      completedAtMs: 3_000,
    },
  ]);
});

test("updateThreadItem preserves same-text agent messages unless the existing item is a delta placeholder", () => {
  const thread = updateThreadItem(makeThread(), "turn-1", {
    type: "agentMessage",
    id: "first-item",
    text: "same response",
    phase: null,
    memoryCitation: null,
  });

  const updated = updateThreadItem(thread, "turn-1", {
    type: "agentMessage",
    id: "second-item",
    text: "same response",
    phase: null,
    memoryCitation: null,
  });

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "agentMessage",
      id: "first-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
    },
    {
      type: "agentMessage",
      id: "second-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
    },
  ]);
});

test("updateThreadItem preserves completed agent messages and matching delta placeholders", () => {
  const threadWithFirst = appendAgentDelta(
    makeThread(),
    "turn-1",
    "first-delta-item",
    "same response",
  );
  const thread = appendAgentDelta(
    threadWithFirst,
    "turn-1",
    "second-delta-item",
    "same",
  );

  const updated = updateThreadItem(
    thread,
    "turn-1",
    {
      type: "agentMessage",
      id: "completed-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
    },
    { completedAtMs: 3_000 },
  );

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "agentMessage",
      id: "first-delta-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
    },
    {
      type: "agentMessage",
      id: "second-delta-item",
      text: "same",
      phase: null,
      memoryCitation: null,
    },
    {
      type: "agentMessage",
      id: "completed-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
      completedAtMs: 3_000,
    },
  ]);
});

test("updateThreadItem preserves distinct completed agent messages with matching text", () => {
  const thread = updateThreadItem(
    makeThread(),
    "turn-1",
    {
      type: "agentMessage",
      id: "first-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
    },
    { completedAtMs: 2_000 },
  );

  const updated = updateThreadItem(
    thread,
    "turn-1",
    {
      type: "agentMessage",
      id: "second-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
    },
    { completedAtMs: 3_000 },
  );

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "agentMessage",
      id: "first-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
      completedAtMs: 2_000,
    },
    {
      type: "agentMessage",
      id: "second-item",
      text: "same response",
      phase: null,
      memoryCitation: null,
      completedAtMs: 3_000,
    },
  ]);
});

test("updateThreadItem preserves same-turn collab agent messages with different ids", () => {
  const thread = updateThreadItem(makeThread(), "turn-1", {
    type: "collabAgentMessage",
    id: "live-item",
    operation: "sendMessage",
    senderThreadId: "thread-2",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    otherRecipientPaths: [],
    content: "same backend message",
    triggerTurn: true,
  });

  const updated = updateThreadItem(thread, "turn-1", {
    type: "collabAgentMessage",
    id: "completed-item",
    operation: "sendMessage",
    senderThreadId: "thread-2",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    otherRecipientPaths: [],
    content: "same backend message",
    triggerTurn: true,
  });

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "collabAgentMessage",
      id: "live-item",
      operation: "sendMessage",
      senderThreadId: "thread-2",
      senderPath: "/root/worker",
      recipientThreadId: "thread-1",
      recipientPath: "/root",
      otherRecipientPaths: [],
      content: "same backend message",
      triggerTurn: true,
    },
    {
      type: "collabAgentMessage",
      id: "completed-item",
      operation: "sendMessage",
      senderThreadId: "thread-2",
      senderPath: "/root/worker",
      recipientThreadId: "thread-1",
      recipientPath: "/root",
      otherRecipientPaths: [],
      content: "same backend message",
      triggerTurn: true,
    },
  ]);
});

test("updateThreadItem keeps only the latest separate context compaction marker", () => {
  const thread = updateThreadItem(makeThread(), "turn-1", {
    type: "contextCompaction",
    id: "first-item",
  });

  const updated = updateThreadItem(thread, "turn-1", {
    type: "contextCompaction",
    id: "second-item",
  });

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "contextCompaction",
      id: "second-item",
    },
  ]);
});

test("updateThreadItem preserves repeated dynamic tool calls with matching content", () => {
  const thread = updateThreadItem(makeThread(), "turn-1", {
    type: "dynamicToolCall",
    id: "first-item",
    namespace: "functions",
    tool: "read",
    arguments: { path: "/tmp/file" },
    status: "completed",
    contentItems: [{ text: "same output" }],
    success: true,
    durationMs: 10,
  });

  const updated = updateThreadItem(thread, "turn-1", {
    type: "dynamicToolCall",
    id: "second-item",
    namespace: "functions",
    tool: "read",
    arguments: { path: "/tmp/file" },
    status: "completed",
    contentItems: [{ text: "same output" }],
    success: true,
    durationMs: 10,
  });

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "dynamicToolCall",
      id: "first-item",
      namespace: "functions",
      tool: "read",
      arguments: { path: "/tmp/file" },
      status: "completed",
      contentItems: [{ text: "same output" }],
      success: true,
      durationMs: 10,
    },
    {
      type: "dynamicToolCall",
      id: "second-item",
      namespace: "functions",
      tool: "read",
      arguments: { path: "/tmp/file" },
      status: "completed",
      contentItems: [{ text: "same output" }],
      success: true,
      durationMs: 10,
    },
  ]);
});

test("pending thread updates replay when the thread snapshot arrives", () => {
  const pendingUpdates = new Map<string, Array<(thread: Thread) => Thread>>();
  queuePendingThreadUpdate(pendingUpdates, "thread-1", (thread) =>
    updateThreadItem(thread, "turn-1", {
      type: "collabAgentStatusUpdate",
      id: "subagent-complete",
      senderThreadId: "thread-child",
      senderPath: "/root/worker",
      recipientThreadId: "thread-1",
      recipientPath: "/root",
      lifecycleStatus: collabAgentLifecycleState("completed", "done", null),
    }),
  );

  const updated = applyPendingThreadUpdates(makeThread(), pendingUpdates);

  assert.equal(pendingUpdates.size, 0);
  assert.deepEqual(updated.turns, [
    {
      id: "turn-1",
      items: [
        {
          type: "collabAgentStatusUpdate",
          id: "subagent-complete",
          senderThreadId: "thread-child",
          senderPath: "/root/worker",
          recipientThreadId: "thread-1",
          recipientPath: "/root",
          lifecycleStatus: collabAgentLifecycleState("completed", "done", null),
        },
      ],
      itemsView: "full",
      status: "running",
      error: null,
      startedAt: null,
      completedAt: null,
      durationMs: null,
    },
  ]);
});

test("uninitialized live command notifications replay after the thread snapshot arrives", () => {
  const pendingUpdates = new Map<string, Array<(thread: Thread) => Thread>>();
  let threads = [makeThread()];
  const initializedThreadIds = new Set<string>();
  const commandStart: ThreadItem = {
    type: "commandExecution",
    id: "cmd-1",
    command: "rtk printf hello",
    cwd: "/tmp",
    status: "running",
    initialWaitMs: 1000,
    notifyOn: "output",
    aggregatedOutput: null,
    exitCode: null,
    durationMs: null,
  };
  const commandEnd: ThreadItem = {
    ...commandStart,
    status: "completed",
    aggregatedOutput: "hello",
    exitCode: 0,
    durationMs: 10,
  };
  const expectedCommandEnd: ThreadItem = {
    ...commandEnd,
    completedAtMs: 2_000,
  };

  threads = applyOrQueueInitializedThreadUpdate(
    threads,
    initializedThreadIds,
    pendingUpdates,
    "thread-1",
    (thread) => updateThreadItem(thread, "turn-command", commandStart),
  );
  threads = applyOrQueueInitializedThreadUpdate(
    threads,
    initializedThreadIds,
    pendingUpdates,
    "thread-1",
    (thread) =>
      appendCommandExecutionDelta(thread, "turn-command", "cmd-1", "hello"),
  );
  threads = applyOrQueueInitializedThreadUpdate(
    threads,
    initializedThreadIds,
    pendingUpdates,
    "thread-1",
    (thread) =>
      updateThreadItem(thread, "turn-command", commandEnd, {
        completedAtMs: 2_000,
      }),
  );

  assert.equal(threads[0]?.turns.length, 0);
  assert.equal(pendingUpdates.get("thread-1")?.length, 3);

  const updated = applyPendingThreadUpdates(makeThread(), pendingUpdates);

  assert.equal(pendingUpdates.size, 0);
  assert.deepEqual(updated.turns[0]?.items, [expectedCommandEnd]);
  assert.deepEqual(
    buildConversationEntries(updated).map((entry) => ({
      id: entry.id,
      role: entry.role,
      text: entry.text,
    })),
    [
      {
        id: "cmd-1",
        role: "system",
        text: "/tmp • exit 0",
      },
    ],
  );
});

test("initialized live thread updates apply immediately without pending queue", () => {
  const pendingUpdates = new Map<string, Array<(thread: Thread) => Thread>>();
  const commandStart: ThreadItem = {
    type: "commandExecution",
    id: "cmd-1",
    command: "rtk pwd",
    cwd: "/tmp",
    status: "running",
    aggregatedOutput: null,
    exitCode: null,
    durationMs: null,
  };

  const threads = applyOrQueueInitializedThreadUpdate(
    [makeThread()],
    new Set(["thread-1"]),
    pendingUpdates,
    "thread-1",
    (thread) => updateThreadItem(thread, "turn-command", commandStart),
  );

  assert.equal(pendingUpdates.size, 0);
  assert.deepEqual(threads[0]?.turns[0]?.items, [commandStart]);
});

test("live command start marks stale completed snapshot active for monitors", () => {
  const commandStart: ThreadItem = {
    type: "commandExecution",
    id: "cmd-1",
    command: "rtk tail -f /tmp/build.log",
    cwd: "/repo",
    status: "running",
    initialWaitMs: 1000,
    notifyOn: "output",
    aggregatedOutput: null,
    exitCode: null,
    durationMs: null,
  };
  const updated = updateThreadItem(
    markThreadCommandExecutionRunning(makeThread()),
    "turn-command",
    commandStart,
    { startedAtMs: 1_000 },
  );
  const analysis = buildThreadAnalysis(updated, 0);

  assert.deepEqual(updated.lifecycleStatus, {
    type: "active",
    activeFlags: ["running"],
  });
  assert.deepEqual(analysis.monitors.sections[0]?.monitors, [
    {
      id: "cmd-1",
      subscriptionId: "cmd-1",
      kind: "command",
      label: "rtk tail -f /tmp/build.log",
      detail: "/repo",
      status: "Running",
      eventCount: 0,
      latestEvent: null,
    },
  ]);
});

test("live command delta before start creates one monitor and merges start", () => {
  const commandStart: ThreadItem = {
    type: "commandExecution",
    id: "cmd-1",
    command: "rtk printf hello",
    cwd: "/tmp",
    status: "running",
    initialWaitMs: 1000,
    notifyOn: "output",
    aggregatedOutput: null,
    exitCode: null,
    durationMs: null,
  };

  const withDelta = appendCommandExecutionDelta(
    markThreadCommandExecutionRunning(makeThread()),
    "turn-command",
    "cmd-1",
    "hello\n",
  );
  const withStart = updateThreadItem(withDelta, "turn-command", commandStart, {
    startedAtMs: 1_000,
  });
  const commandItems =
    withStart.turns[0]?.items.filter(
      (item) => item.type === "commandExecution",
    ) ?? [];
  const analysis = buildThreadAnalysis(withStart, 0);

  assert.equal(commandItems.length, 1);
  assert.deepEqual(analysis.monitors.sections[0]?.monitors, [
    {
      id: "cmd-1",
      subscriptionId: "cmd-1",
      kind: "command",
      label: "rtk printf hello",
      detail: "/tmp",
      status: "Running",
      eventCount: 1,
      latestEvent: "hello",
    },
  ]);
});

test("uninitialized live schedule notifications update monitors after the snapshot arrives", () => {
  const pendingUpdates = new Map<string, Array<(thread: Thread) => Thread>>();
  let threads = [makeThread()];
  const initializedThreadIds = new Set<string>();
  const scheduleSubscribe: ThreadItem = {
    type: "builtinToolCall",
    id: "schedule-1",
    tool: "schedule_subscribe",
    arguments: {
      label: "daily digest",
      schedule: { kind: "every_interval", interval_ms: 21_600_000 },
    },
    status: "completed",
    output: {
      subscription_id: "sub-schedule",
      schedule_summary: "every 21600000 ms",
    },
  };
  const scheduleUnsubscribe: ThreadItem = {
    type: "builtinToolCall",
    id: "schedule-unsubscribe-1",
    tool: "schedule_unsubscribe",
    arguments: {
      subscription_id: "sub-schedule",
    },
    status: "completed",
    output: {
      unsubscribed: true,
      subscription_id: "sub-schedule",
    },
  };

  threads = applyOrQueueInitializedThreadUpdate(
    threads,
    initializedThreadIds,
    pendingUpdates,
    "thread-1",
    (thread) => updateThreadItem(thread, "turn-schedule", scheduleSubscribe),
  );

  assert.equal(threads[0]?.turns.length, 0);
  let updated = applyPendingThreadUpdates(makeThread(), pendingUpdates);
  let analysis = buildThreadAnalysis(updated, 0);

  assert.equal(pendingUpdates.size, 0);
  assert.deepEqual(
    analysis.monitors.sections.find((section) => section.kind === "schedule")
      ?.monitors,
    [
      {
        id: "schedule-1",
        subscriptionId: "sub-schedule",
        kind: "schedule",
        label: "daily digest",
        detail: "every_interval 6h",
        status: "Listening",
        eventCount: 0,
        latestEvent: null,
      },
    ],
  );

  threads = [updated];
  threads = applyOrQueueInitializedThreadUpdate(
    threads,
    initializedThreadIds,
    pendingUpdates,
    "thread-1",
    (thread) =>
      updateThreadItem(thread, "turn-schedule-unsubscribe", scheduleUnsubscribe),
  );
  updated = applyPendingThreadUpdates(updated, pendingUpdates);
  analysis = buildThreadAnalysis(updated, 0);

  assert.equal(threads[0]?.turns.length, 1);
  assert.equal(pendingUpdates.size, 0);
  assert.deepEqual(
    analysis.monitors.sections.find((section) => section.kind === "schedule")
      ?.monitors,
    [],
  );
});

test("pending live schedule subscribe does not duplicate restored active subscription monitor", () => {
  const pendingUpdates = new Map<string, Array<(thread: Thread) => Thread>>();
  const restoredSchedule: ThreadItem = {
    type: "builtinToolCall",
    id: "schedule-restored",
    tool: "schedule_subscribe",
    arguments: {
      label: "daily digest",
      schedule: { kind: "every_interval", interval_ms: 21_600_000 },
    },
    status: "completed",
    output: {
      subscription_id: "sub-schedule",
      schedule_summary: "every 21600000 ms",
    },
  };
  const liveSchedule: ThreadItem = {
    ...restoredSchedule,
    id: "schedule-live",
  };
  const snapshot: Thread = {
    ...makeThread(),
    turns: [
      {
        id: "active-subscriptions",
        items: [restoredSchedule],
        itemsView: "full",
        status: "completed",
        error: null,
        startedAt: null,
        completedAt: null,
        durationMs: null,
      },
    ],
  };

  queuePendingThreadUpdate(pendingUpdates, "thread-1", (thread) =>
    updateThreadItem(thread, "turn-schedule", liveSchedule),
  );

  const updated = applyPendingThreadUpdates(snapshot, pendingUpdates);
  const analysis = buildThreadAnalysis(updated, 0);

  assert.equal(pendingUpdates.size, 0);
  assert.deepEqual(
    analysis.monitors.sections.find((section) => section.kind === "schedule")
      ?.monitors,
    [
      {
        id: "schedule-live",
        subscriptionId: "sub-schedule",
        kind: "schedule",
        label: "daily digest",
        detail: "every_interval 6h",
        status: "Listening",
        eventCount: 0,
        latestEvent: null,
      },
    ],
  );
});

test("lazy list and subscribe metadata hydrate restored active subscription turns", () => {
  const listedThread = {
    ...makeThread(),
    turns: [],
  };
  const subscribedThread = {
    ...makeThread(),
    preview: "subscribed metadata",
    turns: [],
  };
  const restoredSchedule: ThreadItem = {
    type: "builtinToolCall",
    id: "active-subscription:sub-schedule",
    tool: "schedule_subscribe",
    arguments: {
      label: "daily digest",
      schedule: { kind: "every_interval", interval_ms: 21_600_000 },
    },
    status: "completed",
    output: {
      subscription_id: "sub-schedule",
      schedule_summary: "every 21600000 ms",
    },
  };
  const readSnapshot: Thread = {
    ...makeThread(),
    turns: [
      {
        id: "active-subscriptions",
        items: [restoredSchedule],
        itemsView: "full",
        status: "completed",
        error: null,
        startedAt: null,
        completedAt: null,
        durationMs: null,
      },
    ],
  };

  let threads = upsertThread([], listedThread);
  threads = upsertThreadMetadataPreservingTurns(threads, subscribedThread);
  threads = upsertThread(threads, readSnapshot);
  const analysis = buildThreadAnalysis(threads[0]!, 0);

  assert.deepEqual(
    analysis.monitors.sections.find((section) => section.kind === "schedule")
      ?.monitors,
    [
      {
        id: "active-subscription:sub-schedule",
        subscriptionId: "sub-schedule",
        kind: "schedule",
        label: "daily digest",
        detail: "every_interval 6h",
        status: "Listening",
        eventCount: 0,
        latestEvent: null,
      },
    ],
  );
});

test("metadata-only thread updates preserve active subscription current state", () => {
  const restoredSchedule: ThreadItem = {
    type: "builtinToolCall",
    id: "active-subscription:sub-schedule",
    tool: "schedule_subscribe",
    arguments: {
      label: "daily digest",
      schedule: { kind: "every_interval", interval_ms: 21_600_000 },
    },
    status: "completed",
    output: {
      subscription_id: "sub-schedule",
      schedule_summary: "every 21600000 ms",
    },
  };
  const loadedThread = {
    ...makeThread(),
    turns: [],
    activeSubscriptionItems: [restoredSchedule],
  };
  const metadataThread = {
    ...makeThread(),
    preview: "metadata refresh",
    turns: [],
  };

  let threads = upsertThread([], loadedThread);
  threads = upsertThreadMetadataPreservingTurns(threads, metadataThread);

  assert.deepEqual(threads[0]?.activeSubscriptionItems, [restoredSchedule]);
});

test("normalizing active subscription current state updates same-length items", () => {
  const originalSchedule: ThreadItem = {
    type: "builtinToolCall",
    id: "active-subscription:sub-schedule",
    tool: "schedule_subscribe",
    arguments: {
      label: "daily digest",
      schedule: { kind: "every_interval", interval_ms: 21_600_000 },
    },
    status: "completed",
    output: {
      subscription_id: "sub-schedule",
      schedule_summary: "every 21600000 ms",
    },
  };
  const updatedSchedule: ThreadItem = {
    ...originalSchedule,
    arguments: {
      label: "daily cargo clean",
      schedule: { kind: "every_interval", interval_ms: 21_600_000 },
    },
  };

  const thread = normalizeThreadSnapshot({
    ...makeThread(),
    turns: [],
    activeSubscriptionItems: [updatedSchedule],
  });

  assert.notDeepEqual(thread.activeSubscriptionItems, [originalSchedule]);
  assert.deepEqual(thread.activeSubscriptionItems, [updatedSchedule]);
});

for (const terminalStatus of ["completed", "errored", "shutdown", "notFound"]) {
  test(`updateThreadItem preserves repeated terminal collab status updates for ${terminalStatus}`, () => {
    const thread = updateThreadItem(makeThread(), "turn-1", {
      type: "collabAgentStatusUpdate",
      id: "first-completion",
      senderThreadId: "thread-child",
      senderPath: "/root/worker",
      recipientThreadId: "thread-1",
      recipientPath: "/root",
      lifecycleStatus: collabAgentLifecycleState(terminalStatus, "done", null),
    });

    const updated = updateThreadItem(thread, "turn-1", {
      type: "collabAgentStatusUpdate",
      id: "second-completion",
      senderThreadId: "thread-child",
      senderPath: "/root/worker",
      recipientThreadId: "thread-1",
      recipientPath: "/root",
      lifecycleStatus: collabAgentLifecycleState(terminalStatus, "done", null),
    });

    assert.deepEqual(updated.turns[0]?.items, [
      {
        type: "collabAgentStatusUpdate",
        id: "first-completion",
        senderThreadId: "thread-child",
        senderPath: "/root/worker",
        recipientThreadId: "thread-1",
        recipientPath: "/root",
        lifecycleStatus: collabAgentLifecycleState(terminalStatus, "done", null),
      },
      {
        type: "collabAgentStatusUpdate",
        id: "second-completion",
        senderThreadId: "thread-child",
        senderPath: "/root/worker",
        recipientThreadId: "thread-1",
        recipientPath: "/root",
        lifecycleStatus: collabAgentLifecycleState(terminalStatus, "done", null),
      },
    ]);
  });
}

test("collab status item notifications target the recipient thread", () => {
  const item: ThreadItem = {
    type: "collabAgentStatusUpdate",
    id: "subagent-complete",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    lifecycleStatus: collabAgentLifecycleState("completed"),
  };

  assert.deepEqual(
    getThreadItemNotificationTargetThreadIds("thread-child", item),
    ["thread-1"],
  );

  const updatedRoot = updateThreadItem(makeThread(), "turn-child", item, {
    startedAtMs: 2_000,
    completedAtMs: 3_000,
    syntheticTurnStatus: "completed",
  });
  const entries = buildConversationEntries(updatedRoot);

  assert.equal(
    isThreadThinking(updatedRoot, {
      isLoadingThread: false,
      isSending: false,
    }),
    false,
  );
  assert.deepEqual(
    updatedRoot.turns.map((turn) => ({
      id: turn.id,
      status: turn.status,
      startedAt: turn.startedAt,
      completedAt: turn.completedAt,
      durationMs: turn.durationMs,
    })),
    [
      {
        id: "turn-child",
        status: "completed",
        startedAt: 2,
        completedAt: 3,
        durationMs: 1_000,
      },
    ],
  );
  assert.deepEqual(updatedRoot.turns[0]?.items, [
    {
      ...item,
      startedAtMs: 2_000,
      completedAtMs: 3_000,
    },
  ]);
  assert.deepEqual(
    entries.map((entry) => [entry.kind, entry.toolName, entry.text]),
    [
      [
        "tool",
        "/root/worker subagent completion",
        "/root/worker • completed • done",
      ],
    ],
  );
});

test("collab status completion notifications join the active parent turn", () => {
  const activeThread: Thread = {
    ...makeThread(),
    turns: [
      {
        id: "turn-parent",
        items: [
          {
            type: "userMessage",
            id: "user-1",
            content: [{ type: "text", text: "wait for worker" }],
          },
        ],
        itemsView: "full",
        status: "running",
        error: null,
        startedAt: 2,
        completedAt: null,
        durationMs: null,
      },
    ],
  };
  const item: ThreadItem = {
    type: "collabAgentStatusUpdate",
    id: "subagent-complete",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    lifecycleStatus: collabAgentLifecycleState("completed"),
  };

  const updatedRoot = updateThreadItem(activeThread, "turn-child", item, {
    completedAtMs: 3_000,
    syntheticTurnStatus: "completed",
  });

  assert.deepEqual(
    updatedRoot.turns.map((turn) => ({
      id: turn.id,
      status: turn.status,
      startedAt: turn.startedAt,
      completedAt: turn.completedAt,
      durationMs: turn.durationMs,
    })),
    [
      {
        id: "turn-parent",
        status: "running",
        startedAt: 2,
        completedAt: null,
        durationMs: null,
      },
    ],
  );
  assert.deepEqual(updatedRoot.turns[0]?.items, [
    activeThread.turns[0]?.items[0],
    {
      ...item,
      completedAtMs: 3_000,
    },
  ]);
});

test("legacy child completion notifications join the active parent turn", () => {
  const activeThread: Thread = {
    ...makeThread(),
    turns: [
      {
        id: "turn-parent",
        items: [],
        itemsView: "full",
        status: "inProgress",
        error: null,
        startedAt: 2,
        completedAt: null,
        durationMs: null,
      },
    ],
  };
  const item: ThreadItem = {
    type: "collabAgentMessage",
    id: "child-completion",
    operation: "childCompletion",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    otherRecipientPaths: [],
    content: "done",
    triggerTurn: true,
  };

  const updatedRoot = updateThreadItem(activeThread, "turn-child", item, {
    completedAtMs: 3_000,
    syntheticTurnStatus: "completed",
  });

  assert.equal(updatedRoot.turns.length, 1);
  assert.deepEqual(updatedRoot.turns[0]?.items, [
    {
      ...item,
      completedAtMs: 3_000,
    },
  ]);
  assert.equal(updatedRoot.turns[0]?.status, "inProgress");
});

test("collab status item completion updates an existing synthetic recipient turn", () => {
  const item: ThreadItem = {
    type: "collabAgentStatusUpdate",
    id: "subagent-complete",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    lifecycleStatus: collabAgentLifecycleState("completed"),
  };
  const runningRoot = updateThreadItem(makeThread(), "turn-child", item, {
    startedAtMs: 2_000,
  });

  const completedRoot = updateThreadItem(runningRoot, "turn-child", item, {
    completedAtMs: 3_000,
    syntheticTurnStatus: "completed",
  });

  assert.equal(
    isThreadThinking(completedRoot, {
      isLoadingThread: false,
      isSending: false,
    }),
    false,
  );
  assert.deepEqual(
    completedRoot.turns.map((turn) => ({
      id: turn.id,
      status: turn.status,
      startedAt: turn.startedAt,
      completedAt: turn.completedAt,
      durationMs: turn.durationMs,
    })),
    [
      {
        id: "turn-child",
        status: "completed",
        startedAt: 2,
        completedAt: 3,
        durationMs: 1_000,
      },
    ],
  );
  assert.deepEqual(completedRoot.turns[0]?.items, [
    {
      ...item,
      startedAtMs: 2_000,
      completedAtMs: 3_000,
    },
  ]);
});

test("child completion synthetic turn moves before later parent messages", () => {
  const parentThread: Thread = {
    ...makeThread(),
    turns: [
      {
        id: "turn-parent",
        items: [
          {
            type: "userMessage",
            id: "user-1",
            content: [{ type: "text", text: "wait for worker" }],
          },
        ],
        itemsView: "full",
        status: "completed",
        error: null,
        startedAt: 1,
        completedAt: 2,
        durationMs: 1_000,
      },
    ],
  };
  const childCompletion: ThreadItem = {
    type: "collabAgentStatusUpdate",
    id: "subagent-complete",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    lifecycleStatus: collabAgentLifecycleState("completed"),
  };
  const withSyntheticChildCompletion = updateThreadItem(
    parentThread,
    "turn-child",
    childCompletion,
    {
      completedAtMs: 3_000,
      syntheticTurnStatus: "completed",
    },
  );

  const updated = updateThreadItem(
    withSyntheticChildCompletion,
    "turn-parent",
    {
      type: "agentMessage",
      id: "agent-after-child",
      text: "continuing after worker",
      phase: null,
      memoryCitation: null,
    },
    { completedAtMs: 4_000 },
  );

  assert.equal(updated.turns.length, 1);
  assert.deepEqual(
    buildConversationEntries(updated).map((entry) => entry.text),
    [
      "wait for worker",
      "/root/worker • completed • done",
      "continuing after worker",
    ],
  );
});

test("turn lifecycle updates preserve live child completion items", () => {
  const childCompletion: ThreadItem = {
    type: "collabAgentStatusUpdate",
    id: "subagent-complete",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    lifecycleStatus: collabAgentLifecycleState("completed"),
  };
  const liveThread: Thread = {
    ...makeThread(),
    turns: [
      {
        id: "turn-parent",
        items: [
          {
            type: "userMessage",
            id: "user-1",
            content: [{ type: "text", text: "run worker" }],
          },
          {
            ...childCompletion,
            completedAtMs: 3_000,
          },
        ],
        itemsView: "full",
        status: "running",
        error: null,
        startedAt: 1,
        completedAt: null,
        durationMs: null,
      },
    ],
  };

  const updated = updateThreadTurnLifecycle(liveThread, {
    id: "turn-parent",
    items: [
      {
        type: "userMessage",
        id: "user-1",
        content: [{ type: "text", text: "run worker" }],
      },
    ],
    itemsView: "full",
    status: "completed",
    error: null,
    startedAt: 1,
    completedAt: 4,
    durationMs: 3_000,
  });

  assert.equal(updated.turns[0]?.status, "completed");
  assert.deepEqual(
    buildConversationEntries(updated).map((entry) => entry.text),
    ["run worker", "/root/worker • completed • done"],
  );
});

test("turn lifecycle updates create empty turns when local items are missing", () => {
  const updated = updateThreadTurnLifecycle(makeThread(), {
    id: "turn-lifecycle",
    items: [
      {
        type: "agentMessage",
        id: "agent-from-snapshot",
        text: "snapshot-only text",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "full",
    status: "running",
    error: null,
    startedAt: 5,
    completedAt: null,
    durationMs: null,
  });

  assert.deepEqual(updated.turns, [
    {
      id: "turn-lifecycle",
      items: [],
      itemsView: "full",
      status: "running",
      error: null,
      startedAt: 5,
      completedAt: null,
      durationMs: null,
    },
  ]);
});

test("metadata updates preserve repeated terminal collab status live items", () => {
  const firstCompletion: ThreadItem = {
    type: "collabAgentStatusUpdate",
    id: "first-completion",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    lifecycleStatus: collabAgentLifecycleState("completed"),
  };
  const secondCompletion: ThreadItem = {
    ...firstCompletion,
    id: "second-completion",
  };
  const liveThread: Thread = {
    ...makeThread(),
    turns: [
      {
        id: "turn-live",
        items: [firstCompletion, secondCompletion],
        itemsView: "full",
        status: "completed",
        error: null,
        startedAt: 1,
        completedAt: 2,
        durationMs: 1_000,
      },
    ],
  };

  const updated = upsertThreadMetadataPreservingTurns([liveThread], {
    ...liveThread,
    preview: "metadata refreshed",
    updatedAt: 10,
    turns: [],
  });

  assert.equal(updated[0]?.preview, "metadata refreshed");
  assert.deepEqual(
    updated[0]?.turns[0]?.items.map((item) => item.id),
    ["first-completion", "second-completion"],
  );
});

test("direct collab status completion notifications create completed synthetic turns", () => {
  const item: ThreadItem = {
    type: "collabAgentStatusUpdate",
    id: "subagent-complete",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    lifecycleStatus: collabAgentLifecycleState("completed"),
  };

  assert.equal(
    getThreadItemNotificationSyntheticTurnStatus("item/started", item),
    undefined,
  );
  assert.equal(
    getThreadItemNotificationSyntheticTurnStatus("item/completed", item),
    "completed",
  );

  const updatedRoot = updateThreadItem(makeThread(), "turn-root", item, {
    completedAtMs: 3_000,
    syntheticTurnStatus: getThreadItemNotificationSyntheticTurnStatus(
      "item/completed",
      item,
    ),
  });

  assert.equal(
    isThreadThinking(updatedRoot, {
      isLoadingThread: false,
      isSending: false,
    }),
    false,
  );
  assert.deepEqual(
    updatedRoot.turns.map((turn) => ({
      id: turn.id,
      status: turn.status,
      startedAt: turn.startedAt,
      completedAt: turn.completedAt,
      durationMs: turn.durationMs,
    })),
    [
      {
        id: "turn-root",
        status: "completed",
        startedAt: 3,
        completedAt: 3,
        durationMs: null,
      },
    ],
  );
});

test("terminal child status does not hide restored conversation history or keep loaded thread active", () => {
  const restoredThread: Thread = {
    ...makeThread(),
    lifecycleStatus: { type: "final", result: { type: "completed" } },
    turns: [
      {
        id: "turn-user",
        items: [
          {
            type: "userMessage",
            id: "user-1",
            content: [{ type: "text", text: "please fix this" }],
          },
          {
            type: "agentMessage",
            id: "agent-1",
            text: "I fixed it.",
            phase: null,
            memoryCitation: null,
          },
        ],
        itemsView: "full",
        status: "completed",
        error: null,
        startedAt: 1,
        completedAt: 2,
        durationMs: 1_000,
      },
    ],
  };

  const childStatus: ThreadItem = {
    type: "collabAgentStatusUpdate",
    id: "child-shutdown",
    senderThreadId: "thread-child",
    senderPath: "/root/worker/tester",
    recipientThreadId: "thread-1",
    recipientPath: "/root/worker",
    lifecycleStatus: collabAgentLifecycleState("shutdown", "completed", "/root/worker/tester"),
  };
  const updated = updateThreadItem(restoredThread, "turn-child", childStatus, {
    completedAtMs: 3_000,
    syntheticTurnStatus: "completed",
  });

  assert.deepEqual(
    buildConversationEntries(updated).map((entry) => entry.text),
    [
      "please fix this",
      "I fixed it.",
      "/root/worker/tester • shutdown • completed",
    ],
  );
  assert.equal(getThreadPresenceLabel(updated), "Complete");
  assert.equal(threadDisplayStatusClass(updated), "todo");
});

test("initialized live child completion preserves existing assistant messages", () => {
  const initializedThread: Thread = {
    ...makeThread(),
    turns: [
      {
        id: "turn-parent",
        items: [
          {
            type: "userMessage",
            id: "user-1",
            content: [{ type: "text", text: "run worker" }],
          },
          {
            type: "agentMessage",
            id: "agent-1",
            text: "Worker is running.",
            phase: null,
            memoryCitation: null,
          },
        ],
        itemsView: "full",
        status: "completed",
        error: null,
        startedAt: 1,
        completedAt: 2,
        durationMs: 1_000,
      },
    ],
  };
  const childStatus: ThreadItem = {
    type: "collabAgentStatusUpdate",
    id: "child-complete",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    lifecycleStatus: collabAgentLifecycleState("completed"),
  };

  const [updated] = applyInitializedThreadUpdate(
    [initializedThread],
    new Set(["thread-1"]),
    "thread-1",
    (thread) =>
      updateThreadItem(thread, "turn-child", childStatus, {
        completedAtMs: 3_000,
        syntheticTurnStatus: "completed",
      }),
  );

  assert.ok(updated);
  assert.deepEqual(
    buildConversationEntries(updated).map((entry) => entry.text),
    [
      "run worker",
      "Worker is running.",
      "/root/worker • completed • done",
    ],
  );
});

test("uninitialized live child completion does not create display turn cache", () => {
  const thread = makeThread();
  const childStatus: ThreadItem = {
    type: "collabAgentStatusUpdate",
    id: "child-complete",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    lifecycleStatus: collabAgentLifecycleState("completed"),
  };

  const updatedThreads = applyInitializedThreadUpdate(
    [thread],
    new Set(),
    "thread-1",
    (current) =>
      updateThreadItem(current, "turn-child", childStatus, {
        completedAtMs: 3_000,
        syntheticTurnStatus: "completed",
      }),
  );

  assert.deepEqual(updatedThreads, [thread]);
  assert.deepEqual(
    buildConversationEntries(updatedThreads[0]!).map((entry) => entry.text),
    [],
  );
});

test("collab status item notifications stay on the notification thread unless sent by that thread", () => {
  const item: ThreadItem = {
    type: "collabAgentStatusUpdate",
    id: "subagent-complete",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    lifecycleStatus: collabAgentLifecycleState("completed"),
  };

  assert.deepEqual(getThreadItemNotificationTargetThreadIds("thread-1", item), [
    "thread-1",
  ]);
});

test("legacy child completion message notifications target the recipient thread", () => {
  const item: ThreadItem = {
    type: "collabAgentMessage",
    id: "child-completion",
    operation: "childCompletion",
    senderThreadId: "thread-child",
    senderPath: "/root/worker",
    recipientThreadId: "thread-1",
    recipientPath: "/root",
    otherRecipientPaths: [],
    content: "done",
    triggerTurn: true,
  };

  assert.deepEqual(
    getThreadItemNotificationTargetThreadIds("thread-child", item),
    ["thread-1"],
  );
});

test("pending thread updates preserve same-content items with different ids", () => {
  const pendingUpdates = new Map<string, Array<(thread: Thread) => Thread>>();
  queuePendingThreadUpdate(pendingUpdates, "thread-1", (thread) =>
    updateThreadItem(thread, "turn-1", {
      type: "collabAgentMessage",
      id: "pending-item",
      operation: "sendMessage",
      senderThreadId: "thread-child",
      senderPath: "/root/worker",
      recipientThreadId: "thread-1",
      recipientPath: "/root",
      otherRecipientPaths: [],
      content: "same backend message",
      triggerTurn: true,
    }),
  );
  const snapshot = {
    ...makeThread(),
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "collabAgentMessage" as const,
            id: "snapshot-item",
            operation: "sendMessage",
            senderThreadId: "thread-child",
            senderPath: "/root/worker",
            recipientThreadId: "thread-1",
            recipientPath: "/root",
            otherRecipientPaths: [],
            content: "same backend message",
            triggerTurn: true,
          },
        ],
        itemsView: "full" as const,
        status: "running" as const,
        error: null,
        startedAt: 1,
        completedAt: null,
        durationMs: null,
      },
    ],
  };

  const updated = applyPendingThreadUpdates(snapshot, pendingUpdates);

  assert.equal(pendingUpdates.size, 0);
  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "collabAgentMessage",
      id: "snapshot-item",
      operation: "sendMessage",
      senderThreadId: "thread-child",
      senderPath: "/root/worker",
      recipientThreadId: "thread-1",
      recipientPath: "/root",
      otherRecipientPaths: [],
      content: "same backend message",
      triggerTurn: true,
    },
    {
      type: "collabAgentMessage",
      id: "pending-item",
      operation: "sendMessage",
      senderThreadId: "thread-child",
      senderPath: "/root/worker",
      recipientThreadId: "thread-1",
      recipientPath: "/root",
      otherRecipientPaths: [],
      content: "same backend message",
      triggerTurn: true,
    },
  ]);
});

test("pending agent deltas preserve same-content completed assistant blocks with different ids", () => {
  const pendingUpdates = new Map<string, Array<(thread: Thread) => Thread>>();
  queuePendingThreadUpdate(pendingUpdates, "thread-1", (thread) =>
    appendAgentDelta(thread, "pending-turn", "pending-item", "same response"),
  );
  const snapshot = {
    ...makeThread(),
    turns: [
      {
        id: "snapshot-turn",
        items: [
          {
            type: "agentMessage" as const,
            id: "snapshot-item",
            text: "same response",
            phase: null,
            memoryCitation: null,
          },
        ],
        itemsView: "full" as const,
        status: "completed" as const,
        error: null,
        startedAt: 10,
        completedAt: 12,
        durationMs: 2000,
      },
    ],
  };

  const updated = applyPendingThreadUpdates(snapshot, pendingUpdates);

  assert.equal(pendingUpdates.size, 0);
  assert.deepEqual(updated.turns, [
    ...snapshot.turns,
    {
      id: "pending-turn",
      items: [
        {
          type: "agentMessage",
          id: "pending-item",
          text: "same response",
          phase: null,
          memoryCitation: null,
        },
      ],
      itemsView: "full",
      status: "running",
      error: null,
      startedAt: null,
      completedAt: null,
      durationMs: null,
    },
  ]);
});

for (const operation of ["sendMessage", "send_message"]) {
  test(`mergeThreadSnapshot preserves raw ${operation} assistant envelope text`, () => {
    const turn: Turn = {
      id: "turn-1",
      items: [
        {
          type: "agentMessage",
          id: `raw-${operation}`,
          text: JSON.stringify({
            author: "/root/worker",
            recipient: "/root",
            content: "legacy message",
            operation,
          }),
          phase: null,
          memoryCitation: null,
        },
      ],
      itemsView: "full",
      status: "completed",
      error: null,
      startedAt: 10,
      completedAt: 12,
      durationMs: 2000,
    };

    const merged = mergeThreadSnapshot(null, {
      ...makeThread(),
      turns: [turn],
    });

    assert.deepEqual(merged.turns[0]?.items, turn.items);
  });
}

test("pending agent deltas preserve structured process exit marker text with a distinct id", () => {
  const pendingUpdates = new Map<string, Array<(thread: Thread) => Thread>>();
  queuePendingThreadUpdate(pendingUpdates, "thread-1", (thread) =>
    appendAgentDelta(
      thread,
      "pending-turn",
      "pending-item",
      '<event_driven_tool>{"tool":"process_exit_subscribe","title":"Process exited","text":"Session 42 exited with code 0"}</event_driven_tool>',
    ),
  );
  const snapshot = {
    ...makeThread(),
    turns: [
      {
        id: "snapshot-turn",
        items: [
          {
            type: "eventDrivenTool" as const,
            id: "snapshot-item",
            tool: "process_exit_subscribe",
            title: "Process exited",
            text: "Session 42 exited with code 0",
          },
        ],
        itemsView: "full" as const,
        status: "completed" as const,
        error: null,
        startedAt: 10,
        completedAt: 12,
        durationMs: 2000,
      },
    ],
  };

  const updated = applyPendingThreadUpdates(snapshot, pendingUpdates);

  assert.equal(pendingUpdates.size, 0);
  assert.deepEqual(updated.turns, [
    ...snapshot.turns,
    {
      id: "pending-turn",
      items: [
        {
          type: "agentMessage",
          id: "pending-item",
          text: '<event_driven_tool>{"tool":"process_exit_subscribe","title":"Process exited","text":"Session 42 exited with code 0"}</event_driven_tool>',
          phase: null,
          memoryCitation: null,
        },
      ],
      itemsView: "full",
      status: "running",
      error: null,
      startedAt: null,
      completedAt: null,
      durationMs: null,
    },
  ]);
});

test("appendAgentDelta preserves split structured process exit marker text", () => {
  const partial = appendAgentDelta(
    makeThread(),
    "turn-1",
    "item-1",
    "<event",
  );
  const middle = appendAgentDelta(
    partial,
    "turn-1",
    "item-1",
    '_driven_tool>{"tool":"process_exit_subscribe",',
  );
  const later = appendAgentDelta(
    middle,
    "turn-1",
    "item-1",
    '"title":"Process exited",',
  );

  const updated = appendAgentDelta(
    later,
    "turn-1",
    "item-1",
    '"text":"Session 42 exited with code 0"}</event_driven_tool>',
  );

  assert.equal(updated.turns[0]?.items[0]?.type, "agentMessage");
  assert.equal(
    updated.turns[0]?.items[0]?.type === "agentMessage"
      ? updated.turns[0].items[0].text
      : "",
    '<event_driven_tool>{"tool":"process_exit_subscribe","title":"Process exited","text":"Session 42 exited with code 0"}</event_driven_tool>',
  );
});

for (const operation of ["sendMessage", "send_message", "childCompletion"]) {
  test(`appendAgentDelta preserves split raw ${operation} assistant envelope`, () => {
    const envelope = JSON.stringify({
      author: "/root/worker",
      recipient: "/root",
      content: "legacy message",
      operation,
    });
    const chunks = [
      envelope.slice(0, 8),
      envelope.slice(8, 34),
      envelope.slice(34, 72),
      envelope.slice(72),
    ];

    let thread = makeThread();
    for (const chunk of chunks) {
      thread = appendAgentDelta(thread, "turn-1", "item-1", chunk);
    }
    assert.equal(thread.turns[0]?.items[0]?.type, "agentMessage");
    assert.equal(
      thread.turns[0]?.items[0]?.type === "agentMessage"
        ? thread.turns[0].items[0].text
        : "",
      envelope,
    );
  });
}

test("appendAgentDelta keeps split ordinary JSON assistant text", () => {
  const partial = appendAgentDelta(
    makeThread(),
    "turn-1",
    "item-1",
    '{"foo":',
  );

  const updated = appendAgentDelta(partial, "turn-1", "item-1", '"bar"}');

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "agentMessage",
      id: "item-1",
      text: '{"foo":"bar"}',
      phase: null,
      memoryCitation: null,
    },
  ]);
});

test("appendAgentDelta releases split nonlegacy author JSON assistant text", () => {
  const partial = appendAgentDelta(
    makeThread(),
    "turn-1",
    "item-1",
    '{"author":',
  );

  const updated = appendAgentDelta(partial, "turn-1", "item-1", '"assistant"}');

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "agentMessage",
      id: "item-1",
      text: '{"author":"assistant"}',
      phase: null,
      memoryCitation: null,
    },
  ]);
});

test("updateThreadItem clears suppressed legacy stream buffers", () => {
  const partial = appendAgentDelta(
    makeThread(),
    "turn-1",
    "item-1",
    '{"author":',
  );
  const completed = updateThreadItem(
    partial,
    "turn-1",
    {
      type: "agentMessage",
      id: "item-1",
      text: "visible",
      phase: null,
      memoryCitation: null,
    },
    { completedAtMs: 1_000 },
  );

  const updated = appendAgentDelta(completed, "turn-1", "item-1", " text");

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "agentMessage",
      id: "item-1",
      text: "visible text",
      phase: null,
      memoryCitation: null,
      completedAtMs: 1_000,
    },
  ]);
});

test("updateThreadTurn clears suppressed legacy stream buffers", () => {
  const partial = appendAgentDelta(
    makeThread(),
    "turn-1",
    "item-1",
    "<event",
  );
  const completed = updateThreadTurn(partial, {
    id: "turn-1",
    items: [
      {
        type: "agentMessage",
        id: "item-1",
        text: "visible",
        phase: null,
        memoryCitation: null,
      },
    ],
    itemsView: "full",
    status: "completed",
    error: null,
    startedAt: 1,
    completedAt: 2,
    durationMs: 1000,
  });

  const updated = appendAgentDelta(completed, "turn-1", "item-1", " text");

  assert.deepEqual(updated.turns[0]?.items, [
    {
      type: "agentMessage",
      id: "item-1",
      text: "visible text",
      phase: null,
      memoryCitation: null,
    },
  ]);
});

test("threadStatusClass treats active thread status as doing", () => {
  assert.equal(
    threadStatusClass({
      type: "active",
      activeFlags: [],
    }),
    "doing",
  );
});

test("treeThreadLifecycleStatusClass keeps self active thread green", () => {
  const thread = {
    ...makeThread(),
    lifecycleStatus: { type: "active" as const, activeFlags: [] },
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "agentMessage" as const,
            id: "item-1",
            text: "working",
            phase: null,
            memoryCitation: null,
          },
        ],
        itemsView: "full" as const,
        status: "running" as const,
        error: null,
        startedAt: 1,
        completedAt: null,
        durationMs: null,
      },
    ],
  };

  assert.equal(treeThreadLifecycleStatusClass(makeTreeNode(thread)), "doing");
});

test("treeThreadLifecycleStatusClass shows subagent waiting separately", () => {
  const thread = {
    ...makeThread(),
    lifecycleStatus: { type: "waiting" as const, reason: "child" as const },
  } satisfies Thread;

  assert.equal(treeThreadLifecycleStatusClass(makeTreeNode(thread)), "waiting-subagent");
  assert.equal(
    treeThreadLifecycleStatusLabel("waiting-subagent"),
    "Waiting on subagent",
  );
});

test("treeThreadLifecycleStatusClass ignores item-derived subagent waits", () => {
  const thread = {
    ...makeThread(),
    lifecycleStatus: { type: "active" as const, activeFlags: [] },
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "agentMessage" as const,
            id: "item-1",
            text: "working",
            phase: null,
            memoryCitation: null,
          },
          {
            type: "collabAgentToolCall" as const,
            id: "item-2",
            tool: "Wait",
            status: "InProgress",
            senderThreadId: "thread-1",
            senderPath: "/root",
            receiverThreadIds: ["thread-2"],
            receiverPaths: ["/root/worker"],
            prompt: null,
            model: null,
            reasoningEffort: null,
            agentsStates: {},
          },
        ],
        itemsView: "full" as const,
        status: "inProgress" as const,
        error: null,
        startedAt: 1,
        completedAt: null,
        durationMs: null,
      },
    ],
  };

  assert.equal(treeThreadLifecycleStatusClass(makeTreeNode(thread)), "doing");
});

test("treeThreadLifecycleStatusClass shows event tool waiting separately", () => {
  const thread = {
    ...makeThread(),
    lifecycleStatus: { type: "waiting" as const, reason: "command" as const },
  } satisfies Thread;

  assert.equal(
    treeThreadLifecycleStatusClass(makeTreeNode(thread)),
    "waiting-eventtool",
  );
});

test("treeThreadLifecycleStatusClass prioritizes backend event tool flags over subagent flags", () => {
  const thread = {
    ...makeThread(),
    lifecycleStatus: { type: "waiting" as const, reason: "command" as const },
  } satisfies Thread;

  assert.equal(
    treeThreadLifecycleStatusClass(makeTreeNode(thread)),
    "waiting-eventtool",
  );
});

test("treeThreadLifecycleStatusClass ignores process exit restore failures when backend status is idle", () => {
  const thread = {
    ...makeThread(),
    lifecycleStatus: { type: "final" as const, result: { type: "completed" as const } },
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "eventDrivenToolCall" as const,
            id: "item-1",
            tool: "process_exit_subscribe",
            arguments: { session_id: 42, label: "build process" },
            status: "completed",
            output: { subscription_id: "sub-1" },
          },
          {
            type: "eventDrivenTool" as const,
            id: "item-2",
            tool: "process_exit_subscribe",
            title: "Process exit restore failed",
            text: "[Process exit subscription restore (build process)] Could not restore session 42 after restart because the original exec session is no longer available.",
          },
        ],
        itemsView: "full" as const,
        status: "completed" as const,
        error: null,
        startedAt: 1,
        completedAt: 2,
        durationMs: 1000,
      },
    ],
  };

  assert.equal(treeThreadLifecycleStatusClass(makeTreeNode(thread)), "todo");
});

test("treeThreadLifecycleStatusClass ignores event tool subscriptions after unsubscribe when backend status is idle", () => {
  const thread = {
    ...makeThread(),
    lifecycleStatus: { type: "final" as const, result: { type: "completed" as const } },
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "eventDrivenToolCall" as const,
            id: "item-1",
            tool: "process_exit_subscribe",
            arguments: { session_id: 42 },
            status: "completed",
            output: { subscription_id: "sub-1" },
          },
          {
            type: "eventDrivenToolCall" as const,
            id: "item-2",
            tool: "process_exit_unsubscribe",
            arguments: { subscription_id: "sub-1" },
            status: "completed",
            output: { ok: true },
          },
        ],
        itemsView: "full" as const,
        status: "completed" as const,
        error: null,
        startedAt: 1,
        completedAt: 2,
        durationMs: 1000,
      },
    ],
  };

  assert.equal(treeThreadLifecycleStatusClass(makeTreeNode(thread)), "todo");
});

test("treeThreadLifecycleStatusClass uses backend parent wait-child status directly", () => {
  const parent = {
    ...makeThread(),
    id: "parent",
    lifecycleStatus: { type: "waiting" as const, reason: "child" as const },
  };
  const child = {
    ...makeThread(),
    id: "child",
    lifecycleStatus: { type: "active" as const, activeFlags: [] },
  };

  assert.equal(
    treeThreadLifecycleStatusClass(makeTreeNode(parent, [makeTreeNode(child)])),
    "waiting-subagent",
  );
});

test("treeThreadLifecycleStatusClass does not infer parent status from active descendants", () => {
  const parent = {
    ...makeThread(),
    id: "parent",
  };
  const activeChild = {
    ...makeThread(),
    id: "active-child",
    lifecycleStatus: { type: "active" as const, activeFlags: [] },
  };

  assert.equal(
    treeThreadLifecycleStatusClass(makeTreeNode(parent, [makeTreeNode(activeChild)])),
    "todo",
  );
});

test("shouldNotifyProjectThreadCompleted only fires on project completed edges", () => {
  const completed = {
    type: "final" as const,
    result: { type: "completed" as const },
  };
  const active = { type: "active" as const, activeFlags: [] };
  const waiting = { type: "waiting" as const, reason: "command" as const };
  const failed = {
    type: "final" as const,
    result: { type: "errored" as const, message: "failed" },
  };
  const project = makeSidebarThread({
    id: "project",
    cwd: "/work/project",
    lifecycleStatus: active,
  });

  assert.equal(shouldNotifyProjectThreadCompleted(project, completed), true);
  assert.equal(
    shouldNotifyProjectThreadCompleted(
      { ...project, lifecycleStatus: waiting },
      completed,
    ),
    true,
  );
  assert.equal(
    shouldNotifyProjectThreadCompleted(
      { ...project, lifecycleStatus: completed },
      completed,
    ),
    false,
  );
  assert.equal(shouldNotifyProjectThreadCompleted(project, failed), false);
  assert.equal(
    shouldNotifyProjectThreadCompleted(
      makeSidebarThread({
        id: "chat",
        cwd: `/tmp/root-worker/${CHAT_COMPAT_CWD_BASENAME}`,
        lifecycleStatus: active,
      }),
      completed,
    ),
    false,
  );
  assert.equal(
    shouldNotifyProjectThreadCompleted(
      makeSubagentThread("child", "project", "/root/child", {
        cwd: "/work/project",
        lifecycleStatus: active,
      }),
      completed,
    ),
    false,
  );
});

test("treeThreadLifecycleStatusClass does not infer parent status from descendant waits", () => {
  const parent = {
    ...makeThread(),
    id: "parent",
  };
  const child = {
    ...makeThread(),
    id: "child",
    lifecycleStatus: { type: "waiting" as const, reason: "command" as const },
  } satisfies Thread;

  assert.equal(
    treeThreadLifecycleStatusClass(makeTreeNode(parent, [makeTreeNode(child)])),
    "todo",
  );
});

test("treeThreadLifecycleStatusClass does not infer parent status from descendant errors", () => {
  const parent = {
    ...makeThread(),
    id: "parent",
  };
  const child = {
    ...makeThread(),
    id: "child",
    lifecycleStatus: { type: "systemError" as const },
  };

  assert.equal(
    treeThreadLifecycleStatusClass(makeTreeNode(parent, [makeTreeNode(child)])),
    "todo",
  );
});

test("treeThreadLifecycleStatusClass leaves inactive trees unchanged", () => {
  const parent = {
    ...makeThread(),
    id: "parent",
  };
  const child = {
    ...makeThread(),
    id: "child",
  };

  assert.equal(
    treeThreadLifecycleStatusClass(makeTreeNode(parent, [makeTreeNode(child)])),
    "todo",
  );
});

test("thread path helpers read snake_case thread spawn metadata", () => {
  const thread = {
    ...makeThread(),
    source: {
      subAgent: {
        thread_spawn: {
          parent_thread_id: "parent-1",
          depth: 1,
          agent_path: "/root/worker",
          agent_nickname: "worker",
          agent_role: "Worker Agent",
        },
      },
    },
  } satisfies Thread;

  assert.equal(getThreadPath(thread), "/root/worker");
  assert.equal(getParentThreadId(thread), "parent-1");
});

test("thread path helpers read camelCase thread spawn metadata", () => {
  const thread = {
    ...makeThread(),
    source: {
      subAgent: {
        threadSpawn: {
          parentThreadId: "parent-2",
          depth: 1,
          agentPath: "/root/reviewer",
          agentNickname: "reviewer",
          agentRole: "Reviewer",
        },
      },
    } as unknown as Thread["source"],
  };

  assert.equal(getThreadPath(thread), "/root/reviewer");
  assert.equal(getParentThreadId(thread), "parent-2");
});

test("buildCurrentThreadTodoItems only returns direct child threads", () => {
  const directChildUpdatedAt = Math.floor(Date.now() / 1000);
  const parent = {
    ...makeThread(),
    id: "parent",
    updatedAt: 1,
  } satisfies Thread;
  const directChild = {
    ...makeThread(),
    id: "child",
    updatedAt: directChildUpdatedAt,
    name: "Direct Child",
    source: {
      subAgent: {
        thread_spawn: {
          parent_thread_id: "parent",
          depth: 1,
          agent_path: "/root/child",
          agent_nickname: "child",
          agent_role: "worker",
        },
      },
    },
  } satisfies Thread;
  const sibling = {
    ...makeThread(),
    id: "sibling",
    updatedAt: 5,
    name: "Sibling",
    source: {
      subAgent: {
        thread_spawn: {
          parent_thread_id: "other-parent",
          depth: 1,
          agent_path: "/root/sibling",
          agent_nickname: "sibling",
          agent_role: "worker",
        },
      },
    },
  } satisfies Thread;
  const grandchild = {
    ...makeThread(),
    id: "grandchild",
    updatedAt: 6,
    name: "Grandchild",
    source: {
      subAgent: {
        thread_spawn: {
          parent_thread_id: "child",
          depth: 2,
          agent_path: "/root/child/grandchild",
          agent_nickname: "grandchild",
          agent_role: "worker",
        },
      },
    },
  } satisfies Thread;

  assert.deepEqual(
    buildCurrentThreadTodoItems(
      [parent, directChild, sibling, grandchild],
      "parent",
      "all",
    ),
    [
      {
        id: "child",
        title: "Direct Child",
        ownerPath: "/root/child",
        status: "todo",
        statusLabel: "Todo",
        updatedLabel: formatUpdatedLabel(directChildUpdatedAt),
        summary: "",
        threadId: "child",
      },
    ],
  );
});

test("getPresenceLabel surfaces canonical thread lifecycle status", () => {
  assert.equal(
    getPresenceLabel({
      type: "waiting",
      reason: "command",
    }),
    "Waiting on Event Tool",
  );
  assert.equal(
    getPresenceLabel({
      type: "waiting",
      reason: "child",
    }),
    "Waiting on Subagent",
  );
  assert.equal(
    getPresenceLabel({
      type: "active",
      activeFlags: ["waitingOnApproval"],
    }),
    "Waiting on Approval",
  );
  assert.equal(
    getPresenceLabel({
      type: "active",
      activeFlags: ["waitingOnUserInput"],
    }),
    "Waiting on Input",
  );
  assert.equal(
    getPresenceLabel({
      type: "active",
      activeFlags: [],
    }),
    "Active",
  );
  assert.equal(
    getPresenceLabel({
      type: "active",
      activeFlags: ["running"],
    }),
    "Running",
  );
  assert.equal(
    getPresenceLabel({
      type: "final",
      result: { type: "completed" },
    }),
    "Complete",
  );
});

test("isThreadThinking stays false while a turn only injects init context", () => {
  const thread: Thread = {
    ...makeThread(),
    lifecycleStatus: { type: "active" as const, activeFlags: ["waitingOnUserInput"] },
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "userMessage" as const,
            id: "msg-1",
            content: [{ type: "text", text: "hello" }],
          },
          {
            type: "injectedContext" as const,
            id: "ctx-1",
            title: "environment",
            preview: "workspace context",
            sections: [],
          },
        ],
        itemsView: "full" as const,
        status: "inProgress" as const,
        error: null,
        startedAt: 1,
        completedAt: null,
        durationMs: null,
      },
    ],
  };

  assert.equal(
    isThreadThinking(thread, {
      isLoadingThread: false,
      isSending: false,
    }),
    false,
  );
});

test("isThreadThinking ignores item-derived running state when backend status is idle", () => {
  const thread = {
    ...makeThread(),
    lifecycleStatus: { type: "final" as const, result: { type: "completed" as const } },
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "commandExecution" as const,
            id: "cmd-1",
            command: "rtk git status --short",
            cwd: "/tmp",
            status: "running",
            aggregatedOutput: null,
            exitCode: null,
            durationMs: null,
          },
        ],
        itemsView: "full" as const,
        status: "inProgress" as const,
        error: null,
        startedAt: 1,
        completedAt: null,
        durationMs: null,
      },
    ],
  };

  assert.equal(
    isThreadThinking(thread, {
      isLoadingThread: false,
      isSending: false,
    }),
    false,
  );
});

test("isThreadThinking follows backend running active flag", () => {
  const thread: Thread = {
    ...makeThread(),
    lifecycleStatus: { type: "active" as const, activeFlags: ["running"] },
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "userMessage" as const,
            id: "msg-1",
            content: [{ type: "text", text: "hello" }],
          },
          {
            type: "commandExecution" as const,
            id: "cmd-1",
            command: "rtk git status --short",
            cwd: "/tmp",
            status: "running",
            aggregatedOutput: null,
            exitCode: null,
            durationMs: null,
          },
        ],
        itemsView: "full" as const,
        status: "inProgress" as const,
        error: null,
        startedAt: 1,
        completedAt: null,
        durationMs: null,
      },
    ],
  };

  assert.equal(
    isThreadThinking(thread, {
      isLoadingThread: false,
      isSending: false,
    }),
    true,
  );
});
