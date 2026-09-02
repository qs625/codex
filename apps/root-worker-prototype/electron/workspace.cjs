const path = require("node:path");
const fsSync = require("node:fs");
const fs = require("node:fs/promises");
const os = require("node:os");
const { spawnSync } = require("node:child_process");

const PACKAGED_SOURCE_WORKSPACE_DIR_NAME = "source_workspace";
const INSTALLED_SOURCE_ORIGIN_URL = "git@github.com:qs625/codex.git";
const MORPHEUS_HOME_INSTRUCTIONS_DIR_NAME = "instructions";
const MORPHEUS_SOURCE_INSTRUCTION_FILE_NAME = "morpheus-source-workspace.md";
const MORPHEUS_SOURCE_INSTRUCTION_MARKER =
  "<!-- managed-by-morpheus-source-workspace -->";

function resolvePrototypeMorpheusHome(env = process.env) {
  return env.MORPHEUS_HOME ?? path.join(os.homedir(), ".morpheus");
}

function resolveDefaultWorkspace(env = process.env, options = {}) {
  if (env.ROOT_WORKER_WORKSPACE) {
    return env.ROOT_WORKER_WORKSPACE;
  }
  if (isPackagedApp(options)) {
    return path.join(
      resolvePrototypeMorpheusHome(env),
      PACKAGED_SOURCE_WORKSPACE_DIR_NAME,
    );
  }
  return path.join(resolvePrototypeMorpheusHome(env), "root_workspace");
}

async function ensureWorkspaceExists(workspacePath) {
  await fs.mkdir(workspacePath, { recursive: true });
  return workspacePath;
}

function ensureWorkspaceExistsSync(workspacePath) {
  fsSync.mkdirSync(workspacePath, { recursive: true });
  return workspacePath;
}

async function ensureDefaultWorkspace(env = process.env, options = {}) {
  const workspace = resolveDefaultWorkspace(env, options);
  await cloneInstalledSourceWorkspaceIfNeeded(workspace, env, options);
  await ensureWorkspaceExists(workspace);
  if (!env.ROOT_WORKER_WORKSPACE && isPackagedApp(options)) {
    ensureMorpheusSourceInstructionSync(env, workspace, options);
  }
  return workspace;
}

function ensureDefaultWorkspaceSync(env = process.env, options = {}) {
  const workspace = resolveDefaultWorkspace(env, options);
  cloneInstalledSourceWorkspaceIfNeededSync(workspace, env, options);
  fsSync.mkdirSync(workspace, { recursive: true });
  if (!env.ROOT_WORKER_WORKSPACE && isPackagedApp(options)) {
    ensureMorpheusSourceInstructionSync(env, workspace, options);
  }
  return workspace;
}

async function cloneInstalledSourceWorkspaceIfNeeded(
  workspace,
  env = process.env,
  options = {},
) {
  return cloneInstalledSourceWorkspaceIfNeededSync(workspace, env, options);
}

function cloneInstalledSourceWorkspaceIfNeededSync(
  workspace,
  env = process.env,
  options = {},
) {
  if (env.ROOT_WORKER_WORKSPACE) {
    return false;
  }
  if (!isPackagedApp(options) || pathExistsSync(workspace, options)) {
    return false;
  }

  const mkdirSync = options.mkdirSync ?? fsSync.mkdirSync;
  const parentDir = path.dirname(workspace);
  const tempWorkspace = cloneTempWorkspacePath(workspace, options);
  mkdirSync(parentDir, { recursive: true });
  cleanupPath(tempWorkspace, options);
  const spawn = options.spawnSync ?? spawnSync;
  const result = spawn(
    "rtk",
    ["git", "clone", INSTALLED_SOURCE_ORIGIN_URL, tempWorkspace],
    {
      cwd: parentDir,
      encoding: "utf8",
      stdio: options.stdio ?? "pipe",
    },
  );
  if (result.error) {
    cleanupPath(tempWorkspace, options);
    throw result.error;
  }
  if (result.status !== 0) {
    cleanupPath(tempWorkspace, options);
    const stderr = result.stderr ? String(result.stderr).trim() : "";
    throw new Error(
      `rtk git clone ${INSTALLED_SOURCE_ORIGIN_URL} ${workspace} exited with ${result.status}${stderr ? `: ${stderr}` : ""}`,
    );
  }
  if (pathExistsSync(workspace, options)) {
    cleanupPath(tempWorkspace, options);
    return false;
  }
  if (!claimWorkspacePath(workspace, options)) {
    cleanupPath(tempWorkspace, options);
    return false;
  }
  try {
    moveDirectoryContentsSync(tempWorkspace, workspace, options);
    cleanupPath(tempWorkspace, options);
  } catch (error) {
    cleanupPath(tempWorkspace, options);
    cleanupPath(workspace, options);
    throw error;
  }
  return true;
}

function claimWorkspacePath(workspace, options = {}) {
  const mkdirSync = options.mkdirSync ?? fsSync.mkdirSync;
  try {
    mkdirSync(workspace);
    return true;
  } catch (error) {
    if (error && error.code === "EEXIST") {
      return false;
    }
    throw error;
  }
}

function moveDirectoryContentsSync(sourceDir, targetDir, options = {}) {
  const readdirSync = options.readdirSync ?? fsSync.readdirSync;
  const renameSync = options.renameSync ?? fsSync.renameSync;
  for (const name of readdirSync(sourceDir)) {
    renameSync(path.join(sourceDir, name), path.join(targetDir, name));
  }
}

function cloneTempWorkspacePath(workspace, options = {}) {
  const suffix =
    options.cloneTempSuffix ??
    `${process.pid}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  return path.join(path.dirname(workspace), `.${path.basename(workspace)}.clone-${suffix}`);
}

function cleanupPath(targetPath, options = {}) {
  if (!pathExistsSync(targetPath, options)) {
    return;
  }
  const rmSync = options.rmSync ?? fsSync.rmSync;
  rmSync(targetPath, { recursive: true, force: true });
}

function ensureMorpheusSourceInstructionSync(
  env = process.env,
  workspace,
  options = {},
) {
  const instructionPath = morpheusSourceInstructionPath(env);
  const existsSync = options.existsSync ?? fsSync.existsSync;
  const readFileSync = options.readFileSync ?? fsSync.readFileSync;
  const writeFileSync = options.writeFileSync ?? fsSync.writeFileSync;
  const mkdirSync = options.mkdirSync ?? fsSync.mkdirSync;

  if (existsSync(instructionPath)) {
    const current = readFileSync(instructionPath, "utf8");
    if (!current.includes(MORPHEUS_SOURCE_INSTRUCTION_MARKER)) {
      return false;
    }
  }

  mkdirSync(path.dirname(instructionPath), { recursive: true });
  writeFileSync(instructionPath, morpheusSourceInstructionContent(workspace));
  return true;
}

function morpheusSourceInstructionPath(env = process.env) {
  return path.join(
    resolvePrototypeMorpheusHome(env),
    MORPHEUS_HOME_INSTRUCTIONS_DIR_NAME,
    MORPHEUS_SOURCE_INSTRUCTION_FILE_NAME,
  );
}

function morpheusSourceInstructionContent(workspace) {
  return `${MORPHEUS_SOURCE_INSTRUCTION_MARKER}
# Morpheus Source Workspace

The Morpheus source workspace for this app is:
\`${workspace}\`

When modifying Morpheus runtime, client, server, frontend, or backend code in this workspace, complete the relevant tests first. After those tests pass, call \`request_runtime_restart\`; installed desktop builds will rebuild this source workspace, update the runnable app artifacts, and relaunch so the running app can load the latest compiled code.
`;
}

function isPackagedApp(options = {}) {
  if (typeof options.isPackagedApp === "boolean") {
    return options.isPackagedApp;
  }
  if (typeof process.defaultApp === "boolean") {
    return !process.defaultApp;
  }
  const resourcesPath = options.resourcesPath ?? currentResourcesPath();
  return Boolean(
    resourcesPath &&
      path.basename(path.dirname(resourcesPath)) === "Contents" &&
      resourcesPath.endsWith(path.join(".app", "Contents", "Resources")),
  );
}

function pathExistsSync(targetPath, options = {}) {
  const existsSync = options.existsSync ?? fsSync.existsSync;
  return existsSync(targetPath);
}

function currentResourcesPath() {
  return typeof process.resourcesPath === "string"
    ? process.resourcesPath
    : null;
}

module.exports = {
  ensureDefaultWorkspace,
  ensureDefaultWorkspaceSync,
  ensureWorkspaceExists,
  ensureWorkspaceExistsSync,
  cloneInstalledSourceWorkspaceIfNeeded,
  cloneInstalledSourceWorkspaceIfNeededSync,
  cloneTempWorkspacePath,
  ensureMorpheusSourceInstructionSync,
  INSTALLED_SOURCE_ORIGIN_URL,
  isPackagedApp,
  MORPHEUS_HOME_INSTRUCTIONS_DIR_NAME,
  MORPHEUS_SOURCE_INSTRUCTION_FILE_NAME,
  morpheusSourceInstructionContent,
  morpheusSourceInstructionPath,
  resolveDefaultWorkspace,
  resolvePrototypeMorpheusHome,
};
