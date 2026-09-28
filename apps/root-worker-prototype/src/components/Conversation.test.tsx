import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import {
  ApprovalRequestsPanel,
  ArchivedHistoryRow,
  ArtifactRow,
  CompactRow,
  MessageRow,
  ToolRow,
  TERMINAL_OUTPUT_PREVIEW_RESET_SEQUENCE,
  planTerminalOutputPreviewWrite,
} from "./Conversation";
import {
  ConversationVirtualList,
  buildConversationRowClassName,
  conversationCellInstanceKey,
  planConversationCellMeasurement,
  planFocusedItemScrollAttempt,
  shouldHandleFocusedItemRequest,
} from "./ConversationVirtualList";
import {
  buildConversationEntries,
  buildConversationState,
} from "../lib/conversation";
import { normalizeThreadSnapshot } from "../lib/thread";
import type { ConversationEntry } from "../types";
import type { Thread } from "../types";

function artifactEntry(
  overrides: Partial<NonNullable<ConversationEntry["artifact"]>> = {},
): ConversationEntry {
  const artifact = {
    title: "Inline artifact",
    mimeType: "text/html",
    content: "<main><h1>Hello</h1><script>window.bad = true</script></main>",
    ...overrides,
  };
  artifact.source ??= {
    type: "inline",
    content: artifact.content,
    mimeType: artifact.mimeType,
    language: artifact.language,
    truncated: artifact.truncated,
  };
  return {
    id: "artifact-1",
    kind: "artifact",
    author: "root",
    role: "agent",
    text: `${artifact.title} • ${artifact.mimeType}`,
    timestamp: "09:41",
    attachments: [],
    artifact,
  };
}

test("approval request panel renders pending command approval actions", () => {
  const markup = renderToStaticMarkup(
    <ApprovalRequestsPanel
      requests={[
        {
          requestId: 7,
          kind: "commandExecution",
          threadId: "thread-1",
          turnId: "turn-1",
          itemId: "cmd-1",
          startedAtMs: 1_725_000_000_000,
          reason: "Needs network access",
          title: "Command approval",
          detail: "rtk pnpm build",
          metadata: [
            { label: "cwd", value: "/tmp/project" },
            { label: "network", value: "enabled" },
          ],
          status: "pending",
          error: null,
          availableDecisions: [
            "accept",
            "acceptForSession",
            "decline",
            "cancel",
          ],
        },
      ]}
      onRespond={() => {}}
    />,
  );

  assert.match(markup, /Pending approvals/);
  assert.match(markup, /Command approval/);
  assert.match(markup, /rtk pnpm build/);
  assert.match(markup, /Needs network access/);
  assert.match(markup, /Approve/);
  assert.match(markup, /Session/);
  assert.match(markup, /Deny/);
  assert.match(markup, /Cancel/);
});

test("approval request panel disables actions while submitting", () => {
  const markup = renderToStaticMarkup(
    <ApprovalRequestsPanel
      requests={[
        {
          requestId: "request-1",
          kind: "permissions",
          threadId: "thread-1",
          turnId: "turn-1",
          itemId: "permissions-1",
          startedAtMs: 1_725_000_000_000,
          reason: null,
          title: "Permissions request",
          detail: "Grant the requested runtime permissions.",
          metadata: [{ label: "write paths", value: "/tmp/project" }],
          permissions: { fileSystem: { write: ["/tmp/project"] } },
          status: "submitting",
          error: null,
          availableDecisions: ["accept", "decline"],
        },
      ]}
      onRespond={() => {}}
    />,
  );

  assert.match(markup, /Submitting/);
  assert.match(markup, /Grant/);
  assert.match(markup, /disabled=""/);
});

test("tool row renders command summary without command output payload", () => {
  const markup = renderToStaticMarkup(
    <ToolRow
      entries={[
        {
          id: "cmd-1",
          kind: "tool",
          author: "root",
          role: "system",
          text: "tmp/project • exit 0",
          timestamp: "09:41",
          attachments: [],
          toolName: "npm test",
          toolStatus: "completed",
          toolDetails:
            "Command\nnpm test\n\nCwd\n/tmp/project\n\nStatus\ncompleted",
          toolCategory: "command",
        },
      ]}
      isOpen
    />,
  );

  assert.match(markup, /command-timeline-row/);
  assert.match(markup, /command-timeline-card/);
  assert.match(markup, /npm test/);
  assert.match(markup, /tmp\/project/);
  assert.match(markup, /Status/);
  assert.doesNotMatch(markup, /SECRET_STDOUT/);
  assert.doesNotMatch(markup, /tool-output-block/);
});

test("tool row renders an init context section entry", () => {
  const markup = renderToStaticMarkup(
    <ToolRow
      entries={[
        {
          id: "ctx-1",
          kind: "tool",
          author: "root",
          role: "system",
          text: "# AGENTS.md instructions Use Chinese.",
          timestamp: "09:41",
          attachments: [],
          toolName: "Init Context · AGENTS.md",
          toolStatus: "completed",
          toolDetails: "AGENTS.md\n# AGENTS.md instructions\nUse Chinese.",
          toolCategory: "context",
        },
      ]}
      isOpen
    />,
  );

  assert.match(markup, /Init Context · AGENTS\.md/);
  assert.match(markup, /# AGENTS\.md instructions Use Chinese\./);
  assert.match(markup, /AGENTS\.md/);
  assert.match(markup, /# AGENTS\.md instructions/);
  assert.doesNotMatch(markup, /tool-detail-sections/);
});

test("conversation renders live active command anchor without active output tail", () => {
  const thread = {
    id: "thread-1",
    updatedAt: 1,
    turns: [],
    activeCommandItems: [
      {
        type: "commandExecution",
        id: "cmd-live",
        command: "cargo test",
        cwd: "/tmp/project",
        status: "running",
        initialWaitMs: 1000,
        notifyOn: "exit",
        aggregatedOutput: "ACTIVE_STDOUT",
        exitCode: null,
        durationMs: null,
      },
    ],
  } as Thread;
  const entries = buildConversationEntries(thread);

  assert.deepEqual(
    entries.map((entry) => ({
      id: entry.id,
      kind: entry.kind,
      toolName: entry.toolName,
      toolStatus: entry.toolStatus,
      toolOutput: entry.toolOutput,
      hasActiveOutputInDetails: entry.toolDetails?.includes("ACTIVE_STDOUT"),
    })),
    [
      {
        id: "cmd-live",
        kind: "tool",
        toolName: "cargo test",
        toolStatus: "running",
        toolOutput: undefined,
        hasActiveOutputInDetails: false,
      },
    ],
  );
});

const entries: ConversationEntry[] = [
  {
    id: "tool-1",
    kind: "tool",
    author: "root",
    role: "system",
    text: "first summary",
    timestamp: "09:41",
    attachments: [],
    toolName: "first tool",
    toolStatus: "completed",
    toolDetails: "first details",
    toolCategory: "eventDrivenSubscription",
  },
  {
    id: "tool-2",
    kind: "tool",
    author: "root",
    role: "system",
    text: "second summary",
    timestamp: "09:42",
    attachments: [],
    toolName: "second tool",
    toolStatus: "completed",
    toolDetails: "second details",
    toolCategory: "eventDrivenEvent",
  },
];

test("tool rows show compact lists first and only reveal the selected detail body", () => {
  const listMarkup = renderToStaticMarkup(
    <ToolRow entries={entries} isOpen selectedEntryId={null} />,
  );

  assert.match(listMarkup, /tool-card-list/);
  assert.match(listMarkup, /2\/2 done/);
  assert.match(listMarkup, /tool-status-badge done/);
  assert.doesNotMatch(listMarkup, /first details/);
  assert.doesNotMatch(listMarkup, /second details/);

  const detailMarkup = renderToStaticMarkup(
    <ToolRow entries={entries} isOpen selectedEntryId="tool-2" />,
  );

  assert.match(detailMarkup, /second details/);
  assert.doesNotMatch(detailMarkup, /first details/);
  assert.match(detailMarkup, /second summary[\s\S]*second details/);
  assert.match(detailMarkup, /tool-card-item-head selected/);
});

test("message rows expose role classes for chat alignment", () => {
  const userEntry: ConversationEntry = {
    id: "user-1",
    kind: "message",
    author: "You",
    role: "user",
    text: "please update the layout",
    timestamp: "09:40",
    attachments: [],
  };
  const agentEntry: ConversationEntry = {
    id: "agent-1",
    kind: "message",
    author: "Codex",
    role: "agent",
    text: "layout updated",
    timestamp: "09:41",
    attachments: [],
  };

  const userMarkup = renderToStaticMarkup(<MessageRow entries={[userEntry]} />);
  const agentMarkup = renderToStaticMarkup(
    <MessageRow entries={[agentEntry]} />,
  );

  assert.match(userMarkup, /class="message-row message-row-user"/);
  assert.match(userMarkup, /class="message-avatar user"/);
  assert.match(userMarkup, /data-conversation-row="message"/);
  assert.match(userMarkup, /data-conversation-role="user"/);
  assert.match(userMarkup, /data-conversation-entry-ids="user-1"/);
  assert.match(agentMarkup, /class="message-row message-row-agent"/);
  assert.match(agentMarkup, /class="message-avatar agent"/);
  assert.match(agentMarkup, /data-conversation-role="agent"/);
});

test("tool rows expose semantic attributes for display diagnostics", () => {
  const markup = renderToStaticMarkup(
    <ToolRow
      entries={[
        {
          id: "tool-1",
          kind: "tool",
          author: "Codex",
          role: "system",
          text: "tmp/project • exit 0",
          timestamp: "09:42",
          attachments: [],
          toolName: "npm test",
          toolStatus: "completed",
          toolCategory: "command",
        },
      ]}
    />,
  );

  assert.match(markup, /class="[^"]*\btool-row\b[^"]*"/);
  assert.match(markup, /class="[^"]*\btool-row-command\b[^"]*"/);
  assert.match(markup, /data-conversation-row="tool"/);
  assert.match(markup, /data-tool-category="command"/);
  assert.match(markup, /data-conversation-entry-ids="tool-1"/);
});

test("inter-agent tool rows render chat-native sender and target presentation", () => {
  const markup = renderToStaticMarkup(
    <ToolRow
      entries={[
        {
          id: "send-1",
          kind: "tool",
          author: "root",
          role: "system",
          text: "/root -> /root/worker",
          timestamp: "09:42",
          attachments: [],
          toolName: "followup task",
          toolStatus: "completed",
          toolDetails:
            "Tool\nfollowup_task\n\nSender\n/root\n\nReceivers\n/root/worker\n\nPrompt\nplease check this",
          toolCategory: "multiAgent",
          interAgent: {
            kind: "followup",
            direction: "outgoing",
            senderPath: "/root",
            targetPaths: ["/root/worker"],
            primaryPath: "/root/worker",
            title: "You -> @/root/worker",
            body: "please check this",
            status: "completed",
            chips: [],
          },
        },
      ]}
      isOpen
    />,
  );

  assert.match(markup, /inter-agent-row/);
  assert.match(markup, /inter-agent-card/);
  assert.match(markup, /You/);
  assert.match(markup, /@\/root\/worker/);
  assert.match(markup, /please check this/);
  assert.match(markup, /Tool[\s\S]*followup_task/);
});

test("grouped inter-agent rows keep each chat item selectable with audit details", () => {
  const entries: ConversationEntry[] = [
    {
      id: "spawn-1",
      kind: "tool",
      author: "root",
      role: "system",
      text: "/root -> /root/worker",
      timestamp: "09:41",
      attachments: [],
      toolName: "spawn agent",
      toolStatus: "completed",
      toolDetails: "Tool\nspawn_agent",
      toolCategory: "multiAgent",
      interAgent: {
        kind: "spawn",
        direction: "outgoing",
        senderPath: "/root",
        targetPaths: ["/root/worker"],
        primaryPath: "/root/worker",
        title: "Created @/root/worker",
        body: "start here",
        status: "completed",
        chips: ["gpt-5.6"],
      },
    },
    {
      id: "send-1",
      kind: "tool",
      author: "root",
      role: "system",
      text: "/root -> /root/worker",
      timestamp: "09:42",
      attachments: [],
      toolName: "followup task",
      toolStatus: "completed",
      toolDetails: "Tool\nfollowup_task",
      toolCategory: "multiAgent",
      interAgent: {
        kind: "followup",
        direction: "outgoing",
        senderPath: "/root",
        targetPaths: ["/root/worker"],
        primaryPath: "/root/worker",
        title: "You -> @/root/worker",
        body: "continue here",
        status: "completed",
        chips: [],
      },
    },
  ];

  const collapsedMarkup = renderToStaticMarkup(
    <ToolRow entries={entries} isOpen selectedEntryId={null} />,
  );
  assert.match(collapsedMarkup, /2 inter-agent updates/);
  assert.match(collapsedMarkup, /Created @\/root\/worker/);
  assert.match(collapsedMarkup, /You -&gt; @\/root\/worker/);
  assert.doesNotMatch(collapsedMarkup, /Tool[\s\S]*followup_task/);

  const selectedMarkup = renderToStaticMarkup(
    <ToolRow entries={entries} isOpen selectedEntryId="send-1" />,
  );
  assert.match(selectedMarkup, /Tool[\s\S]*followup_task/);
  assert.doesNotMatch(selectedMarkup, /Tool[\s\S]*spawn_agent/);
});

test("artifact row renders html preview in a sandboxed iframe", () => {
  const markup = renderToStaticMarkup(
    <ArtifactRow
      entry={artifactEntry({
        mimeType: "text/html; charset=utf-8",
      })}
    />,
  );

  assert.match(markup, /class="artifact-row"/);
  assert.match(markup, /class="artifact-card"/);
  assert.match(markup, /Inline artifact/);
  assert.match(markup, /text\/html/);
  assert.match(markup, /<iframe /);
  assert.match(markup, /sandbox="allow-scripts"/);
  assert.doesNotMatch(markup, /allow-same-origin/);
  assert.match(markup, /script-src &#x27;unsafe-inline&#x27;/);
});

test("artifact row renders svg preview through the same sandbox", () => {
  const markup = renderToStaticMarkup(
    <ArtifactRow
      entry={artifactEntry({
        mimeType: "image/svg+xml",
        content:
          '<svg viewBox="0 0 10 10"><circle cx="5" cy="5" r="4" /></svg>',
      })}
    />,
  );

  assert.match(markup, /image\/svg\+xml/);
  assert.match(markup, /artifact-svg-host/);
  assert.match(markup, /sandbox="allow-scripts"/);
  assert.doesNotMatch(markup, /allow-same-origin/);
  assert.match(markup, /script-src &#x27;none&#x27;/);
});

test("artifact row previews markdown with MarkdownContent", () => {
  const markup = renderToStaticMarkup(
    <ArtifactRow
      entry={artifactEntry({
        mimeType: "text/markdown; charset=utf-8",
        content: "## Heading\n\n- one",
      })}
    />,
  );

  assert.match(markup, /artifact-markdown-preview/);
  assert.match(markup, /class="markdown-content"/);
  assert.match(markup, /<h2>Heading<\/h2>/);
});

test("unsupported artifact row falls back to source", () => {
  const markup = renderToStaticMarkup(
    <ArtifactRow
      entry={artifactEntry({
        mimeType: "text/mermaid",
        content: "graph TD; A-->B;",
      })}
    />,
  );

  assert.match(markup, /Preview unavailable for this artifact type/);
  assert.match(markup, /class="artifact-source"/);
  assert.doesNotMatch(markup, /<iframe /);
});

test("url artifact row exposes explicit browser action", () => {
  const opened: string[] = [];
  const markup = renderToStaticMarkup(
    <ArtifactRow
      entry={artifactEntry({
        title: "Preview server",
        mimeType: "text/uri-list",
        content: "http://localhost:5173/",
        source: {
          type: "url",
          url: "http://localhost:5173/",
          mimeType: "text/html",
          fallbackContent: "Local preview",
        },
      })}
      onOpenArtifactUrl={(url) => opened.push(url)}
    />,
  );

  assert.match(markup, /Preview server/);
  assert.match(markup, /http:\/\/localhost:5173\//);
  assert.match(markup, />Open<\/button>/);
  assert.doesNotMatch(markup, /<iframe /);
  assert.deepEqual(opened, []);
});

test("virtual list renders url artifacts with browser action", () => {
  const markup = renderToStaticMarkup(
    <ConversationVirtualList
      conversationKey="thread-1"
      cells={[
        {
          id: "artifact-cell",
          kind: "artifact",
          entries: [
            artifactEntry({
              title: "Preview server",
              mimeType: "text/uri-list",
              content: "http://localhost:5173/",
              source: {
                type: "url",
                url: "http://localhost:5173/",
                mimeType: "text/html",
                fallbackContent: "Local preview",
              },
            }),
          ],
        },
      ]}
      containerRef={React.createRef<HTMLDivElement>()}
      focusedItem={null}
      onOpenLocalFile={() => {}}
      onOpenArtifactUrl={() => {}}
      searchCurrentCellId={null}
      searchMatchCellIds={new Set()}
    />,
  );

  assert.match(markup, /Preview server/);
  assert.match(markup, />Open<\/button>/);
  assert.match(markup, /data-conversation-key="thread-1"/);
});

test("virtual list cell instance keys are scoped to their conversation", () => {
  assert.equal(
    conversationCellInstanceKey("thread-1", "item-1"),
    "thread-1:item-1",
  );
  assert.equal(
    conversationCellInstanceKey("thread-2", "item-1"),
    "thread-2:item-1",
  );
  assert.notEqual(
    conversationCellInstanceKey("thread-1", "item-1"),
    conversationCellInstanceKey("thread-2", "item-1"),
  );
  assert.equal(conversationCellInstanceKey(null, "item-1"), "item-1");
});

test("virtual list renders backend compact summary read projection as a visible message", () => {
  const thread = normalizeThreadSnapshot({
    id: "thread-1",
    sessionId: "session-1",
    forkedFromId: null,
    preview: "",
    ephemeral: false,
    modelProvider: "openai",
    model: "gpt-5",
    reasoningEffort: null,
    createdAt: 1,
    updatedAt: 2,
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
    turns: [
      {
        id: "turn-old",
        items: [
          {
            type: "agentMessage",
            id: "old-agent",
            text: "old answer hidden by compact",
            phase: null,
            memoryCitation: null,
          },
        ],
        itemsView: "full",
        status: "completed",
        error: null,
        startedAt: 1,
        completedAt: 1,
        durationMs: 0,
      },
      {
        id: "compact-turn",
        items: [
          { type: "contextCompaction", id: "item-1" },
          {
            type: "agentMessage",
            id: "item-1:summary",
            text: "compact summary body from thread/read",
            phase: null,
            memoryCitation: null,
          },
          {
            type: "injectedContext",
            id: "ctx-1",
            title: "Init Context",
            preview: "Init Context",
            sections: [
              {
                label: "User Preferences",
                text: "# User Preferences\n\nProject body",
              },
            ],
          },
        ],
        itemsView: "full",
        status: "completed",
        error: null,
        startedAt: 2,
        completedAt: 2,
        durationMs: 0,
      },
    ],
  } satisfies Thread);
  const { cells } = buildConversationState(thread);
  const markup = renderToStaticMarkup(
    <ConversationVirtualList
      cells={cells}
      containerRef={React.createRef<HTMLDivElement>()}
      focusedItem={null}
      onOpenLocalFile={() => {}}
      onOpenArtifactUrl={() => {}}
      searchCurrentCellId="item-1:summary"
      searchMatchCellIds={new Set(["item-1:summary"])}
    />,
  );

  assert.match(markup, /Context compacted/);
  assert.match(markup, /compact summary body from thread\/read/);
  assert.match(markup, /Init Context · User Preferences/);
  const compactDetailsMarkup =
    markup.match(
      /<details class="compact-history-details">[\s\S]*?<\/details>/,
    )?.[0] ?? "";
  assert.match(compactDetailsMarkup, /old answer hidden by compact/);
  const mainTimelineMarkup = markup.replace(compactDetailsMarkup, "");
  assert.doesNotMatch(mainTimelineMarkup, /old answer hidden by compact/);
  const searchCurrentRowMarkup =
    mainTimelineMarkup.match(
      /<div class="conversation-virtual-row search-match search-current"[\s\S]*?(?=<div class="conversation-virtual-row|$)/,
    )?.[0] ?? "";
  assert.match(searchCurrentRowMarkup, /compact summary body from thread\/read/);
  assert.doesNotMatch(searchCurrentRowMarkup, /old answer hidden by compact/);
  assert.doesNotMatch(markup, /compact-summary-details/);
});

test("compact rows show archived artifact evidence without inline previews", () => {
  const markup = renderToStaticMarkup(
    <CompactRow
      entry={{
        id: "compact-with-artifact",
        kind: "compact",
        author: "Root",
        role: "system",
        text: "Context compacted",
        timestamp: "09:43",
        attachments: [],
        archivedEntryCount: 1,
        archivedCells: [
          {
            id: "archived-artifact",
            kind: "artifact",
            entries: [artifactEntry()],
          },
        ],
      }}
    />,
  );

  assert.match(markup, /Context compacted/);
  assert.match(markup, /Previous context · 1 item/);
  assert.match(markup, /Inline artifact/);
  assert.match(markup, /text\/html/);
  assert.doesNotMatch(markup, /class="artifact-row"/);
  assert.doesNotMatch(markup, /class="artifact-card"/);
  assert.doesNotMatch(markup, /<iframe /);
  assert.doesNotMatch(markup, /Archived item/);
});

test("conversation text surfaces keep long urls inside measured cells", () => {
  const longUrl =
    "https://example.com/search?q=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
  const longPath =
    "/aio_sandbox/sandboxes/123456789012345678901234567890/sessions/987654321098765432109876543210/bash_server/run_bash_with_a_very_long_route";
  const longMethod =
    "AIOSandbox_runBashCommandWithVeryLongUnderscoreFreeMethodName";
  const messageMarkup = renderToStaticMarkup(
    <MessageRow
      entries={[
        {
          id: "agent-url",
          kind: "message",
          author: "Codex",
          role: "agent",
          text: `[result](${longUrl})`,
          timestamp: "09:41",
          attachments: [
            {
              kind: "file",
              label: `modified ${longPath}`,
            },
          ],
        },
      ]}
    />,
  );
  const toolMarkup = renderToStaticMarkup(
    <ToolRow
      entries={[
        {
          id: "web-search-url",
          kind: "tool",
          author: "root",
          role: "system",
          text: `session schema: ${longPath}`,
          timestamp: "09:42",
          attachments: [],
          toolName: longMethod,
          toolStatus: "completed_with_extremely_long_status_identifier",
          toolDetails: `Route\n${longPath}\n\nMethod\n${longMethod}\n\nSession ID\n1234567890123456789012345678901234567890`,
          toolOutput: {
            label: `Output ${longPath}`,
            text: longUrl,
            isEmpty: false,
          },
          toolCategory: "external",
        },
      ]}
      isOpen
    />,
  );
  const compactMarkup = renderToStaticMarkup(
    <CompactRow
      entry={{
        id: "compact-long-text",
        kind: "compact",
        author: "Root",
        role: "system",
        text: "Context compacted",
        timestamp: "09:43",
        attachments: [],
        archivedEntryCount: 1,
        archivedCells: [
          {
            id: "archived-tool",
            kind: "tool",
            entries: [
              {
                id: "archived-tool-entry",
                kind: "tool",
                author: "root",
                role: "system",
                text: `archived ${longPath}`,
                timestamp: "09:41",
                attachments: [],
                toolName: longMethod,
                toolStatus: "completed",
                toolDetails: longPath,
                toolCategory: "external",
              },
            ],
          },
        ],
      }}
    />,
  );
  const artifactMarkup = renderToStaticMarkup(
    <ArtifactRow
      entry={artifactEntry({
        title: longMethod,
        mimeType: "text/html",
        content: `<main>${longPath}</main>`,
      })}
    />,
  );
  const approvalMarkup = renderToStaticMarkup(
    <ApprovalRequestsPanel
      requests={[
        {
          requestId: "long-approval",
          kind: "permissions",
          threadId: "thread-1",
          turnId: "turn-1",
          itemId: "permissions-1",
          startedAtMs: 1_725_000_000_000,
          reason: `needs ${longPath}`,
          title: "Permissions request",
          detail: `Grant access for ${longMethod}`,
          metadata: [{ label: "write paths", value: longPath }],
          permissions: { fileSystem: { write: [longPath] } },
          status: "pending",
          error: null,
          availableDecisions: ["accept", "decline"],
        },
      ]}
      onRespond={() => {}}
    />,
  );
  const styles = readFileSync(
    new URL("../styles.css", import.meta.url),
    "utf8",
  );

  assert.match(messageMarkup, /class="message-bubble"/);
  assert.match(messageMarkup, /class="markdown-content"/);
  assert.match(messageMarkup, /class="attachment-chip"/);
  assert.match(toolMarkup, /class="tool-card-copy"/);
  assert.match(toolMarkup, new RegExp(longMethod));
  assert.match(toolMarkup, /completed_with_extremely_long_status_identifier/);
  assert.match(toolMarkup, /Session ID/);
  assert.match(compactMarkup, /Context compacted/);
  assert.match(compactMarkup, /Previous context · 1 item/);
  assert.match(compactMarkup, /archive-tool-stack/);
  assert.match(compactMarkup, new RegExp(longMethod));
  assert.match(compactMarkup, new RegExp(longPath));
  assert.match(artifactMarkup, /class="artifact-card-copy"/);
  assert.match(artifactMarkup, new RegExp(longMethod));
  assert.match(approvalMarkup, /class="approval-request-metadata"/);
  assert.match(approvalMarkup, new RegExp(longMethod));
  assert.match(styles, /\.conversation-virtual-row[\s\S]*max-width: 100%;/);
  assert.match(
    styles,
    /\.message-row,[\s\S]*\.archive-row \{[\s\S]*width: 100%;[\s\S]*max-width: 100%;/,
  );
  assert.match(styles, /\.tool-card \{[\s\S]*flex: 1 1 auto;/);
  assert.match(styles, /\.tool-card \{[\s\S]*max-width: 100%;/);
  assert.match(styles, /\.markdown-content p[\s\S]*overflow-wrap: anywhere;/);
  assert.match(styles, /\.markdown-content a[\s\S]*overflow-wrap: anywhere;/);
  assert.match(
    styles,
    /\.markdown-content code[\s\S]*overflow-wrap: anywhere;/,
  );
  assert.match(styles, /\.tool-card-copy span[\s\S]*overflow-wrap: anywhere;/);
  assert.match(
    styles,
    /\.tool-card-copy strong[\s\S]*overflow-wrap: anywhere;/,
  );
  assert.match(styles, /\.artifact-card \{[\s\S]*max-width: 100%;/);
  assert.match(styles, /\.artifact-card-head \{[^}]*flex-wrap: wrap;/);
  assert.match(styles, /\.artifact-card-copy \{[^}]*flex: 1 1 220px;/);
  assert.match(
    styles,
    /\.artifact-card-copy strong[\s\S]*overflow-wrap: anywhere;/,
  );
  assert.match(styles, /\.artifact-preview-frame \{[\s\S]*min-height: 220px;/);
  assert.match(styles, /\.artifact-source pre[\s\S]*overflow-wrap: anywhere;/);
  assert.match(styles, /\.tool-card-summary \{[^}]*flex-wrap: wrap;/);
  assert.match(styles, /\.tool-card-item-head \{[^}]*flex-wrap: wrap;/);
  assert.match(styles, /\.tool-card-copy \{[^}]*flex: 1 1 240px;/);
  assert.match(styles, /\.tool-card-meta \{[^}]*flex: 0 0 auto;/);
  assert.match(styles, /\.tool-status-badge \{[^}]*white-space: nowrap;/);
  assert.match(styles, /\.tool-status-badge \{[^}]*text-overflow: ellipsis;/);
  assert.match(
    styles,
    /\.tool-status-badge \{[^}]*max-width: min\(100%, 22ch\);/,
  );
  assert.doesNotMatch(
    styles,
    /\.tool-status-badge \{[^}]*overflow-wrap: anywhere;/,
  );
  assert.match(styles, /\.tool-card-body pre[\s\S]*overflow-wrap: anywhere;/);
  assert.match(
    styles,
    /\.tool-output-block summary[\s\S]*overflow-wrap: anywhere;/,
  );
  assert.match(
    styles,
    /\.compact-card,[\s\S]*\.archive-card \{[\s\S]*flex: 1 1 auto;/,
  );
  assert.match(styles, /\.archive-cell \{[\s\S]*min-width: 0;/);
  assert.match(styles, /\.compact-history-body \{[\s\S]*min-width: 0;/);
  assert.match(
    styles,
    /\.approval-request-metadata dd[\s\S]*white-space: normal;/,
  );
  assert.match(
    styles,
    /\.approval-request-metadata dd[\s\S]*overflow-wrap: anywhere;/,
  );
  assert.match(styles, /\.attachment-chip \{[\s\S]*max-width: 100%;/);
  assert.match(styles, /\.attachment-chip span[\s\S]*overflow-wrap: anywhere;/);
});

test("grouped agent message rows render one bubble with multiple segments", () => {
  const firstEntry: ConversationEntry = {
    id: "agent-1",
    kind: "message",
    author: "Codex",
    role: "agent",
    text: "first",
    timestamp: "09:41",
    attachments: [],
  };
  const secondEntry: ConversationEntry = {
    ...firstEntry,
    id: "agent-2",
    text: "second",
    timestamp: "09:42",
  };

  const markup = renderToStaticMarkup(
    <MessageRow entries={[firstEntry, secondEntry]} />,
  );

  assert.equal(markup.match(/class="message-bubble/g)?.length ?? 0, 1);
  assert.match(markup, /class="message-bubble message-bubble-combined"/);
  assert.equal(markup.match(/class="message-segment"/g)?.length ?? 0, 2);
  assert.match(markup, /first[\s\S]*second/);
});

test("message rows only combine bubbles when every entry is an agent message", () => {
  const agentEntry: ConversationEntry = {
    id: "agent-1",
    kind: "message",
    author: "Codex",
    role: "agent",
    text: "agent",
    timestamp: "09:41",
    attachments: [],
  };
  const userEntry: ConversationEntry = {
    id: "user-1",
    kind: "message",
    author: "You",
    role: "user",
    text: "user",
    timestamp: "09:42",
    attachments: [],
  };

  const markup = renderToStaticMarkup(
    <MessageRow entries={[agentEntry, userEntry]} />,
  );

  assert.equal(markup.match(/class="message-bubble"/g)?.length ?? 0, 2);
  assert.doesNotMatch(markup, /message-bubble-combined/);
});

test("single tool rows render a single inline item and auto-expand details with the card", () => {
  const [singleEntry] = entries;
  assert.ok(singleEntry);

  const collapsedMarkup = renderToStaticMarkup(
    <ToolRow entries={[singleEntry]} selectedEntryId={null} />,
  );

  assert.doesNotMatch(collapsedMarkup, /tool-card-list/);
  assert.doesNotMatch(collapsedMarkup, /tool-card-item-single/);
  assert.doesNotMatch(collapsedMarkup, /first details/);

  const expandedMarkup = renderToStaticMarkup(
    <ToolRow entries={[singleEntry]} isOpen />,
  );

  assert.match(expandedMarkup, /first summary[\s\S]*first details/);
  assert.doesNotMatch(expandedMarkup, /tool-card-item-single/);
  assert.equal(expandedMarkup.match(/first summary/g)?.length ?? 0, 1);
});

test("command notification tool rows keep output collapsed inside expanded cards", () => {
  const markup = renderToStaticMarkup(
    <ToolRow
      entries={[
        {
          id: "cmd-1:notification:exit",
          kind: "tool",
          author: "root",
          role: "system",
          text: "Command notification • exit 1 • npm test",
          timestamp: "09:41",
          attachments: [],
          toolName: "Command notification",
          toolStatus: "failed",
          toolDetails:
            "Kind\nexit\n\nCommand\nnpm test\n\nCommand ID\ncmd-1\n\nExit Code\n1",
          toolOutput: {
            label: "Output",
            text: "line one\nline two",
            isEmpty: false,
          },
          toolCategory: "commandNotification",
        },
      ]}
      isOpen
    />,
  );

  assert.match(markup, /tool-card-commandNotification/);
  assert.match(markup, /Command notification[\s\S]*Exit Code[\s\S]*1/);
  assert.match(markup, /<details class="tool-output-block">/);
  assert.doesNotMatch(markup, /<details class="tool-output-block" open/);
  assert.match(markup, /<summary>Output<\/summary>/);
  assert.match(markup, /line one[\s\S]*line two/);
});

test("terminal-emulated tool output renders in a terminal preview surface", () => {
  const markup = renderToStaticMarkup(
    <ToolRow
      entries={[
        {
          id: "cmd-1:notification:output",
          kind: "tool",
          author: "root",
          role: "system",
          text: "Command notification • output • fly deploy",
          timestamp: "09:41",
          attachments: [],
          toolName: "fly deploy",
          toolStatus: "completed",
          toolDetails: "Kind\noutput\n\nCommand\nfly deploy",
          toolOutput: {
            label: "Command output",
            text: "Layer already exists\r\u001b[2KPushed\n",
            isEmpty: false,
            terminalEmulated: true,
          },
          toolCategory: "command",
        },
      ]}
      isOpen
    />,
  );

  assert.match(markup, /<details class="tool-output-block" open="">/);
  assert.match(markup, /<summary>Command output<\/summary>/);
  assert.match(markup, /tool-terminal-output-preview/);
  assert.match(markup, /Terminal-rendered command output/);
  assert.doesNotMatch(markup, /<pre>Layer already exists/);
});

test("terminal output preview appends streaming growth without replaying full output", () => {
  assert.deepEqual(
    planTerminalOutputPreviewWrite("line one\n", "line one\nline two\n"),
    {
      kind: "append",
      text: "line two\n",
    },
  );
  assert.deepEqual(planTerminalOutputPreviewWrite("line one\n", "line one\n"), {
    kind: "noop",
  });
  assert.deepEqual(
    planTerminalOutputPreviewWrite("old output\n", "new output\n"),
    {
      kind: "reset",
      text: "new output\n",
    },
  );
  assert.equal(
    `${TERMINAL_OUTPUT_PREVIEW_RESET_SEQUENCE}new output\n`,
    "\x1b[3J\x1b[H\x1b[2Jnew output\n",
  );
});

test("terminal output preview resets through the xterm write queue", () => {
  const source = readFileSync(
    new URL("./Conversation.tsx", import.meta.url),
    "utf8",
  );
  const previewStart = source.indexOf("function TerminalOutputPreview");
  const previewEnd = source.indexOf(
    "function toolOutputClassName",
    previewStart,
  );
  const previewSource = source.slice(previewStart, previewEnd);

  assert.notEqual(previewStart, -1);
  assert.notEqual(previewEnd, -1);
  assert.match(
    previewSource,
    /terminal\.write\(`\$\{TERMINAL_OUTPUT_PREVIEW_RESET_SEQUENCE\}/,
  );
  assert.doesNotMatch(previewSource, /terminal\.clear\(\)/);
});

test("command start, output, and exit notifications render in one command cell with selectable output", () => {
  const state = buildConversationState({
    id: "thread-1",
    updatedAt: 1,
    turns: [
      {
        id: "turn-1",
        items: [
          {
            type: "commandExecution",
            id: "cmd-1",
            command: "npm test",
            cwd: "/tmp/project",
            status: "running",
            aggregatedOutput: null,
            exitCode: null,
            durationMs: null,
          },
          {
            type: "commandExecutionNotification",
            id: "cmd-1:notification:output",
            commandItemId: "cmd-1",
            kind: "output",
            message: "Command output notification received.",
            output: "stdout line",
            exitCode: null,
            createdAtMs: 2_000,
          },
          {
            type: "commandExecutionNotification",
            id: "cmd-1:notification:exit",
            commandItemId: "cmd-1",
            kind: "exit",
            message: "Command exit notification received.",
            output: "stderr line",
            exitCode: 1,
            createdAtMs: 3_000,
          },
        ],
        itemsView: "full",
        status: "completed",
        error: null,
        startedAt: 1,
        completedAt: 3,
        durationMs: 2,
      },
    ],
  } as Thread);

  assert.equal(state.cells.length, 1);
  assert.deepEqual(
    state.cells[0]?.entries.map((entry) => entry.id),
    ["cmd-1", "cmd-1:notification:output", "cmd-1:notification:exit"],
  );

  const listMarkup = renderToStaticMarkup(
    <ToolRow entries={state.cells[0]?.entries ?? []} isOpen />,
  );
  assert.match(listMarkup, /npm test/);
  assert.match(listMarkup, /Command notification • output • npm test/);
  assert.match(listMarkup, /Command notification • exit 1 • npm test/);
  assert.doesNotMatch(listMarkup, /stdout line/);
  assert.doesNotMatch(listMarkup, /stderr line/);

  const outputMarkup = renderToStaticMarkup(
    <ToolRow
      entries={state.cells[0]?.entries ?? []}
      isOpen
      selectedEntryId="cmd-1:notification:output"
    />,
  );
  assert.match(outputMarkup, /<summary>Command output<\/summary>/);
  assert.match(outputMarkup, /tool-terminal-output-preview/);
  assert.doesNotMatch(outputMarkup, /<pre>stdout line<\/pre>/);

  const exitMarkup = renderToStaticMarkup(
    <ToolRow
      entries={state.cells[0]?.entries ?? []}
      isOpen
      selectedEntryId="cmd-1:notification:exit"
    />,
  );
  assert.match(exitMarkup, /<summary>Command exit output<\/summary>/);
  assert.match(exitMarkup, /tool-terminal-output-preview/);
  assert.doesNotMatch(exitMarkup, /<pre>stderr line<\/pre>/);
});

test("expanded poll_event tool rows render current wait progress", () => {
  const markup = renderToStaticMarkup(
    <ToolRow
      entries={[
        {
          id: "poll-event",
          kind: "tool",
          author: "root",
          role: "system",
          text: "poll_event • waiting up to 10s",
          timestamp: "09:41",
          attachments: [],
          toolName: "poll_event",
          toolStatus: "inProgress",
          toolDetails: "Output\ncurrentTimeoutMs: 10000",
          toolCategory: "multiAgent",
          pollEventProgress: {
            startedAtMs: Date.now() - 2_000,
            currentTimeoutMs: 10_000,
          },
        },
      ]}
      isOpen
    />,
  );

  assert.match(markup, /Elapsed/);
  assert.match(markup, /Remaining/);
  assert.match(markup, /role="progressbar"/);
  assert.match(markup, /poll-event-progress-track/);
});

test("tool rows treat partially completed lists as in progress", () => {
  const mixedEntries: ConversationEntry[] = [
    entries[0]!,
    {
      ...entries[1]!,
      id: "tool-3",
      toolStatus: "running",
    },
  ];

  const markup = renderToStaticMarkup(
    <ToolRow entries={mixedEntries} isOpen selectedEntryId={null} />,
  );

  assert.match(markup, /1\/2 done/);
  assert.match(markup, /tool-status-badge doing/);
  assert.match(markup, /tool-status-badge done[^>]*>completed/);
  assert.match(markup, /tool-status-badge doing[^>]*>running/);
});

test("conversation virtual rows expose search match and current highlight classes", () => {
  assert.equal(
    buildConversationRowClassName({
      highlighted: false,
      searchCurrent: false,
      searchMatch: true,
    }),
    "conversation-virtual-row search-match",
  );
  assert.equal(
    buildConversationRowClassName({
      highlighted: true,
      searchCurrent: true,
      searchMatch: true,
    }),
    "conversation-virtual-row highlighted search-match search-current",
  );
});

test("focused conversation jumps only consume each focus token once", () => {
  assert.equal(
    shouldHandleFocusedItemRequest({
      focusedItem: { itemId: "cmd-1", token: 1 },
      lastHandledRequest: null,
    }),
    true,
  );
  assert.equal(
    shouldHandleFocusedItemRequest({
      focusedItem: { itemId: "cmd-1", token: 1 },
      lastHandledRequest: { itemId: "cmd-1", token: 1 },
    }),
    false,
  );
  assert.equal(
    shouldHandleFocusedItemRequest({
      focusedItem: { itemId: "cmd-1", token: 2 },
      lastHandledRequest: { itemId: "cmd-1", token: 1 },
    }),
    true,
  );
  assert.equal(
    shouldHandleFocusedItemRequest({
      focusedItem: { itemId: "search-hit-1", token: 1 },
      lastHandledRequest: { itemId: "cmd-1", token: 1 },
    }),
    true,
  );
});

test("focused conversation jumps retry until the target row is measured", () => {
  assert.deepEqual(
    planFocusedItemScrollAttempt({
      attempts: 0,
      targetMeasured: false,
    }),
    {
      behavior: "smooth",
      nextAttempts: 1,
      shouldComplete: false,
    },
  );
  assert.deepEqual(
    planFocusedItemScrollAttempt({
      attempts: 1,
      targetMeasured: true,
    }),
    {
      behavior: "auto",
      nextAttempts: 2,
      shouldComplete: true,
    },
  );
});

test("focused conversation jumps complete immediately for measured target rows", () => {
  assert.deepEqual(
    planFocusedItemScrollAttempt({
      attempts: 0,
      targetMeasured: true,
    }),
    {
      behavior: "smooth",
      nextAttempts: 1,
      shouldComplete: true,
    },
  );
});

test("focused conversation jumps stop retrying after a bounded number of attempts", () => {
  assert.deepEqual(
    planFocusedItemScrollAttempt({
      attempts: 2,
      targetMeasured: false,
    }),
    {
      behavior: "auto",
      nextAttempts: 3,
      shouldComplete: true,
    },
  );
});

test("conversation measurements mark rows measured even when height matches the estimate", () => {
  assert.deepEqual(
    planConversationCellMeasurement({
      measuredHeight: 112,
      previousHeight: 112,
      wasMeasured: false,
    }),
    {
      roundedHeight: 112,
      heightChanged: false,
      shouldBumpHeightVersion: true,
    },
  );
  assert.deepEqual(
    planConversationCellMeasurement({
      measuredHeight: 112,
      previousHeight: 112,
      wasMeasured: true,
    }),
    {
      roundedHeight: 112,
      heightChanged: false,
      shouldBumpHeightVersion: false,
    },
  );
});

test("compact rows render only the marker without grouped history body", () => {
  const markup = renderToStaticMarkup(
    <CompactRow
      entry={{
        id: "compact-1",
        kind: "compact",
        author: "Root",
        role: "system",
        text: "Context compacted",
        timestamp: "09:43",
        attachments: [],
        compactSummary: "Compacted summary remains visible in the chat.",
      }}
    />,
  );

  assert.match(markup, /Context compacted/);
  assert.doesNotMatch(markup, /Compacted summary remains visible in the chat/);
  assert.match(markup, /09:43/);
  assert.doesNotMatch(markup, /button/);
  assert.doesNotMatch(markup, /load the archived conversation/);
  assert.doesNotMatch(markup, /recent request/);
  assert.doesNotMatch(markup, /functions\/exec_command/);
});

test("compact rows do not render compact summary text inline", () => {
  const markup = renderToStaticMarkup(
    <CompactRow
      entry={{
        id: "compact-1",
        kind: "compact",
        author: "Root",
        role: "system",
        text: "## Current Goal\n\n- Preserve compact summary",
        timestamp: "09:43",
        attachments: [],
        compactSummary: "## Current Goal\n\n- Preserve compact summary",
      }}
    />,
  );

  assert.match(markup, /Context compacted/);
  assert.doesNotMatch(markup, /Preserve compact summary/);
  assert.doesNotMatch(markup, /Summary available/);
  assert.doesNotMatch(markup, /<details class="compact-summary-details">/);
  assert.doesNotMatch(markup, /<summary>View summary<\/summary>/);
});

test("compact rows omit fallback text when summary is missing", () => {
  const markup = renderToStaticMarkup(
    <CompactRow
      entry={{
        id: "compact-1",
        kind: "compact",
        author: "Root",
        role: "system",
        text: "Context compacted",
        timestamp: "09:43",
        attachments: [],
        compactSummary: null,
      }}
    />,
  );

  assert.match(markup, /Context compacted/);
  assert.doesNotMatch(markup, /Summary unavailable/);
  assert.doesNotMatch(markup, /<details class="compact-summary-details">/);
  assert.doesNotMatch(markup, /View summary/);
});

test("compact row does not render large compact summary previews", () => {
  const longSummary = `${"summary line\n".repeat(400)}UNBOUNDED_COMPACT_SENTINEL`;
  const markup = renderToStaticMarkup(
    <CompactRow
      entry={{
        id: "compact-1",
        kind: "compact",
        author: "Root",
        role: "system",
        text: "Context compacted",
        timestamp: "09:43",
        attachments: [],
        compactSummary: longSummary,
      }}
    />,
  );

  assert.match(markup, /Context compacted/);
  assert.doesNotMatch(markup, /View summary/);
  assert.doesNotMatch(markup, /summary line/);
  assert.doesNotMatch(markup, /\[truncated: [\d,]+ characters omitted\]/);
  assert.doesNotMatch(markup, /UNBOUNDED_COMPACT_SENTINEL/);
  assert.doesNotMatch(markup, /<details class="compact-summary-details">/);
});

test("compact rows expose direct archived history evidence", () => {
  const collapsedMarkup = renderToStaticMarkup(
    <CompactRow
      entry={{
        id: "compact-1",
        kind: "compact",
        author: "Root",
        role: "system",
        text: "Previous conversation was archived; compacted model context continues below.",
        timestamp: "09:43",
        attachments: [],
        archivedEntryCount: 1,
        archivedCells: [
          {
            id: "archived-message",
            kind: "message",
            entries: [
              {
                id: "archived-message",
                kind: "message",
                author: "You",
                role: "user",
                text: "old request",
                timestamp: "09:41",
                attachments: [],
              },
            ],
          },
        ],
      }}
    />,
  );
  assert.match(collapsedMarkup, /Previous context · 1 item/);
  assert.match(collapsedMarkup, /old request/);
  assert.doesNotMatch(collapsedMarkup, /Compacted context/);

  const expandedMarkup = renderToStaticMarkup(
    <CompactRow
      entry={{
        id: "compact-1",
        kind: "compact",
        author: "Root",
        role: "system",
        text: "Previous conversation was archived; compacted model context continues below.",
        timestamp: "09:43",
        attachments: [],
        archivedEntryCount: 1,
        archivedCells: [
          {
            id: "archived-message",
            kind: "message",
            entries: [
              {
                id: "archived-message",
                kind: "message",
                author: "You",
                role: "user",
                text: "old request",
                timestamp: "09:41",
                attachments: [],
              },
            ],
          },
        ],
      }}
    />,
  );

  assert.match(expandedMarkup, /Context compacted/);
  assert.match(expandedMarkup, /Previous context · 1 item/);
  assert.match(expandedMarkup, /old request/);
  assert.doesNotMatch(expandedMarkup, /Compacted context/);
  assert.doesNotMatch(expandedMarkup, /recent request/);
});

test("expanded compact rows do not render nested compact groups from archived history", () => {
  const markup = renderToStaticMarkup(
    <CompactRow
      entry={{
        id: "compact-2",
        kind: "compact",
        author: "Root",
        role: "system",
        text: "Previous conversation was archived; compacted model context continues below.",
        timestamp: "09:43",
        attachments: [],
        archivedEntryCount: 3,
        archivedCells: [
          {
            id: "compact-1",
            kind: "compact",
            entries: [
              {
                id: "compact-1",
                kind: "compact",
                author: "Root",
                role: "system",
                text: "Previous conversation was archived; compacted model context continues below.",
                timestamp: "09:41",
                attachments: [],
                archivedEntryCount: 1,
                archivedCells: [
                  {
                    id: "old-message",
                    kind: "message",
                    entries: [
                      {
                        id: "old-message",
                        kind: "message",
                        author: "You",
                        role: "user",
                        text: "old request",
                        timestamp: "09:39",
                        attachments: [],
                      },
                    ],
                  },
                ],
              },
            ],
          },
        ],
      }}
    />,
  );

  assert.match(markup, /Context compacted/);
  assert.doesNotMatch(markup, /first replacement/);
  assert.doesNotMatch(markup, /old request/);
});

test("archived history rows collapse previous conversation by default", () => {
  const markup = renderToStaticMarkup(
    <ArchivedHistoryRow
      entry={{
        id: "archive-1",
        kind: "archive",
        author: "Root",
        role: "system",
        text: "Previous conversation is no longer the active model context.",
        timestamp: "09:43",
        attachments: [],
        archivedEntryCount: 1,
        archivedCells: [
          {
            id: "old-message",
            kind: "message",
            entries: [
              {
                id: "old-message",
                kind: "message",
                author: "You",
                role: "user",
                text: "old request",
                timestamp: "09:41",
                attachments: [],
              },
            ],
          },
        ],
      }}
    />,
  );

  assert.match(markup, /Previous conversation/);
  assert.match(markup, /1 archived item/);
  assert.match(markup, /<details class="archive-card">/);
  assert.doesNotMatch(markup, /<details class="archive-card" open/);
  assert.match(markup, /old request/);
});

test("archived history rows show archived tool details when expanded", () => {
  const markup = renderToStaticMarkup(
    <ArchivedHistoryRow
      entry={{
        id: "archive-1",
        kind: "archive",
        author: "Root",
        role: "system",
        text: "Previous conversation is no longer the active model context.",
        timestamp: "09:43",
        attachments: [],
        archivedEntryCount: 1,
        archivedCells: [
          {
            id: "tool-1",
            kind: "tool",
            entries: [
              {
                id: "tool-1",
                kind: "tool",
                author: "Root",
                role: "system",
                text: "pwd",
                timestamp: "09:41",
                attachments: [],
                toolName: "shell",
                toolStatus: "completed",
                toolDetails: "Command\npwd",
                toolCategory: "command",
              },
            ],
          },
        ],
      }}
    />,
  );

  assert.match(markup, /archive-tool-stack/);
  assert.match(markup, /Command[\s\S]*pwd/);
});
