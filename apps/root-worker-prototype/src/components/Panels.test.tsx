import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import type {
  ProjectAgentSidebar,
  SidebarProjectNode,
  Thread,
  TreeNode,
} from "../types";

(globalThis as typeof globalThis & { React: typeof React }).React = React;
const {
  ConversationPanel,
  NewThreadDialog,
  NewThreadPopover,
  ProjectSection,
  SidebarPanel,
  TreeContextMenu,
  buildBlankChatThreadDraft,
  buildNewThreadDraft,
  defaultNewThreadStartParams,
  isValidAgentPathSegment,
  isValidNewThreadAgentPath,
  resolveNewThreadAgentTypeSelection,
  resolveNewThreadProjectScopedOptions,
  resolveNewThreadProviderControls,
  resolveNewThreadStartParamsForProject,
} = await import("./Panels");

type ReactElementProps = {
  children?: React.ReactNode;
  [key: string]: unknown;
};

const BLANK_THREAD_DRAFT = {
  mode: "chat",
  projectPath: "",
  taskName: "",
  threadProvider: null,
  agentType: null,
  model: null,
  modelProvider: null,
  reasoningEffort: null,
  serviceTier: null,
};

function makeThread(id: string, cwd: string, name: string): Thread {
  return {
    id,
    sessionId: `session-${id}`,
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
    cwd,
    cliVersion: "test",
    source: "cli",
    threadSource: null,
    agentNickname: null,
    agentRole: null,
    gitInfo: null,
    name,
    skills: [],
    turns: [],
  };
}

function makeSubagentThread(
  id: string,
  cwd: string,
  name: string,
  parentThreadId: string,
): Thread {
  return {
    ...makeThread(id, cwd, name),
    source: {
      subAgent: {
        thread_spawn: {
          parent_thread_id: parentThreadId,
          agent_path: `/${name}`,
        },
      },
    },
    threadSource: "subagent",
  };
}

function makeNode(thread: Thread, children: TreeNode[] = []): TreeNode {
  return {
    key: thread.id,
    label: thread.name ?? thread.id,
    path: thread.preview || "Complete",
    thread,
    threadId: thread.id,
    isPlaceholder: false,
    children,
  };
}

function makeProject(
  id: string,
  label: string,
  tree: TreeNode,
  overrides: Partial<SidebarProjectNode> = {},
): SidebarProjectNode {
  return {
    id,
    label,
    subtitle: `/work/${label}`,
    cwd: `/work/${label}`,
    statusClass: "todo",
    updatedAt: 1,
    tree,
    descendantCount: 0,
    activeCount: 0,
    waitingCount: 0,
    failedCount: 0,
    duplicateRootThreadIds: [],
    ...overrides,
  };
}

function makeChatSidebar(
  overrides: Partial<ProjectAgentSidebar["chat"]> = {},
): ProjectAgentSidebar["chat"] {
  return {
    id: "chat",
    statusClass: "todo",
    updatedAt: 0,
    conversations: [],
    ...overrides,
  };
}

function makeSidebar(
  projects: SidebarProjectNode[] = [],
  chat: Partial<ProjectAgentSidebar["chat"]> = {},
): ProjectAgentSidebar {
  return {
    projects,
    chat: makeChatSidebar(chat),
  };
}

function renderSidebar(
  sidebar: ProjectAgentSidebar,
  options?: {
    collapsedProjects?: string[];
    isCreatingChatThread?: boolean;
    selectedThreadId?: string | null;
    collapsedTreeNodes?: string[];
  },
) {
  return renderToStaticMarkup(
    <SidebarPanel
      collapsedProjectSet={new Set(options?.collapsedProjects ?? [])}
      collapsedSet={new Set(options?.collapsedTreeNodes ?? [])}
      isCreatingChatThread={options?.isCreatingChatThread}
      newProjectName="Project chat"
      onArchiveChatThread={() => {}}
      onArchiveProjectThread={() => {}}
      onCreateChatThread={() => {}}
      onCreateProjectThread={() => {}}
      onOpenMenu={() => {}}
      onSelectProject={() => {}}
      onSelectThread={() => {}}
      onSetNewProjectName={() => {}}
      onOpenSettings={() => {}}
      onSubmitNewThreadDraft={() => {}}
      onToggleProject={() => {}}
      onToggleTreeNode={() => {}}
      projectSidebar={sidebar}
      selectedThreadId={options?.selectedThreadId ?? null}
      workspacePath="/work/alpha"
    />,
  );
}

function renderNewThreadPopover() {
  return renderToStaticMarkup(
    <NewThreadPopover
      existingProjectPaths={["/work/alpha", "/work/beta"]}
      onCancel={() => {}}
      onSubmit={() => {}}
      workspacePath="/work/alpha"
    />,
  );
}

function renderNewThreadDialog() {
  return renderToStaticMarkup(
    <NewThreadDialog
      existingProjectPaths={["/work/alpha", "/work/beta"]}
      onCancel={() => {}}
      onSubmit={() => {}}
      workspacePath="/work/alpha"
    />,
  );
}

function renderConversationPanel(
  options: Partial<React.ComponentProps<typeof ConversationPanel>> = {},
) {
  const selectedThread =
    options.selectedThread === undefined
      ? makeThread("thread-1", "/work/alpha", "Alpha chat")
      : options.selectedThread;
  return renderToStaticMarkup(
    <ConversationPanel
      availableSkills={[]}
      availableWorkflows={[]}
      approvalRequests={[]}
      conversationCells={[]}
      conversationScrollRef={React.createRef<HTMLDivElement>()}
      draft=""
      draftImages={[]}
      draftSkills={[]}
      focusedConversationItem={null}
      goal={null}
      goalAction={null}
      goalActionError={null}
      imageInputRef={React.createRef<HTMLInputElement>()}
      isLoadingThread={false}
      isSending={false}
      isStoppingTurn={false}
      onAddDraftSkill={() => {}}
      onCancelGoal={() => {}}
      onConversationScroll={() => {}}
      onDraftChange={() => {}}
      onHandleComposerPaste={() => {}}
      onHandleImageSelection={() => {}}
      onOpenArtifactUrl={() => {}}
      onOpenLocalFile={() => {}}
      onPauseGoal={() => {}}
      onRemoveDraftImage={() => {}}
      onRemoveDraftSkill={() => {}}
      onRespondApproval={() => {}}
      onResumeGoal={() => {}}
      onRunSlashCommand={() => {}}
      onSendMessage={() => {}}
      onStopTurn={() => {}}
      onToggleVoiceCapture={() => {}}
      onUpdateRunConfig={() => {}}
      selectedThread={selectedThread}
      selectedThreadId={selectedThread?.id ?? null}
      voiceCaptureMessage={null}
      voiceCaptureStatus="idle"
      {...options}
    />,
  );
}

function findElementByAriaLabel(
  node: React.ReactNode,
  ariaLabel: string,
): React.ReactElement<ReactElementProps> | null {
  if (!React.isValidElement(node)) {
    return null;
  }
  const element = node as React.ReactElement<ReactElementProps>;
  if (element.props["aria-label"] === ariaLabel) {
    return element;
  }
  const children = React.Children.toArray(element.props.children);
  for (const child of children) {
    const found = findElementByAriaLabel(child, ariaLabel);
    if (found) {
      return found;
    }
  }
  return null;
}

function findElementByClassName(
  node: React.ReactNode,
  className: string,
): React.ReactElement<ReactElementProps> | null {
  if (!React.isValidElement(node)) {
    return null;
  }
  const element = node as React.ReactElement<ReactElementProps>;
  if (element.props.className === className) {
    return element;
  }
  const children = React.Children.toArray(element.props.children);
  for (const child of children) {
    const found = findElementByClassName(child, className);
    if (found) {
      return found;
    }
  }
  return null;
}

function assertSourceOrder(source: string, first: string, second: string) {
  assert.ok(source.indexOf(first) < source.indexOf(second));
}

test("SidebarPanel renders projects with nested subagents and no extra root row", () => {
  const root = makeNode(makeThread("root-alpha", "/work/alpha", "Alpha chat"), [
    makeNode(makeThread("owner-alpha", "/work/alpha", "owner_dev")),
  ]);
  const sidebar = makeSidebar([
    makeProject("project:/work/alpha", "alpha", root, {
      descendantCount: 1,
      activeCount: 1,
    }),
    makeProject(
      "project:/work/beta",
      "beta",
      makeNode(makeThread("root-beta", "/work/beta", "Beta chat")),
    ),
  ]);

  const markup = renderSidebar(sidebar);

  assert.match(markup, /Projects/);
  assert.match(markup, /sidebar-window-drag-strip/);
  assert.match(
    markup,
    /sidebar-actions[\s\S]*>New<[\s\S]*section-heading[\s\S]*<h2>Projects<\/h2>[\s\S]*2 projects · 0 chats/,
  );
  assert.match(markup, /alpha/);
  assert.match(markup, /beta/);
  assert.match(markup, /owner_dev/);
  assert.match(markup, /class="tree-node tree-node-root project-tree-root"/);
  assert.match(markup, /class="tree-node-button project-tree-button"/);
  assert.match(markup, /aria-expanded="true"/);
  assert.match(markup, /\/work\/alpha/);
  assert.doesNotMatch(markup, /Alpha chat/);
  assert.doesNotMatch(markup, /Beta chat/);
  assert.doesNotMatch(markup, /Agent Tree/);
  assert.doesNotMatch(markup, /New Root/);
});

test("ConversationPanel keeps thread metadata in the composer without placeholder buttons", () => {
  const markup = renderConversationPanel();

  assert.doesNotMatch(markup, /conversation-header/);
  assert.match(markup, /composer-metadata-row/);
  assert.doesNotMatch(markup, /composer-thread-title/);
  assert.doesNotMatch(markup, /status-dot/);
  assert.doesNotMatch(markup, /Complete/);
  assert.match(markup, /cwd: .*alpha/);
  assert.match(markup, /run-config-trigger/);
  assert.match(markup, /aria-label="Search conversation"/);
  assert.match(markup, /aria-label="Attach image"/);
  assert.match(markup, /aria-label="Start voice input"/);
  assert.match(markup, /aria-label="Send message"/);
  assert.doesNotMatch(markup, /aria-label="Attach file"/);
  assert.doesNotMatch(markup, /aria-label="Insert code"/);
  assert.doesNotMatch(markup, /aria-label="Open thread details"/);
  assert.doesNotMatch(markup, /aria-label="More thread actions"/);
});

test("SidebarPanel exposes the settings action", () => {
  const sidebar = makeSidebar();

  const markup = renderSidebar(sidebar);

  assert.match(markup, /aria-label="Open settings"/);
  assert.match(markup, /Settings/);
});

test("sidebar project lists avoid internal separators while preserving panel resizers", () => {
  const css = readFileSync(new URL("../styles.css", import.meta.url), "utf8");

  assert.match(
    css,
    /UI polish: source sidebars use fill, spacing, and type instead of hard internal separators\./,
  );
  assert.match(
    css,
    /\.sidebar-section-header,[\s\S]*\.sidebar-footer \{[\s\S]*border-top: 0;[\s\S]*border-bottom: 0;/,
  );
  assert.match(css, /\.sidebar \{[\s\S]*border-right: 0;/);
  assert.match(
    css,
    /\.conversation-scroll \{[\s\S]*background: #ffffff;[\s\S]*background-image: none;/,
  );
  assert.match(css, /\.tree-node::before \{[\s\S]*display: none;/);
  assert.match(css, /\.panel-resizer \{[\s\S]*background: transparent;/);
  assert.match(
    css,
    /\.panel-resizer::before \{[\s\S]*left: 50%;[\s\S]*width: 1px;[\s\S]*background: rgba\(16, 24, 40, 0\.08\);/,
  );
  assert.match(
    css,
    /\.is-resizing-panels \.panel-resizer::before \{[\s\S]*background: rgba\(217, 119, 6, 0\.68\);/,
  );
});

test("workspace keeps 4px resizers outside sidebar project-list polish", () => {
  const source = readFileSync(new URL("../App.tsx", import.meta.url), "utf8");

  assert.match(source, /const PANEL_RESIZER_WIDTH = 4/);
  assert.match(
    source,
    /gridTemplateColumns: `\$\{sidebarWidth\}px \$\{PANEL_RESIZER_WIDTH\}px minmax\(0, 1fr\) \$\{PANEL_RESIZER_WIDTH\}px/,
  );
});

test("SidebarPanel indents project subagents relative to the project header", () => {
  const owner = makeNode(
    makeSubagentThread("owner-alpha", "/work/alpha", "owner_dev", "root-alpha"),
    [
      makeNode(
        makeSubagentThread(
          "reviewer-alpha",
          "/work/alpha",
          "reviewer",
          "owner-alpha",
        ),
      ),
    ],
  );
  const root = makeNode(makeThread("root-alpha", "/work/alpha", "Alpha chat"), [
    owner,
  ]);
  const sidebar = makeSidebar(
    [makeProject("project:/work/alpha", "alpha", root)],
    { conversations: [makeNode(makeThread("chat-1", "", "General Q&A"))] },
  );

  const markup = renderSidebar(sidebar);

  assert.match(markup, /class="tree-node " style="--depth:1"[\s\S]*owner_dev/);
  assert.match(markup, /class="tree-node " style="--depth:2"[\s\S]*reviewer/);
  assert.match(markup, /class="chat-list-row"[\s\S]*General Q&amp;A/);
  assert.match(markup, /aria-label="Delete chat General Q&amp;A"/);
  assert.doesNotMatch(markup, /tree-node-copy"><strong>General Q&amp;A/);
});

test("SidebarPanel keeps collapsed project children hidden across sidebar updates", () => {
  const child = makeNode(makeThread("owner-alpha", "/work/alpha", "owner_dev"));
  const root = makeNode(makeThread("root-alpha", "/work/alpha", "Alpha chat"), [
    child,
  ]);
  const baseSidebar = makeSidebar([
    makeProject("project:/work/alpha", "alpha", root),
  ]);
  const updatedSidebar: ProjectAgentSidebar = {
    ...baseSidebar,
    projects: [
      makeProject("project:/work/alpha", "alpha", root, {
        activeCount: 1,
        waitingCount: 1,
        descendantCount: 1,
      }),
    ],
  };

  const baseMarkup = renderSidebar(baseSidebar, {
    collapsedProjects: ["project:/work/alpha"],
  });
  const updatedMarkup = renderSidebar(updatedSidebar, {
    collapsedProjects: ["project:/work/alpha"],
  });

  assert.match(baseMarkup, /aria-expanded="false"/);
  assert.doesNotMatch(baseMarkup, /owner_dev/);
  assert.match(updatedMarkup, /aria-expanded="false"/);
  assert.match(updatedMarkup, /active/);
  assert.match(updatedMarkup, /waiting/);
  assert.doesNotMatch(updatedMarkup, /owner_dev/);
});

test("SidebarPanel exposes project create and chat quick create", () => {
  const sidebar = makeSidebar();

  const markup = renderSidebar(sidebar);

  assert.match(markup, /New/);
  assert.match(markup, /aria-label="New chat"/);
  assert.match(markup, /class="chat-create-button"/);
  assert.doesNotMatch(markup, /New Chat/);
  assert.doesNotMatch(markup, /Open Project/);
  assert.doesNotMatch(markup, /id="new-thread-popover"/);
});

test("SidebarPanel disables chat quick create while a chat is being created", () => {
  const sidebar = makeSidebar();

  const markup = renderSidebar(sidebar, { isCreatingChatThread: true });

  assert.match(markup, /class="chat-create-button"/);
  assert.match(markup, /aria-label="New chat"/);
  assert.match(markup, /disabled=""/);
});

test("buildBlankChatThreadDraft creates a cwd-free chat draft", () => {
  assert.deepEqual(buildBlankChatThreadDraft(), BLANK_THREAD_DRAFT);
});

test("NewThreadPopover renders thread/start parameter fields", () => {
  const markup = renderNewThreadPopover();

  assert.match(markup, /New conversation/);
  assert.match(markup, /Project path/);
  assert.match(markup, /aria-label="Choose project folder"/);
  assert.doesNotMatch(markup, /<span>Title<\/span>/);
  assert.match(markup, /taskName/);
  assert.match(markup, /Path preview/);
  assert.match(markup, /threadProvider/);
  assert.match(markup, /agentType/);
  assert.match(markup, /modelProvider/);
  assert.match(markup, /<select>/);
  assert.match(markup, /Use default/);
  assert.match(markup, /reasoningEffort/);
  assert.match(markup, /serviceTier/);
  assert.match(markup, /chats without a project stay in Chat/);
  assert.match(markup, /Chat without project/);
  assert.doesNotMatch(markup, /value="chat" disabled=""/);
});

test("NewThreadDialog renders centered overlay around the existing form", () => {
  const markup = renderNewThreadDialog();

  assert.match(markup, /class="new-thread-dialog-layer"/);
  assert.match(markup, /class="new-thread-dialog-shell"/);
  assert.match(markup, /role="dialog"/);
  assert.match(markup, /aria-modal="true"/);
  assert.match(markup, /id="new-thread-popover"/);
  assert.match(markup, /Project path/);
  assert.match(markup, /taskName/);
  assert.match(markup, /Path preview/);
});

test("buildNewThreadDraft trims thread start params", () => {
  assert.deepEqual(
    buildNewThreadDraft("project", "  /work/new  ", {
      taskName: "  owner_dev  ",
      threadProvider: "  native  ",
      agentType: "  feature-owner  ",
      model: "  gpt-5.4  ",
      modelProvider: "  openai  ",
      reasoningEffort: "  high  ",
      serviceTier: "  priority  ",
    }),
    {
      mode: "project",
      projectPath: "/work/new",
      taskName: "owner_dev",
      threadProvider: "native",
      agentType: "feature-owner",
      model: "gpt-5.4",
      modelProvider: "openai",
      reasoningEffort: "high",
      serviceTier: "priority",
    },
  );
  assert.equal(
    buildNewThreadDraft("project", "/work/new").taskName,
    defaultNewThreadStartParams("/work/new").taskName,
  );
  assert.equal(
    buildNewThreadDraft("project", "/work/new", {
      model: "   ",
    }).model,
    null,
  );
  assert.deepEqual(
    buildNewThreadDraft("project", "/work/new", {
      taskName: " ",
    }),
    { ...BLANK_THREAD_DRAFT, mode: "project", projectPath: "/work/new" },
  );
});

test("new thread provider controls gate external model and role fields", () => {
  const nativeControls = resolveNewThreadProviderControls({
    fallbackAgentTypes: [{ name: "feature-owner", builtIn: false }],
    selectedThreadProvider: null,
  });
  assert.equal(nativeControls.effectiveThreadProvider, "native");
  assert.equal(nativeControls.canSelectModel, true);
  assert.equal(nativeControls.canStartThread, true);
  assert.deepEqual(nativeControls.agentTypes, [
    { name: "feature-owner", builtIn: false },
  ]);

  const externalControls = resolveNewThreadProviderControls({
    fallbackAgentTypes: [{ name: "feature-owner", builtIn: false }],
    selectedThreadProvider: {
      id: "claude_cli",
      displayName: "Claude Code",
      kind: "externalCli",
      description: "External Claude CLI",
      agentTypes: [],
      modelSelection: {
        mode: "providerDefault",
        modelProviders: [],
      },
      capabilities: {
        startThread: false,
        sendInput: true,
        closeThread: true,
        listChildren: true,
        restoreThread: false,
        restoreSnapshot: true,
        eventStream: true,
        spawnChild: true,
        compact: false,
        workflow: false,
        pollEvent: false,
        commandSession: false,
        permissions: false,
        dynamicTools: false,
      },
    },
  });
  assert.equal(externalControls.effectiveThreadProvider, "claude_cli");
  assert.equal(externalControls.canSelectModel, false);
  assert.equal(externalControls.canStartThread, false);
  assert.deepEqual(externalControls.agentTypes, []);
  assert.deepEqual(externalControls.modelProviders, []);
});

test("new thread agent type selection is scoped to the selected project cwd", () => {
  const sourceWorkspaceAgentTypes = [
    { name: "code-review", builtIn: false },
    { name: "project-pm", builtIn: false },
  ];

  assert.deepEqual(
    resolveNewThreadProjectScopedOptions({
      options: sourceWorkspaceAgentTypes,
      optionsProjectPath: "/Users/bytedance/.morpheus/source_workspace",
      projectPath: "/telebot",
    }),
    [],
  );
  assert.deepEqual(
    resolveNewThreadProjectScopedOptions({
      options: [{ name: "telebot-owner", builtIn: false }],
      optionsProjectPath: "/telebot",
      projectPath: " /telebot ",
    }),
    [{ name: "telebot-owner", builtIn: false }],
  );
  assert.equal(
    resolveNewThreadAgentTypeSelection({
      agentType: "project-pm",
      effectiveThreadProvider: "native",
      providerAgentTypes: sourceWorkspaceAgentTypes,
    }),
    "project-pm",
  );
  assert.equal(
    resolveNewThreadAgentTypeSelection({
      agentType: "shared-owner",
      effectiveThreadProvider: "native",
      providerAgentTypes: [{ name: "shared-owner", builtIn: false }],
    }),
    "shared-owner",
  );
  assert.equal(
    resolveNewThreadAgentTypeSelection({
      agentType: "project-pm",
      effectiveThreadProvider: "native",
      providerAgentTypes: [{ name: "telebot-owner", builtIn: false }],
    }),
    "",
  );
  assert.equal(
    resolveNewThreadAgentTypeSelection({
      agentType: "telebot-owner",
      effectiveThreadProvider: "codex_cli",
      providerAgentTypes: [{ name: "telebot-owner", builtIn: false }],
    }),
    "",
  );
});

test("new thread project path changes clear cwd-scoped provider and role option lists before reloading", () => {
  const source = readFileSync(new URL("./Panels.tsx", import.meta.url), "utf8");
  const effectStart = source.indexOf(
    "  useEffect(() => {\n    setThreadProviders([]);",
  );
  const effectEnd = source.indexOf("  }, [trimmedProjectPath]);", effectStart);
  const effectSource = source.slice(effectStart, effectEnd);

  assert.notEqual(effectStart, -1);
  assert.notEqual(effectEnd, -1);
  assertSourceOrder(
    effectSource,
    "setThreadProviders([]);",
    ".listThreadProviders(trimmedProjectPath)",
  );
  assertSourceOrder(
    effectSource,
    'setThreadProvidersProjectPath("");',
    ".listThreadProviders(trimmedProjectPath)",
  );
  assertSourceOrder(
    effectSource,
    "setAgentTypes([]);",
    ".listAgentTypes(trimmedProjectPath)",
  );
  assertSourceOrder(
    effectSource,
    'setAgentTypesProjectPath("");',
    ".listAgentTypes(trimmedProjectPath)",
  );
});

test("new thread agent path validation matches backend path rules", () => {
  for (const [segment, expected] of [
    ["owner_dev", true],
    ["root", false],
    ["OwnerDev", false],
    ["owner-dev", false],
  ] as const) {
    assert.equal(isValidAgentPathSegment(segment), expected);
  }

  for (const [path, expected] of [
    ["/root", true],
    ["/root/owner_dev", true],
    ["/morpheus", true],
    ["/morpheus/agent_1", true],
    ["/project", true],
    ["/project/owner", true],
    ["/Project", false],
    ["/root/root", false],
    ["/root/owner/root", false],
    ["/root/owner/", false],
    ["/", false],
  ] as const) {
    assert.equal(isValidNewThreadAgentPath(path), expected);
  }
});

test("new thread project defaults use sanitized cwd basename paths", () => {
  const alpha = defaultNewThreadStartParams("/work/alpha-project");
  const beta = defaultNewThreadStartParams("/work/beta-project");

  assert.equal(alpha.taskName, "alpha_project");
  assert.equal(alpha.pathPreview, "/alpha_project");
  assert.equal(beta.taskName, "beta_project");
  assert.equal(beta.pathPreview, "/beta_project");
  assert.notEqual(alpha.taskName, beta.taskName);
  assert.equal(
    defaultNewThreadStartParams("/work/alpha-project").taskName,
    alpha.taskName,
  );
});

test("new thread defaults preserve manual task path edits", () => {
  const manual = resolveNewThreadStartParamsForProject({
    currentTaskName: "custom_project",
    hasManualThreadStartParams: true,
    projectPath: "/work/other",
  });
  assert.deepEqual(manual, {
    taskName: "custom_project",
    pathPreview: "/custom_project",
  });
  assert.deepEqual(
    resolveNewThreadStartParamsForProject({
      currentTaskName: "",
      hasManualThreadStartParams: true,
      projectPath: "/work/other",
    }),
    {
      taskName: "",
      pathPreview: "",
    },
  );

  const automatic = resolveNewThreadStartParamsForProject({
    currentTaskName: "old_project",
    hasManualThreadStartParams: false,
    projectPath: "/work/new-project",
  });
  assert.deepEqual(automatic, {
    taskName: "new_project",
    pathPreview: "/new_project",
  });
});

test("SidebarPanel renders Chat group conversations separately", () => {
  const activeChat = {
    ...makeThread("chat-1", "", "Explain thread status updates"),
    lifecycleStatus: { type: "active", activeFlags: ["running"] },
  } satisfies Thread;
  const waitingChat = {
    ...makeThread("chat-2", "", "API question"),
    lifecycleStatus: { type: "waiting", reason: "eventSubscription" },
  } satisfies Thread;
  const sidebar = makeSidebar([], {
    updatedAt: 2,
    conversations: [makeNode(activeChat), makeNode(waitingChat)],
  });

  const markup = renderSidebar(sidebar);

  assert.match(markup, /Chat/);
  assert.match(markup, /Explain thread status updates/);
  assert.match(markup, /API question/);
  assert.match(markup, /class="chat-list-row-shell"/);
  assert.match(
    markup,
    /class="chat-list-row"[\s\S]*tree-inline-status chat-inline-status doing/,
  );
  assert.match(
    markup,
    /class="chat-list-row"[\s\S]*tree-inline-status chat-inline-status waiting-subscription/,
  );
  assert.match(markup, /aria-label="Active"/);
  assert.match(markup, /aria-label="Waiting on subscription"/);
  assert.match(
    markup,
    /aria-label="Delete chat Explain thread status updates"/,
  );
  assert.match(markup, /aria-label="Delete chat API question"/);
  assert.match(markup, /No projects yet/);
});

test("SidebarPanel exposes project delete only on project roots", () => {
  const root = makeNode(makeThread("root-alpha", "/work/alpha", "Alpha chat"), [
    makeNode(makeThread("owner-alpha", "/work/alpha", "owner_dev")),
  ]);
  const sidebar = makeSidebar(
    [makeProject("project:/work/alpha", "alpha", root)],
    { conversations: [makeNode(makeThread("chat-1", "", "General Q&A"))] },
  );

  const markup = renderSidebar(sidebar);

  assert.match(markup, /aria-label="Delete project alpha"/);
  assert.match(markup, /aria-label="Delete chat General Q&amp;A"/);
  assert.doesNotMatch(markup, /Delete project owner_dev/);
  assert.doesNotMatch(markup, /Delete project General Q&amp;A/);
  assert.doesNotMatch(markup, /Delete chat owner_dev/);
  assert.doesNotMatch(markup, /Delete chat Alpha chat/);
});

test("ProjectSection delete calls project archive without selecting the row", () => {
  const root = makeNode(makeThread("root-alpha", "/work/alpha", "Alpha chat"), [
    makeNode(makeThread("owner-alpha", "/work/alpha", "owner_dev")),
  ]);
  const project = makeProject("project:/work/alpha", "alpha", root);
  const archivedThreadIds: string[] = [];
  const selectedThreadIds: string[] = [];
  let stoppedPropagation = false;
  const element = ProjectSection({
    collapsedProjectSet: new Set(),
    collapsedSet: new Set(),
    onArchiveProjectThread: (threadId: string) =>
      archivedThreadIds.push(threadId),
    onOpenMenu: () => {},
    onSelectProject: (_projectId: string, threadId: string) =>
      selectedThreadIds.push(threadId),
    onSelectThread: () => {},
    onToggleProject: () => {},
    onToggleTreeNode: () => {},
    project,
    selectedThreadId: null,
  });
  const deleteButton = findElementByAriaLabel(element, "Delete project alpha");

  assert.ok(deleteButton);
  const onClick = deleteButton.props.onClick as (event: {
    stopPropagation: () => void;
  }) => void;
  onClick({
    stopPropagation: () => {
      stoppedPropagation = true;
    },
  });

  assert.deepEqual(archivedThreadIds, ["root-alpha"]);
  assert.deepEqual(selectedThreadIds, []);
  assert.equal(stoppedPropagation, true);
});

test("TreeContextMenu routes project root deletion to the project archive callback", () => {
  const root = makeThread("root-alpha", "/work/alpha", "Alpha chat");
  const child = makeSubagentThread(
    "owner-alpha",
    "/work/alpha",
    "owner_dev",
    "root-alpha",
  );
  const archivedProjectThreadIds: string[] = [];
  const archivedAgentThreadIds: string[] = [];
  const element = TreeContextMenu({
    threads: [root, child],
    treeMenu: { threadId: "root-alpha", kind: "project", x: 10, y: 20 },
    onArchiveProjectThread: (threadId: string) =>
      archivedProjectThreadIds.push(threadId),
    onArchiveThread: (threadId: string) =>
      archivedAgentThreadIds.push(threadId),
  });
  const markup = renderToStaticMarkup(element);

  assert.match(markup, /Delete Project Tree/);
  const menuItem = findElementByClassName(
    element,
    "tree-context-menu-item danger",
  );
  assert.ok(menuItem);
  const onClick = menuItem.props.onClick as () => void;
  onClick();

  assert.deepEqual(archivedProjectThreadIds, ["root-alpha"]);
  assert.deepEqual(archivedAgentThreadIds, []);
});

test("SidebarPanel keeps chat as a flat list outside tree collapse", () => {
  const child = makeNode(
    makeThread("owner-alpha", "/work/alpha", "owner_dev"),
    [makeNode(makeThread("reviewer-alpha", "/work/alpha", "reviewer"))],
  );
  const root = makeNode(makeThread("root-alpha", "/work/alpha", "Alpha chat"), [
    child,
  ]);
  const sidebar = makeSidebar(
    [makeProject("project:/work/alpha", "alpha", root)],
    { conversations: [makeNode(makeThread("chat-1", "", "General Q&A"))] },
  );

  const projectCollapsed = renderSidebar(sidebar, {
    collapsedProjects: ["project:/work/alpha"],
  });
  const treeCollapsed = renderSidebar(sidebar, {
    collapsedTreeNodes: ["owner-alpha"],
  });
  const selectedChat = renderSidebar(sidebar, { selectedThreadId: "chat-1" });

  assert.doesNotMatch(projectCollapsed, /owner_dev/);
  assert.match(treeCollapsed, /owner_dev/);
  assert.doesNotMatch(treeCollapsed, /reviewer/);
  assert.match(treeCollapsed, /Chat/);
  assert.match(treeCollapsed, /General Q&amp;A/);
  assert.match(selectedChat, /class="chat-list-row-shell selected"/);
  assert.match(selectedChat, /data-thread-id="chat-1"/);
  assert.doesNotMatch(selectedChat, /tree-node-copy"><strong>General Q&amp;A/);
});
