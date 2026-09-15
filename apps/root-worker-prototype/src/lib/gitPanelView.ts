import type { ChangedFileSummary } from "./threadAnalysis";

export type GitSnapshot = Awaited<
  ReturnType<Window["codexDesktop"]["readGitSnapshot"]>
>;
export type GitChange = GitSnapshot["changes"][number];
export type GitGraphItem = GitSnapshot["graph"][number];
export type GitGraphCommit = Extract<GitGraphItem, { type: "commit" }>;

export type GitPanelViewModel = {
  stagedChanges: GitChange[];
  unstagedChanges: GitChange[];
  changeCount: number;
  graphCommitCount: number;
  branchLabel: string;
};

export function buildGitPanelViewModel(
  snapshot: GitSnapshot | null,
  changedFiles: ChangedFileSummary[],
): GitPanelViewModel {
  if (!snapshot) {
    return {
      stagedChanges: [],
      unstagedChanges: [],
      changeCount: changedFiles.length,
      graphCommitCount: 0,
      branchLabel: "Auto",
    };
  }

  const stagedChanges: GitChange[] = [];
  const unstagedChanges: GitChange[] = [];
  for (const change of snapshot.changes) {
    if (change.staged) {
      stagedChanges.push(change);
    }
    if (change.unstaged) {
      unstagedChanges.push(change);
    }
  }

  return {
    stagedChanges,
    unstagedChanges,
    changeCount: snapshot.changes.length,
    graphCommitCount: countGitGraphCommits(snapshot.graph),
    branchLabel: snapshot.selectedRef ?? snapshot.branch ?? "Auto",
  };
}

export function isGitGraphCommit(
  item: GitGraphItem,
): item is GitGraphCommit {
  return item.type === "commit";
}

function countGitGraphCommits(graph: GitGraphItem[]) {
  let count = 0;
  for (const item of graph) {
    if (isGitGraphCommit(item)) {
      count += 1;
    }
  }
  return count;
}
