const { execFile } = require("node:child_process");
const { promisify } = require("node:util");

const execFileAsync = promisify(execFile);
const GIT_TIMEOUT_MS = 5000;
const GIT_GRAPH_MAX_COUNT = 120;
const GIT_REF_MAX_COUNT = 200;
const GRAPH_RECORD_SEPARATOR = "\x1f";

async function readGitSnapshot(cwd, options = {}) {
  if (typeof cwd !== "string" || !cwd.trim()) {
    return unavailableSnapshot("No workspace is selected.");
  }

  const rootResult = await runGit(cwd, ["rev-parse", "--show-toplevel"]);
  if (!rootResult.ok) {
    return unavailableSnapshot("This workspace is not a Git repository.");
  }

  const root = rootResult.stdout.trim();
  const [branchResult, refsResult, statusResult] = await Promise.all([
    runGit(root, ["branch", "--show-current"]),
    runGit(root, buildGitRefsArgs()),
    runGit(root, ["status", "--porcelain=v1", "-z"]),
  ]);
  const refs = refsResult.ok ? parseGitRefs(refsResult.stdout) : [];
  const requestedRef = normalizeSelectedGitRef(options?.ref);
  const selectedRef =
    requestedRef && refs.some((ref) => ref.name === requestedRef) ? requestedRef : null;
  const graphResult = await runGit(root, buildGitLogArgs(selectedRef));

  return {
    available: true,
    root,
    branch: branchResult.ok ? branchResult.stdout.trim() || null : null,
    selectedRef,
    refs,
    graph: graphResult.ok ? parseGitGraph(graphResult.stdout) : [],
    changes: statusResult.ok ? parseGitStatus(statusResult.stdout) : [],
    error: graphResult.ok && statusResult.ok ? null : "Git snapshot is incomplete.",
  };
}

async function readGitCommitFiles(cwd, hash) {
  if (typeof cwd !== "string" || !cwd.trim()) {
    return unavailableCommitFiles("No workspace is selected.");
  }
  if (!isValidCommitHash(hash)) {
    return unavailableCommitFiles("Invalid commit hash.");
  }

  const rootResult = await runGit(cwd, ["rev-parse", "--show-toplevel"]);
  if (!rootResult.ok) {
    return unavailableCommitFiles("This workspace is not a Git repository.");
  }

  const result = await runGit(rootResult.stdout.trim(), buildGitCommitFilesArgs(hash));
  if (!result.ok) {
    return unavailableCommitFiles("Failed to read commit files.");
  }

  return {
    available: true,
    files: parseGitCommitFiles(result.stdout),
    error: null,
  };
}

function buildGitLogArgs(ref = null) {
  const args = [
    "log",
    "--graph",
    "--date-order",
    "--decorate=short",
    "--pretty=format:%x1f%H%x1f%h%x1f%P%x1f%D%x1f%s%x1f%an%x1f%cr",
    "--abbrev-commit",
    `--max-count=${GIT_GRAPH_MAX_COUNT}`,
  ];
  if (ref) {
    args.push(ref);
  }
  return args;
}

function buildGitRefsArgs() {
  return [
    "for-each-ref",
    "--sort=refname",
    `--count=${GIT_REF_MAX_COUNT}`,
    "--format=%(refname:short)%00%(refname)%00%(HEAD)",
    "refs/heads",
    "refs/remotes",
    "refs/tags",
  ];
}

function buildGitCommitFilesArgs(hash) {
  return [
    "show",
    "--name-status",
    "--format=",
    "--find-renames",
    "--find-copies",
    "-z",
    hash,
  ];
}

function unavailableSnapshot(reason) {
  return {
    available: false,
    root: null,
    branch: null,
    selectedRef: null,
    refs: [],
    graph: [],
    changes: [],
    error: reason,
  };
}

function unavailableCommitFiles(reason) {
  return {
    available: false,
    files: [],
    error: reason,
  };
}

function isValidCommitHash(value) {
  return typeof value === "string" && /^[0-9a-fA-F]{7,64}$/.test(value);
}

function normalizeSelectedGitRef(value) {
  if (typeof value !== "string" || !value.trim()) {
    return null;
  }
  const ref = value.trim();
  if (
    ref.length > 200 ||
    ref.startsWith("-") ||
    ref.includes("..") ||
    ref.includes("//") ||
    /[\s~^:?*[\\\x00-\x1f\x7f]/.test(ref)
  ) {
    return null;
  }
  return ref;
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
    return parseGitGraphConnectorLine(line);
  }

  const graph = line.slice(0, recordIndex);
  const fields = line.slice(recordIndex + 1).split(GRAPH_RECORD_SEPARATOR);
  if (fields.length < 6 || !fields[0]) {
    return null;
  }

  return {
    type: "commit",
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

function parseGitGraphConnectorLine(line) {
  if (!/[|/\\_\-]/.test(line)) {
    return null;
  }
  return {
    type: "connector",
    graph: line,
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

function parseGitRefs(stdout) {
  const refs = [];
  const seen = new Set();

  for (const line of stdout.split(/\r?\n/)) {
    if (!line) {
      continue;
    }
    const [name, fullName, headMarker] = line.split("\0");
    if (!name || seen.has(name) || name.endsWith("/HEAD")) {
      continue;
    }
    seen.add(name);
    refs.push({
      name,
      fullName: fullName || name,
      head: headMarker === "*",
    });
  }

  return refs.sort((left, right) => {
    if (left.head !== right.head) {
      return left.head ? -1 : 1;
    }
    return left.name.localeCompare(right.name);
  });
}

function parseGitCommitFiles(stdout) {
  const entries = stdout.split("\0").filter(Boolean);
  const files = [];

  for (let index = 0; index < entries.length; index += 1) {
    const status = entries[index];
    if (!status) {
      continue;
    }

    if (status.startsWith("R") || status.startsWith("C")) {
      const originalPath = entries[index + 1] ?? null;
      const path = entries[index + 2] ?? "";
      if (path) {
        files.push({
          path,
          originalPath,
          status: status[0],
          score: status.length > 1 ? status.slice(1) : null,
        });
      }
      index += 2;
      continue;
    }

    const path = entries[index + 1] ?? "";
    if (path) {
      files.push({
        path,
        originalPath: null,
        status: status[0],
        score: status.length > 1 ? status.slice(1) : null,
      });
    }
    index += 1;
  }

  return files;
}

module.exports = {
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
  readGitCommitFiles,
  readGitSnapshot,
};
