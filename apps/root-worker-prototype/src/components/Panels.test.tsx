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
const { SidebarPanel } = await import("./Panels");

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
  pmTree: TreeNode,
  overrides: Partial<SidebarProjectNode> = {},
): SidebarProjectNode {
  return {
    id,
    label,
    subtitle: `/work/${label}`,
    cwd: `/work/${label}`,
    statusClass: "todo",
    updatedAt: 1,
    pmTree,
    descendantCount: 0,
    activeCount: 0,
    waitingCount: 0,
    failedCount: 0,
    duplicatePmThreadIds: [],
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
      newProjectName="PM"
      onCreateChatThread={() => {}}
      onCreateProjectThread={() => {}}
      onOpenMenu={() => {}}
      onSelectProjectPm={() => {}}
      onSelectThread={() => {}}
      onSetNewProjectName={() => {}}
      onToggleChat={() => {}}
      onToggleProject={() => {}}
      onToggleTreeNode={() => {}}
      projectSidebar={sidebar}
      selectedThreadId={options?.selectedThreadId ?? null}
    />,
  );
}

test("SidebarPanel renders projects with PM and nested subagents", () => {
  const pm = makeNode(makeThread("pm-alpha", "/work/alpha", "PM"), [
    makeNode(makeThread("owner-alpha", "/work/alpha", "owner_dev")),
  ]);
  const sidebar: ProjectAgentSidebar = {
    projects: [
      makeProject("project:/work/alpha", "alpha", pm, {
        descendantCount: 1,
        activeCount: 1,
      }),
      makeProject(
        "project:/work/beta",
        "beta",
        makeNode(makeThread("pm-beta", "/work/beta", "PM")),
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
  assert.match(markup, /PM/);
  assert.match(markup, /owner_dev/);
  assert.doesNotMatch(markup, /Agent Tree/);
  assert.doesNotMatch(markup, /New Root/);
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
  const child = makeNode(makeThread("owner-alpha", "/work/alpha", "owner_dev"));
  const pm = makeNode(makeThread("pm-alpha", "/work/alpha", "PM"), [child]);
  const sidebar: ProjectAgentSidebar = {
    projects: [makeProject("project:/work/alpha", "alpha", pm)],
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
    collapsedTreeNodes: ["pm-alpha"],
  });
  const chatCollapsed = renderSidebar(sidebar, { chatCollapsed: true });

  assert.doesNotMatch(projectCollapsed, /owner_dev/);
  assert.doesNotMatch(treeCollapsed, /owner_dev/);
  assert.match(treeCollapsed, /Chat/);
  assert.doesNotMatch(chatCollapsed, /General Q&amp;A/);
  assert.match(chatCollapsed, /alpha/);
});
