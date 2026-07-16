import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import type {
  ProjectAgentSidebar,
  SidebarProjectNode,
  Thread,
  TreeNode,
} from "../types";

(globalThis as typeof globalThis & { React: typeof React }).React = React;
const { NewThreadPopover, SidebarPanel, buildNewThreadDraft } =
  await import("./Panels");

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
    status: { type: "complete" },
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

function renderSidebar(
  sidebar: ProjectAgentSidebar,
  options?: {
    collapsedProjects?: string[];
    chatCollapsed?: boolean;
    selectedThreadId?: string | null;
    collapsedTreeNodes?: string[];
  },
) {
  return renderToStaticMarkup(
    <SidebarPanel
      collapsedProjectSet={new Set(options?.collapsedProjects ?? [])}
      collapsedSet={new Set(options?.collapsedTreeNodes ?? [])}
      isChatCollapsed={options?.chatCollapsed ?? false}
      newProjectName="Project chat"
      onCreateProjectThread={() => {}}
      onOpenMenu={() => {}}
      onSelectProject={() => {}}
      onSelectThread={() => {}}
      onSetNewProjectName={() => {}}
      onSubmitNewThreadDraft={() => {}}
      onToggleChat={() => {}}
      onToggleProject={() => {}}
      onToggleTreeNode={() => {}}
      projectSidebar={sidebar}
      selectedThreadId={options?.selectedThreadId ?? null}
      workspacePath="/work/alpha"
    />,
  );
}

test("SidebarPanel renders projects with nested subagents and no extra root row", () => {
  const root = makeNode(makeThread("root-alpha", "/work/alpha", "Alpha chat"), [
    makeNode(makeThread("owner-alpha", "/work/alpha", "owner_dev")),
  ]);
  const sidebar: ProjectAgentSidebar = {
    projects: [
      makeProject("project:/work/alpha", "alpha", root, {
        descendantCount: 1,
        activeCount: 1,
      }),
      makeProject(
        "project:/work/beta",
        "beta",
        makeNode(makeThread("root-beta", "/work/beta", "Beta chat")),
      ),
    ],
    chat: {
      id: "chat",
      statusClass: "todo",
      updatedAt: 0,
      conversations: [],
    },
  };

  const markup = renderSidebar(sidebar);

  assert.match(markup, /Projects/);
  assert.match(markup, /alpha/);
  assert.match(markup, /beta/);
  assert.match(markup, /owner_dev/);
  assert.doesNotMatch(markup, /Alpha chat/);
  assert.doesNotMatch(markup, /Beta chat/);
  assert.doesNotMatch(markup, /Agent Tree/);
  assert.doesNotMatch(markup, /New Root/);
});

test("SidebarPanel exposes one create button", () => {
  const sidebar: ProjectAgentSidebar = {
    projects: [],
    chat: {
      id: "chat",
      statusClass: "todo",
      updatedAt: 0,
      conversations: [],
    },
  };

  const markup = renderSidebar(sidebar);

  assert.match(markup, /New/);
  assert.doesNotMatch(markup, /New Chat/);
  assert.doesNotMatch(markup, /Open Project/);
});

test("NewThreadPopover renders project path, agent type, and gated model controls", () => {
  const markup = renderToStaticMarkup(
    <NewThreadPopover
      existingProjectPaths={["/work/alpha", "/work/beta"]}
      onCancel={() => {}}
      onSubmit={() => {}}
      workspacePath="/work/alpha"
    />,
  );

  assert.match(markup, /New conversation/);
  assert.match(markup, /Project path/);
  assert.match(markup, /Agent type/);
  assert.match(markup, /Feature owner - after backend support/);
  assert.match(markup, /Model/);
  assert.match(markup, /Reasoning/);
  assert.match(markup, /Use current default/);
  assert.match(markup, /No-project chat needs backend support/);
  assert.match(markup, /Chat without project/);
  assert.match(markup, /disabled=""/);
});

test("buildNewThreadDraft trims fields and preserves unsupported params as defaults", () => {
  assert.deepEqual(
    buildNewThreadDraft("project", "  /work/new  ", "  Launch pad  "),
    {
      mode: "project",
      projectPath: "/work/new",
      title: "Launch pad",
      agentType: "project-chat",
      model: "current-default",
      reasoningEffort: "current-default",
    },
  );
  assert.equal(
    buildNewThreadDraft("project", "/work/new", "   ").title,
    "Project chat",
  );
});

test("SidebarPanel renders Chat group conversations separately", () => {
  const sidebar: ProjectAgentSidebar = {
    projects: [],
    chat: {
      id: "chat",
      statusClass: "todo",
      updatedAt: 2,
      conversations: [
        makeNode(makeThread("chat-1", "", "General Q&A")),
        makeNode(makeThread("chat-2", "", "API question")),
      ],
    },
  };

  const markup = renderSidebar(sidebar);

  assert.match(markup, /Chat/);
  assert.match(markup, /General Q&amp;A/);
  assert.match(markup, /API question/);
  assert.match(markup, /No projects yet/);
});

test("SidebarPanel keeps project and tree collapse independent", () => {
  const child = makeNode(makeThread("owner-alpha", "/work/alpha", "owner_dev"), [
    makeNode(makeThread("reviewer-alpha", "/work/alpha", "reviewer")),
  ]);
  const root = makeNode(makeThread("root-alpha", "/work/alpha", "Alpha chat"), [
    child,
  ]);
  const sidebar: ProjectAgentSidebar = {
    projects: [makeProject("project:/work/alpha", "alpha", root)],
    chat: {
      id: "chat",
      statusClass: "todo",
      updatedAt: 0,
      conversations: [makeNode(makeThread("chat-1", "", "General Q&A"))],
    },
  };

  const projectCollapsed = renderSidebar(sidebar, {
    collapsedProjects: ["project:/work/alpha"],
  });
  const treeCollapsed = renderSidebar(sidebar, {
    collapsedTreeNodes: ["owner-alpha"],
  });
  const chatCollapsed = renderSidebar(sidebar, { chatCollapsed: true });

  assert.doesNotMatch(projectCollapsed, /owner_dev/);
  assert.match(treeCollapsed, /owner_dev/);
  assert.doesNotMatch(treeCollapsed, /reviewer/);
  assert.match(treeCollapsed, /Chat/);
  assert.doesNotMatch(chatCollapsed, /General Q&amp;A/);
  assert.match(chatCollapsed, /alpha/);
});
