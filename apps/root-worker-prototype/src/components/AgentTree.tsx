import type { CSSProperties } from "react";

import { countDescendants, isRootThread, threadStatusClass } from "../lib/thread";
import type { TreeMenuState, TreeNode } from "../types";
import { ChevronDownIcon } from "./icons";

export function AgentTreeNode({
  collapsedSet,
  node,
  depth,
  onSelect,
  onToggle,
  onOpenMenu,
  selectedThreadId,
}: {
  collapsedSet: Set<string>;
  node: TreeNode;
  depth: number;
  onSelect: (threadId: string) => void;
  onToggle: (threadId: string) => void;
  onOpenMenu: (menu: TreeMenuState | null) => void;
  selectedThreadId: string | null;
}) {
  const count = countDescendants(node);
  const isCollapsed = collapsedSet.has(node.threadId);
  const hasChildren = node.children.length > 0;
  const isRoot = node.thread ? isRootThread(node.thread) : depth === 0;

  return (
    <div
      className={`tree-node ${isRoot ? "tree-node-root" : ""}`}
      style={{ "--depth": depth } as CSSProperties}
      onContextMenu={(event) => {
        event.preventDefault();
        if (isRoot || node.isPlaceholder || !node.thread) {
          onOpenMenu(null);
          return;
        }
        onOpenMenu({
          threadId: node.threadId,
          x: event.clientX,
          y: event.clientY,
        });
      }}
    >
      <button
        type="button"
        className={`tree-node-button ${node.threadId === selectedThreadId ? "selected" : ""}`}
        onClick={() => {
          if (!node.isPlaceholder) {
            onSelect(node.threadId);
          }
        }}
      >
        <span className="tree-node-leading">
          {hasChildren ? (
            <span
              className={`tree-toggle ${isCollapsed ? "collapsed" : ""}`}
              onClick={(event) => {
                event.stopPropagation();
                onToggle(node.threadId);
              }}
            >
              <ChevronDownIcon />
            </span>
          ) : (
            <span className="tree-spacer" />
          )}
          <span
            className={`status-dot ${node.thread ? threadStatusClass(node.thread.status) : "todo"}`}
          />
        </span>
        <span className="tree-node-copy">
          <strong>{node.label}</strong>
          <span>{node.path}</span>
        </span>
        {count > 0 ? <span className="tree-count">{count}</span> : null}
      </button>
      {hasChildren && !isCollapsed ? (
        <div className="tree-node-children">
          {node.children.map((child) => (
            <AgentTreeNode
              key={child.key}
              collapsedSet={collapsedSet}
              depth={depth + 1}
              node={child}
              onSelect={onSelect}
              onToggle={onToggle}
              onOpenMenu={onOpenMenu}
              selectedThreadId={selectedThreadId}
            />
          ))}
        </div>
      ) : null}
    </div>
  );
}
