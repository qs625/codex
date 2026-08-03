const path = require("node:path");
const { spawnSync } = require("node:child_process");

const DEFAULT_SHELL_PATH_TIMEOUT_MS = 750;
const MACOS_COMMON_PATHS = [
  "/opt/homebrew/bin",
  "/opt/homebrew/sbin",
  "/usr/local/bin",
  "/usr/local/sbin",
  "/usr/bin",
  "/bin",
  "/usr/sbin",
  "/sbin",
];

const SHELL_PATH_CACHE = new Map();

function buildDesktopEnvironment(baseEnv = process.env, options = {}) {
  const env = { ...baseEnv };
  env.PATH = buildDesktopPath(baseEnv.PATH, {
    ...options,
    baseEnv,
    home: options.home ?? baseEnv.HOME,
    shell: options.shell ?? baseEnv.SHELL,
  });
  return env;
}

function buildDesktopPath(basePath, options = {}) {
  const platform = options.platform ?? process.platform;
  const baseEntries = splitPath(basePath);

  if (platform !== "darwin") {
    return joinPathEntries(baseEntries);
  }

  const shellPath =
    options.includeShellPath === false
      ? ""
      : (options.shellPath ?? readLoginShellPathSync(options));

  return joinPathEntries([
    ...baseEntries,
    ...splitPath(shellPath),
    ...macosUserPaths(options.home),
    ...MACOS_COMMON_PATHS,
  ]);
}

function readLoginShellPathSync(options = {}) {
  const platform = options.platform ?? process.platform;
  if (platform !== "darwin") {
    return "";
  }

  if (typeof options.readShellPath === "function") {
    try {
      return options.readShellPath() ?? "";
    } catch {
      return "";
    }
  }

  const shell = usableShell(options.shell) ?? "/bin/zsh";
  const baseEnv = options.baseEnv ?? process.env;
  const cacheKey = [
    shell,
    baseEnv.HOME ?? "",
    baseEnv.PATH ?? "",
    options.timeoutMs ?? DEFAULT_SHELL_PATH_TIMEOUT_MS,
  ].join("\0");

  if (SHELL_PATH_CACHE.has(cacheKey)) {
    return SHELL_PATH_CACHE.get(cacheKey);
  }

  let result;
  try {
    result = spawnSync(shell, ["-lc", 'printf "%s" "$PATH"'], {
      env: baseEnv,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
      timeout: options.timeoutMs ?? DEFAULT_SHELL_PATH_TIMEOUT_MS,
    });
  } catch {
    SHELL_PATH_CACHE.set(cacheKey, "");
    return "";
  }
  const shellPath =
    result.status === 0 && typeof result.stdout === "string"
      ? result.stdout
      : "";
  SHELL_PATH_CACHE.set(cacheKey, shellPath);
  return shellPath;
}

function macosUserPaths(home) {
  if (typeof home !== "string" || !home.trim()) {
    return [];
  }

  return [
    path.join(home, ".local/bin"),
    path.join(home, "bin"),
    path.join(home, ".cargo/bin"),
    path.join(home, ".bun/bin"),
    path.join(home, ".deno/bin"),
    path.join(home, "go/bin"),
    path.join(home, ".pyenv/shims"),
    path.join(home, ".rbenv/shims"),
  ];
}

function splitPath(value) {
  if (typeof value !== "string" || value.length === 0) {
    return [];
  }
  return value
    .split(path.delimiter)
    .map((entry) => entry.trim())
    .filter(Boolean);
}

function joinPathEntries(entries) {
  const seen = new Set();
  const unique = [];
  for (const entry of entries) {
    if (seen.has(entry)) {
      continue;
    }
    seen.add(entry);
    unique.push(entry);
  }
  return unique.join(path.delimiter);
}

function usableShell(shell) {
  if (typeof shell !== "string" || !shell.startsWith("/")) {
    return null;
  }
  return shell;
}

module.exports = {
  MACOS_COMMON_PATHS,
  buildDesktopEnvironment,
  buildDesktopPath,
  readLoginShellPathSync,
};
