import test from "node:test";
import assert from "node:assert/strict";

import {
  buildGitPanelViewModel,
  isGitGraphCommit,
  type GitSnapshot,
} from "./gitPanelView";

function gitSnapshot(overrides: Partial<GitSnapshot> = {}): GitSnapshot {
  return {
    available: true,
    root: "/repo",
    treeRoot: "/repo",
    branch: "main",
    selectedRef: null,
    refs: [],
    graph: [],
    changes: [],
    error: null,
    ...overrides,
  };
}

test("buildGitPanelViewModel falls back to thread analysis changes before Git snapshot loads", () => {
  const model = buildGitPanelViewModel(null, [
    { path: "/repo/a.ts", displayPath: "a.ts", kind: "modified", updateCount: 1 },
    { path: "/repo/b.ts", displayPath: "b.ts", kind: "added", updateCount: 1 },
  ]);

  assert.equal(model.changeCount, 2);
  assert.equal(model.graphCommitCount, 0);
  assert.equal(model.branchLabel, "Auto");
  assert.deepEqual(model.stagedChanges, []);
  assert.deepEqual(model.unstagedChanges, []);
});

test("buildGitPanelViewModel partitions changes and counts only commit graph rows", () => {
  const staged = {
    path: "src/staged.ts",
    originalPath: null,
    stagedStatus: "M",
    unstagedStatus: null,
    staged: true,
    unstaged: false,
  };
  const unstaged = {
    path: "src/unstaged.ts",
    originalPath: null,
    stagedStatus: null,
    unstagedStatus: "M",
    staged: false,
    unstaged: true,
  };
  const both = {
    path: "src/both.ts",
    originalPath: null,
    stagedStatus: "A",
    unstagedStatus: "M",
    staged: true,
    unstaged: true,
  };

  const model = buildGitPanelViewModel(
    gitSnapshot({
      selectedRef: "feature/demo",
      graph: [
        {
          type: "commit",
          graph: "* ",
          hash: "abc123",
          shortHash: "abc123",
          parents: [],
          refs: [],
          subject: "Initial",
          author: "Alice",
          relativeTime: "now",
        },
        { type: "connector", graph: "|/" },
      ],
      changes: [staged, unstaged, both],
    }),
    [],
  );

  assert.equal(model.changeCount, 3);
  assert.equal(model.graphCommitCount, 1);
  assert.equal(model.branchLabel, "feature/demo");
  assert.deepEqual(model.stagedChanges, [staged, both]);
  assert.deepEqual(model.unstagedChanges, [unstaged, both]);
});

test("isGitGraphCommit narrows connector rows out of git graph data", () => {
  assert.equal(isGitGraphCommit({ type: "connector", graph: "|/" }), false);
  assert.equal(
    isGitGraphCommit({
      type: "commit",
      graph: "* ",
      hash: "abc123",
      shortHash: "abc123",
      parents: [],
      refs: [],
      subject: "Initial",
      author: "Alice",
      relativeTime: "now",
    }),
    true,
  );
});
