import type { CSSProperties, MouseEvent } from "react";

import {
  countDescendants,
  isRootThread,
  treeThreadStatusClass,
  treeThreadStatusLabel,
} from "../lib/thread";
import type { TreeMenuState, TreeNode } from "../types";
import { ChevronDownIcon, RobotIcon } from "./icons";

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
  const statusClass = treeThreadStatusClass(node);
  const statusLabel = treeThreadStatusLabel(statusClass);
  const openContextMenu = (event: MouseEvent<HTMLElement>) => {
    event.preventDefault();
    event.stopPropagation();
    if (isRoot || node.isPlaceholder || !node.thread) {
      onOpenMenu(null);
      return;
    }
    onOpenMenu({
      threadId: node.threadId,
      x: event.clientX,
      y: event.clientY,
    });
  };

  return (
    <div
      className={`tree-node ${isRoot ? "tree-node-root" : ""}`}
      style={{ "--depth": depth } as CSSProperties}
      onContextMenu={openContextMenu}
    >
      <button
        type="button"
        className={`tree-node-button ${node.threadId === selectedThreadId ? "selected" : ""}`}
        onContextMenu={openContextMenu}
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
          <span className="tree-agent-column">
            <span className="tree-agent-icon">
              <RobotIcon />
            </span>
            <span
              className={`tree-inline-status ${statusClass}`}
              title={statusLabel}
              aria-label={statusLabel}
            />
          </span>
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
