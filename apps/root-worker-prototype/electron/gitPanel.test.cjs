const test = require("node:test");
const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  GIT_GRAPH_MAX_COUNT,
  GIT_REF_MAX_COUNT,
  buildGitCommitFilesArgs,
  buildGitLogArgs,
  buildGitRefsArgs,
  isValidCommitHash,
  parseGitCommitFiles,
  parseGitGraph,
  parseGitRefs,
  parseGitStatus,
  readGitFileDiff,
} = require("./gitPanel.cjs");

test("parseGitGraph reads git graph commit records", () => {
  const graph = parseGitGraph(
    [
      "* \x1fabc123\x1fabc123\x1fdef456 fed789\x1fHEAD -> main, origin/main\x1fMerge branch 'feature'\x1fAlice\x1f2 hours ago",
      "| * \x1fdef456\x1fdef456\x1f\x1ffeature/demo\x1fAdd demo\x1fBob\x1fyesterday",
      "|/",
    ].join("\n"),
  );

  assert.deepEqual(graph, [
    {
      type: "commit",
      graph: "* ",
      hash: "abc123",
      shortHash: "abc123",
      parents: ["def456", "fed789"],
      refs: ["HEAD -> main", "origin/main"],
      subject: "Merge branch 'feature'",
      author: "Alice",
      relativeTime: "2 hours ago",
    },
    {
      type: "commit",
      graph: "| * ",
      hash: "def456",
      shortHash: "def456",
      parents: [],
      refs: ["feature/demo"],
      subject: "Add demo",
      author: "Bob",
      relativeTime: "yesterday",
    },
    {
      type: "connector",
      graph: "|/",
    },
  ]);
});

test("parseGitGraph matches the git log pretty format used by readGitSnapshot", () => {
  const graph = parseGitGraph(
    "* \x1fabc123\x1fabc123\x1fparent1 parent2\x1fHEAD -> feature/demo\x1fFix graph panel\x1fAlice\x1f1 minute ago",
  );

  assert.equal(graph[0].subject, "Fix graph panel");
  assert.equal(graph[0].author, "Alice");
  assert.equal(graph[0].relativeTime, "1 minute ago");
  assert.deepEqual(graph[0].parents, ["parent1", "parent2"]);
  assert.deepEqual(graph[0].refs, ["HEAD -> feature/demo"]);
});

test("parseGitGraph preserves connector-only topology rows", () => {
  const graph = parseGitGraph(
    [
      "| * \x1fdef456\x1fdef456\x1f\x1ffeature/demo\x1fAdd demo\x1fBob\x1fyesterday",
      "|\\",
      "| |",
      "|/",
    ].join("\n"),
  );

  assert.deepEqual(graph.slice(1), [
    { type: "connector", graph: "|\\" },
    { type: "connector", graph: "| |" },
    { type: "connector", graph: "|/" },
  ]);
});

test("buildGitLogArgs keeps the git graph history bounded", () => {
  const args = buildGitLogArgs();

  assert.equal(GIT_GRAPH_MAX_COUNT, 120);
  assert.ok(args.includes("--graph"));
  assert.ok(args.includes("--decorate=short"));
  assert.ok(args.includes(`--max-count=${GIT_GRAPH_MAX_COUNT}`));
});

test("buildGitLogArgs can target a selected ref without shell interpolation", () => {
  const args = buildGitLogArgs("origin/feature/demo");

  assert.equal(args.at(-1), "origin/feature/demo");
  assert.ok(args.includes(`--max-count=${GIT_GRAPH_MAX_COUNT}`));
});

test("buildGitRefsArgs keeps branch selection refs bounded", () => {
  const args = buildGitRefsArgs();

  assert.equal(GIT_REF_MAX_COUNT, 200);
  assert.ok(args.includes(`--count=${GIT_REF_MAX_COUNT}`));
  assert.ok(args.includes("refs/heads"));
  assert.ok(args.includes("refs/remotes"));
  assert.ok(args.includes("refs/tags"));
});

test("parseGitRefs dedupes refs and keeps the current head first", () => {
  const refs = parseGitRefs(
    [
      "feature/demo\0refs/heads/feature/demo\0",
      "main\0refs/heads/main\0*",
      "origin/HEAD\0refs/remotes/origin/HEAD\0",
      "origin/main\0refs/remotes/origin/main\0",
      "main\0refs/heads/main\0*",
    ].join("\n"),
  );

  assert.deepEqual(refs, [
    { name: "main", fullName: "refs/heads/main", head: true },
    { name: "feature/demo", fullName: "refs/heads/feature/demo", head: false },
    { name: "origin/main", fullName: "refs/remotes/origin/main", head: false },
  ]);
});

test("parseGitCommitFiles reads name-status output including renames and copies", () => {
  const files = parseGitCommitFiles(
    [
      "M",
      "src/app.ts",
      "A",
      "README.md",
      "R100",
      "src/old.ts",
      "src/new.ts",
      "C80",
      "src/base.ts",
      "src/copy.ts",
      "",
    ].join("\0"),
  );

  assert.deepEqual(files, [
    { path: "src/app.ts", originalPath: null, status: "M", score: null },
    { path: "README.md", originalPath: null, status: "A", score: null },
    { path: "src/new.ts", originalPath: "src/old.ts", status: "R", score: "100" },
    { path: "src/copy.ts", originalPath: "src/base.ts", status: "C", score: "80" },
  ]);
});

test("buildGitCommitFilesArgs validates the command shape for one commit", () => {
  const args = buildGitCommitFilesArgs("abc1234");

  assert.deepEqual(args, [
    "show",
    "--name-status",
    "--format=",
    "--find-renames",
    "--find-copies",
    "-z",
    "abc1234",
  ]);
  assert.equal(isValidCommitHash("abc1234"), true);
  assert.equal(isValidCommitHash("--bad"), false);
});

test("parseGitStatus groups staged and unstaged porcelain entries", () => {
  const changes = parseGitStatus(
    [
      " M src/app.ts",
      "M  src/index.ts",
      "AM src/both.ts",
      "?? README.md",
      "R  src/new.ts",
      "src/old.ts",
      "",
    ].join("\0"),
  );

  assert.deepEqual(changes, [
    {
      path: "src/app.ts",
      originalPath: null,
      stagedStatus: null,
      unstagedStatus: "M",
      staged: false,
      unstaged: true,
    },
    {
      path: "src/index.ts",
      originalPath: null,
      stagedStatus: "M",
      unstagedStatus: null,
      staged: true,
      unstaged: false,
    },
    {
      path: "src/both.ts",
      originalPath: null,
      stagedStatus: "A",
      unstagedStatus: "M",
      staged: true,
      unstaged: true,
    },
    {
      path: "README.md",
      originalPath: null,
      stagedStatus: "?",
      unstagedStatus: "?",
      staged: false,
      unstaged: true,
    },
    {
      path: "src/new.ts",
      originalPath: "src/old.ts",
      stagedStatus: "R",
      unstagedStatus: null,
      staged: true,
      unstaged: false,
    },
  ]);
});

test("parseGitStatus reads unstaged rename entries", () => {
  const changes = parseGitStatus([" R src/new.ts", "src/old.ts", ""].join("\0"));

  assert.deepEqual(changes, [
    {
      path: "src/new.ts",
      originalPath: "src/old.ts",
      stagedStatus: null,
      unstagedStatus: "R",
      staged: false,
      unstaged: true,
    },
  ]);
});

test("readGitFileDiff reads unstaged modified files from index to working tree", async (t) => {
  const repo = createTempGitRepo(t);
  writeRepoFile(repo, "src/app.ts", "export const value = 1;\n");
  git(repo, ["add", "src/app.ts"]);
  git(repo, ["commit", "-m", "initial"]);
  writeRepoFile(repo, "src/app.ts", "export const value = 2;\n");

  const diff = await readGitFileDiff(repo, { path: "src/app.ts", staged: false });

  assert.equal(diff.available, true);
  assert.equal(diff.staged, false);
  assert.equal(diff.status, "M");
  assert.equal(diff.oldContent, "export const value = 1;\n");
  assert.equal(diff.newContent, "export const value = 2;\n");
  assert.equal(diff.oldLabel, "Index");
  assert.equal(diff.newLabel, "Working tree");
});

test("readGitFileDiff reads staged modified files from HEAD to index", async (t) => {
  const repo = createTempGitRepo(t);
  writeRepoFile(repo, "src/app.ts", "export const value = 1;\n");
  git(repo, ["add", "src/app.ts"]);
  git(repo, ["commit", "-m", "initial"]);
  writeRepoFile(repo, "src/app.ts", "export const value = 2;\n");
  git(repo, ["add", "src/app.ts"]);

  const diff = await readGitFileDiff(repo, { path: "src/app.ts", staged: true });

  assert.equal(diff.available, true);
  assert.equal(diff.staged, true);
  assert.equal(diff.status, "M");
  assert.equal(diff.oldContent, "export const value = 1;\n");
  assert.equal(diff.newContent, "export const value = 2;\n");
  assert.equal(diff.oldLabel, "HEAD");
  assert.equal(diff.newLabel, "Index");
});

test("readGitFileDiff handles added, deleted, and renamed staged files", async (t) => {
  const repo = createTempGitRepo(t);
  writeRepoFile(repo, "src/delete-me.ts", "delete me\n");
  writeRepoFile(repo, "src/old-name.ts", "rename me\n");
  git(repo, ["add", "src/delete-me.ts", "src/old-name.ts"]);
  git(repo, ["commit", "-m", "initial"]);

  writeRepoFile(repo, "src/added.ts", "new file\n");
  git(repo, ["rm", "src/delete-me.ts"]);
  git(repo, ["mv", "src/old-name.ts", "src/new-name.ts"]);
  git(repo, ["add", "src/added.ts"]);

  const added = await readGitFileDiff(repo, { path: "src/added.ts", staged: true });
  const deleted = await readGitFileDiff(repo, { path: "src/delete-me.ts", staged: true });
  const renamed = await readGitFileDiff(repo, {
    path: "src/new-name.ts",
    originalPath: "src/old-name.ts",
    staged: true,
  });

  assert.equal(added.status, "A");
  assert.equal(added.oldContent, "");
  assert.equal(added.newContent, "new file\n");
  assert.equal(deleted.status, "D");
  assert.equal(deleted.oldContent, "delete me\n");
  assert.equal(deleted.newContent, "");
  assert.equal(renamed.status, "R");
  assert.equal(renamed.originalPath, "src/old-name.ts");
  assert.equal(renamed.oldContent, "rename me\n");
  assert.equal(renamed.newContent, "rename me\n");
});

test("readGitFileDiff returns a typed unavailable diff for binary content", async (t) => {
  const repo = createTempGitRepo(t);
  fs.mkdirSync(path.join(repo, "src"), { recursive: true });
  fs.writeFileSync(path.join(repo, "src/blob.bin"), Buffer.from([0, 1, 2, 3]));
  git(repo, ["add", "src/blob.bin"]);

  const diff = await readGitFileDiff(repo, { path: "src/blob.bin", staged: true });

  assert.equal(diff.available, false);
  assert.equal(diff.binary, true);
  assert.equal(diff.status, "A");
  assert.match(diff.error ?? "", /Binary files/);
});

function createTempGitRepo(t) {
  requireGit(t);
  const repo = fs.mkdtempSync(path.join(os.tmpdir(), "morpheus-git-panel-"));
  t.after(() => {
    fs.rmSync(repo, { force: true, recursive: true });
  });
  git(repo, ["init"]);
  git(repo, ["config", "user.email", "test@example.com"]);
  git(repo, ["config", "user.name", "Test User"]);
  return repo;
}

function requireGit(t) {
  try {
    execFileSync("git", ["--version"], { stdio: "ignore" });
  } catch {
    t.skip("git is required for readGitFileDiff integration coverage");
  }
}

function writeRepoFile(repo, relativePath, content) {
  const target = path.join(repo, relativePath);
  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.writeFileSync(target, content);
}

function git(repo, args) {
  return execFileSync("git", args, {
    cwd: repo,
    encoding: "utf8",
    env: {
      ...process.env,
      GIT_AUTHOR_DATE: "2026-01-01T00:00:00Z",
      GIT_COMMITTER_DATE: "2026-01-01T00:00:00Z",
    },
  });
}
