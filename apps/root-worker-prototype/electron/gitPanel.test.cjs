const test = require("node:test");
const assert = require("node:assert/strict");

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
      graph: "| * ",
      hash: "def456",
      shortHash: "def456",
      parents: [],
      refs: ["feature/demo"],
      subject: "Add demo",
      author: "Bob",
      relativeTime: "yesterday",
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
