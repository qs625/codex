const { execFile } = require("node:child_process");
const fs = require("node:fs/promises");
const path = require("node:path");
const { promisify } = require("node:util");

const execFileAsync = promisify(execFile);
const GIT_TIMEOUT_MS = 5000;
const GIT_GRAPH_MAX_COUNT = 120;
const GIT_REF_MAX_COUNT = 200;
const GIT_DIFF_TEXT_MAX_BYTES = 1024 * 1024;
const GRAPH_RECORD_SEPARATOR = "\x1f";

async function readGitSnapshot(cwd, options = {}) {
  if (typeof cwd !== "string" || !cwd.trim()) {
    return unavailableSnapshot("No workspace is selected.");
  }

  const rootResult = await runGit(cwd, ["rev-parse", "--show-toplevel"]);
  if (!rootResult.ok) {
    return unavailableSnapshot("This workspace is not a Git repository.");
  }

  const { root, treeRoot } = await gitRootInfoForCwd(cwd, rootResult.stdout.trim());
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
    treeRoot,
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

async function readGitStatusSnapshot(cwd) {
  if (typeof cwd !== "string" || !cwd.trim()) {
    return unavailableStatusSnapshot("No workspace is selected.");
  }

  const rootResult = await runGit(cwd, ["rev-parse", "--show-toplevel"]);
  if (!rootResult.ok) {
    return unavailableStatusSnapshot("This workspace is not a Git repository.");
  }

  const { root, treeRoot } = await gitRootInfoForCwd(cwd, rootResult.stdout.trim());
  const statusResult = await runGit(root, ["status", "--porcelain=v1", "-z"]);
  if (!statusResult.ok) {
    return unavailableStatusSnapshot("Failed to read Git status.", root, treeRoot);
  }

  return {
    available: true,
    root,
    treeRoot,
    changes: parseGitStatus(statusResult.stdout),
    error: null,
  };
}

async function readGitFileDiff(cwd, options = {}) {
  if (typeof cwd !== "string" || !cwd.trim()) {
    return unavailableFileDiff("No workspace is selected.");
  }

  const mode = options?.staged ? "staged" : "unstaged";
  const requestedPath = normalizeGitPath(options?.path);
  const requestedOriginalPath = normalizeGitPath(options?.originalPath);
  if (!requestedPath) {
    return unavailableFileDiff("Invalid file path.");
  }

  const rootResult = await runGit(cwd, ["rev-parse", "--show-toplevel"]);
  if (!rootResult.ok) {
    return unavailableFileDiff("This workspace is not a Git repository.");
  }
  const root = rootResult.stdout.trim();

  const statusResult = await runGit(root, ["status", "--porcelain=v1", "-z"]);
  if (!statusResult.ok) {
    return unavailableFileDiff("Failed to read Git status.");
  }

  const changes = parseGitStatus(statusResult.stdout);
  const change = changes.find((entry) =>
    entry.path === requestedPath &&
    (requestedOriginalPath === null || entry.originalPath === requestedOriginalPath) &&
    (mode === "staged" ? entry.staged : entry.unstaged),
  );
  if (!change) {
    return unavailableFileDiff("This file is no longer present in the Git changes list.", {
      root,
      path: requestedPath,
      originalPath: requestedOriginalPath,
      staged: mode === "staged",
    });
  }

  const status =
    (mode === "staged" ? change.stagedStatus : change.unstagedStatus) ??
    change.stagedStatus ??
    change.unstagedStatus ??
    "M";
  const originalPath = change.originalPath ?? null;
  const oldPath = originalPath && (status === "R" || status === "C") ? originalPath : change.path;
  const unifiedDiff = await readUnifiedDiff(root, change, mode);
  const base = {
    available: true,
    root,
    path: change.path,
    originalPath,
    staged: mode === "staged",
    status,
    language: languageFromPath(change.path),
    oldLabel: mode === "staged" ? "HEAD" : "Index",
    newLabel: mode === "staged" ? "Index" : "Working tree",
    unifiedDiff,
    error: null,
    binary: false,
  };

  try {
    const oldContent =
      status === "A" || status === "?"
        ? ""
        : mode === "staged"
          ? await readGitTextObject(root, `HEAD:${oldPath}`)
          : await readGitTextObject(root, `:${oldPath}`);
    const newContent =
      status === "D"
        ? ""
        : mode === "staged"
          ? await readGitTextObject(root, `:${change.path}`)
          : await readWorkingTreeText(root, change.path);

    return {
      ...base,
      oldContent,
      newContent,
    };
  } catch (error) {
    return {
      ...base,
      available: false,
      oldContent: "",
      newContent: "",
      error: error instanceof Error ? error.message : "Failed to read file diff.",
      binary: isBinaryReadError(error),
    };
  }
}

async function readGitCommitFileDiff(cwd, options = {}) {
  if (typeof cwd !== "string" || !cwd.trim()) {
    return unavailableFileDiff("No workspace is selected.");
  }

  const hash = typeof options?.hash === "string" ? options.hash.trim() : "";
  if (!isValidCommitHash(hash)) {
    return unavailableFileDiff("Invalid commit hash.");
  }
  const requestedPath = normalizeGitPath(options?.path);
  const requestedOriginalPath = normalizeGitPath(options?.originalPath);
  if (!requestedPath) {
    return unavailableFileDiff("Invalid file path.");
  }

  const rootResult = await runGit(cwd, ["rev-parse", "--show-toplevel"]);
  if (!rootResult.ok) {
    return unavailableFileDiff("This workspace is not a Git repository.");
  }
  const root = rootResult.stdout.trim();
  const commitFilesResult = await runGit(root, buildGitCommitFilesArgs(hash));
  if (!commitFilesResult.ok) {
    return unavailableFileDiff("Failed to read commit files.", {
      root,
      path: requestedPath,
      originalPath: requestedOriginalPath,
      commit: hash,
    });
  }

  const files = parseGitCommitFiles(commitFilesResult.stdout);
  const file = files.find(
    (entry) =>
      entry.path === requestedPath &&
      (requestedOriginalPath === null || entry.originalPath === requestedOriginalPath),
  );
  if (!file) {
    return unavailableFileDiff("This file is not present in the selected commit.", {
      root,
      path: requestedPath,
      originalPath: requestedOriginalPath,
      commit: hash,
    });
  }

  const status = file.status || "M";
  const originalPath = file.originalPath ?? null;
  const oldPath = originalPath && (status === "R" || status === "C") ? originalPath : file.path;
  const shortHash = hash.slice(0, 7);
  const unifiedDiff = await readCommitUnifiedDiff(root, hash, file);
  const base = {
    available: true,
    root,
    path: file.path,
    originalPath,
    staged: false,
    status,
    language: languageFromPath(file.path),
    oldLabel: `${shortHash}^`,
    newLabel: shortHash,
    modeLabel: "commit",
    commit: hash,
    parent: `${hash}^`,
    unifiedDiff,
    error: null,
    binary: false,
  };

  try {
    const oldContent = status === "A" ? "" : await readGitTextObject(root, `${hash}^:${oldPath}`);
    const newContent = status === "D" ? "" : await readGitTextObject(root, `${hash}:${file.path}`);
    return {
      ...base,
      oldContent,
      newContent,
    };
  } catch (error) {
    return {
      ...base,
      available: false,
      oldContent: "",
      newContent: "",
      error: error instanceof Error ? error.message : "Failed to read commit file diff.",
      binary: isBinaryReadError(error),
    };
  }
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

function buildGitCommitFileDiffArgs(hash, file) {
  const args = [
    "show",
    "--format=",
    "--no-ext-diff",
    "--find-renames",
    "--find-copies",
    hash,
    "--",
    file.path,
  ];
  if (file.originalPath) {
    args.push(file.originalPath);
  }
  return args;
}

function unavailableSnapshot(reason) {
  return {
    available: false,
    root: null,
    treeRoot: null,
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

function unavailableStatusSnapshot(reason, root = null, treeRoot = null) {
  return {
    available: false,
    root,
    treeRoot,
    changes: [],
    error: reason,
  };
}

async function gitRootInfoForCwd(cwd, root) {
  const prefixResult = await runGit(cwd, ["rev-parse", "--show-prefix"]);
  return {
    root,
    treeRoot: prefixResult.ok ? gitTreeRootForCwd(cwd, prefixResult.stdout.trim()) : root,
  };
}

function gitTreeRootForCwd(cwd, gitPrefix) {
  const cwdPath = path.resolve(cwd);
  const prefixParts = gitPrefix.split("/").filter(Boolean);
  if (prefixParts.length === 0) {
    return cwdPath;
  }
  return path.resolve(cwdPath, ...prefixParts.map(() => ".."));
}

function unavailableFileDiff(reason, context = {}) {
  return {
    available: false,
    root: context.root ?? null,
    path: context.path ?? null,
    originalPath: context.originalPath ?? null,
    staged: Boolean(context.staged),
    status: null,
    language: "plaintext",
    oldLabel: null,
    newLabel: null,
    oldContent: "",
    newContent: "",
    unifiedDiff: "",
    error: reason,
    binary: false,
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

async function runGitBuffer(cwd, args) {
  try {
    const { stdout } = await execFileAsync("git", args, {
      cwd,
      encoding: "buffer",
      maxBuffer: GIT_DIFF_TEXT_MAX_BYTES,
      timeout: GIT_TIMEOUT_MS,
    });
    return { ok: true, stdout };
  } catch (error) {
    return {
      ok: false,
      stdout: Buffer.isBuffer(error?.stdout) ? error.stdout : Buffer.alloc(0),
      stderr: Buffer.isBuffer(error?.stderr)
        ? error.stderr.toString("utf8")
        : typeof error?.stderr === "string"
          ? error.stderr
          : "",
      code: error?.code ?? null,
    };
  }
}

async function readGitTextObject(root, spec) {
  const result = await runGitBuffer(root, ["show", spec]);
  if (!result.ok) {
    throw new Error("Failed to read Git object content.");
  }
  return bufferToGitPreviewText(result.stdout);
}

async function readWorkingTreeText(root, gitPath) {
  const target = path.resolve(root, gitPath);
  const relative = path.relative(root, target);
  if (relative.startsWith("..") || path.isAbsolute(relative)) {
    throw new Error("Git path escapes the repository root.");
  }
  const stat = await fs.stat(target);
  if (!stat.isFile()) {
    throw new Error("Changed path is not a file.");
  }
  if (stat.size > GIT_DIFF_TEXT_MAX_BYTES) {
    throw new Error("File is too large to preview as a diff.");
  }
  return bufferToGitPreviewText(await fs.readFile(target));
}

function bufferToGitPreviewText(buffer) {
  if (buffer.includes(0)) {
    const error = new Error("Binary files cannot be previewed as side-by-side text.");
    error.code = "ERR_GIT_DIFF_BINARY";
    throw error;
  }
  return buffer.toString("utf8");
}

function isBinaryReadError(error) {
  return error?.code === "ERR_GIT_DIFF_BINARY";
}

async function readUnifiedDiff(root, change, mode) {
  const args =
    mode === "staged"
      ? ["diff", "--cached", "--no-ext-diff", "--find-renames", "--", change.path]
      : ["diff", "--no-ext-diff", "--find-renames", "--", change.path];
  if (change.originalPath) {
    args.push(change.originalPath);
  }
  const result = await runGit(root, args);
  return result.ok ? result.stdout : "";
}

async function readCommitUnifiedDiff(root, hash, file) {
  const result = await runGit(root, buildGitCommitFileDiffArgs(hash, file));
  return result.ok ? result.stdout : "";
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
    const renamed =
      stagedCode === "R" || stagedCode === "C" || unstagedCode === "R" || unstagedCode === "C";
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

function normalizeGitPath(value) {
  if (value === null || value === undefined) {
    return null;
  }
  if (typeof value !== "string") {
    return null;
  }
  const gitPath = value.trim();
  if (
    !gitPath ||
    gitPath.length > 4096 ||
    gitPath.startsWith("/") ||
    gitPath.startsWith("\\") ||
    /^[A-Za-z]:[\\/]/.test(gitPath) ||
    gitPath.split(/[\\/]/).some((part) => part === "..") ||
    /[\x00-\x1f\x7f]/.test(gitPath)
  ) {
    return null;
  }
  return gitPath;
}

function languageFromPath(filePath) {
  const extension = filePath.split(".").pop()?.toLowerCase() ?? "";
  switch (extension) {
    case "cjs":
    case "js":
    case "mjs":
      return "javascript";
    case "css":
      return "css";
    case "go":
      return "go";
    case "html":
      return "html";
    case "json":
      return "json";
    case "md":
    case "mdx":
      return "markdown";
    case "py":
      return "python";
    case "rs":
      return "rust";
    case "sh":
      return "shell";
    case "tsx":
      return "typescript";
    case "ts":
      return "typescript";
    case "xml":
      return "xml";
    case "yaml":
    case "yml":
      return "yaml";
    default:
      return "plaintext";
  }
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
  GIT_DIFF_TEXT_MAX_BYTES,
  buildGitCommitFilesArgs,
  buildGitCommitFileDiffArgs,
  buildGitLogArgs,
  buildGitRefsArgs,
  isValidCommitHash,
  languageFromPath,
  parseGitCommitFiles,
  parseGitGraph,
  parseGitRefs,
  parseGitStatus,
  readGitFileDiff,
  readGitCommitFileDiff,
  readGitCommitFiles,
  readGitStatusSnapshot,
  readGitSnapshot,
};
