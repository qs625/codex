const { execFile } = require("node:child_process");
const { promisify } = require("node:util");

const execFileAsync = promisify(execFile);
const GIT_TIMEOUT_MS = 5000;
const GIT_GRAPH_MAX_COUNT = 120;
const GRAPH_RECORD_SEPARATOR = "\x1f";

async function readGitSnapshot(cwd) {
  if (typeof cwd !== "string" || !cwd.trim()) {
    return unavailableSnapshot("No workspace is selected.");
  }

  const rootResult = await runGit(cwd, ["rev-parse", "--show-toplevel"]);
  if (!rootResult.ok) {
    return unavailableSnapshot("This workspace is not a Git repository.");
  }

  const root = rootResult.stdout.trim();
  const [branchResult, graphResult, statusResult] = await Promise.all([
    runGit(root, ["branch", "--show-current"]),
    runGit(root, buildGitLogArgs()),
    runGit(root, ["status", "--porcelain=v1", "-z"]),
  ]);

  return {
    available: true,
    root,
    branch: branchResult.ok ? branchResult.stdout.trim() || null : null,
    graph: graphResult.ok ? parseGitGraph(graphResult.stdout) : [],
    changes: statusResult.ok ? parseGitStatus(statusResult.stdout) : [],
    error: graphResult.ok && statusResult.ok ? null : "Git snapshot is incomplete.",
  };
}

function buildGitLogArgs() {
  return [
    "log",
    "--graph",
    "--date-order",
    "--decorate=short",
    "--pretty=format:%x1f%H%x1f%h%x1f%P%x1f%D%x1f%s%x1f%an%x1f%cr",
    "--abbrev-commit",
    `--max-count=${GIT_GRAPH_MAX_COUNT}`,
  ];
}

function unavailableSnapshot(reason) {
  return {
    available: false,
    root: null,
    branch: null,
    graph: [],
    changes: [],
    error: reason,
  };
}

async function runGit(cwd, args) {
  try {
    const { stdout } = await execFileAsync("git", args, {
      cwd,
      encoding: "utf8",
      maxBuffer: 1024 * 1024,
      timeout: GIT_TIMEOUT_MS,
    });
    return { ok: true, stdout };
  } catch (error) {
    return {
      ok: false,
      stdout: typeof error?.stdout === "string" ? error.stdout : "",
      stderr: typeof error?.stderr === "string" ? error.stderr : "",
    };
  }
}

function parseGitGraph(stdout) {
  return stdout
    .split(/\r?\n/)
    .map((line) => parseGitGraphLine(line))
    .filter(Boolean);
}

function parseGitGraphLine(line) {
  const recordIndex = line.indexOf(GRAPH_RECORD_SEPARATOR);
  if (recordIndex < 0) {
    return null;
  }

  const graph = line.slice(0, recordIndex);
  const fields = line.slice(recordIndex + 1).split(GRAPH_RECORD_SEPARATOR);
  if (fields.length < 6 || !fields[0]) {
    return null;
  }

  return {
    graph,
    hash: fields[0],
    shortHash: fields[1],
    parents: fields[2] ? fields[2].split(" ").filter(Boolean) : [],
    refs: fields[3] ? fields[3].split(", ").filter(Boolean) : [],
    subject: fields[4],
    author: fields[5],
    relativeTime: fields[6] ?? "",
  };
}

function parseGitStatus(stdout) {
  const entries = stdout.split("\0").filter(Boolean);
  const changes = [];

  for (let index = 0; index < entries.length; index += 1) {
    const entry = entries[index];
    if (entry.length < 4) {
      continue;
    }

    const stagedCode = entry[0];
    const unstagedCode = entry[1];
    const path = entry.slice(3);
    const renamed = stagedCode === "R" || stagedCode === "C";
    const originalPath = renamed ? entries[index + 1] ?? null : null;
    if (renamed) {
      index += 1;
    }

    changes.push({
      path,
      originalPath,
      stagedStatus: stagedCode === " " ? null : stagedCode,
      unstagedStatus: unstagedCode === " " ? null : unstagedCode,
      staged: stagedCode !== " " && stagedCode !== "?",
      unstaged: unstagedCode !== " " || stagedCode === "?",
    });
  }

  return changes;
}

module.exports = {
  GIT_GRAPH_MAX_COUNT,
  buildGitLogArgs,
  parseGitGraph,
  parseGitStatus,
  readGitSnapshot,
};
