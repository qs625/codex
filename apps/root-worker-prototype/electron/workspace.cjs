const path = require("node:path");
const fsSync = require("node:fs");
const fs = require("node:fs/promises");
const os = require("node:os");

const PACKAGED_SOURCE_WORKSPACE_DIR_NAME = "source_workspace";
const MORPHEUS_HOME_INSTRUCTIONS_DIR_NAME = "instructions";
const MORPHEUS_SOURCE_INSTRUCTION_FILE_NAME = "morpheus-source-workspace.md";
const MORPHEUS_SOURCE_INSTRUCTION_MARKER =
  "<!-- managed-by-morpheus-source-workspace -->";

function resolvePrototypeMorpheusHome(env = process.env) {
  return env.MORPHEUS_HOME ?? path.join(os.homedir(), ".morpheus");
}

function resolveDefaultWorkspace(env = process.env, options = {}) {
  if (env.ROOT_WORKER_WORKSPACE) {
    if (
      !isPackagedApp(options) ||
      isSourceWorkspaceSync(env.ROOT_WORKER_WORKSPACE, options)
    ) {
      return env.ROOT_WORKER_WORKSPACE;
    }
    if (options.sourceOnly) {
      return null;
    }
    return resolveOperationalWorkspace(env);
  }
  if (isPackagedApp(options)) {
    const sourceWorkspace = path.join(
      resolvePrototypeMorpheusHome(env),
      PACKAGED_SOURCE_WORKSPACE_DIR_NAME,
    );
    if (isSourceWorkspaceSync(sourceWorkspace, options)) {
      return sourceWorkspace;
    }
    if (options.sourceOnly) {
      return null;
    }
  }
  return resolveOperationalWorkspace(env);
}

function resolveOperationalWorkspace(env = process.env) {
  return path.join(resolvePrototypeMorpheusHome(env), "root_workspace");
}

function isSourceWorkspaceSync(workspace, options = {}) {
  if (typeof workspace !== "string" || !workspace.trim()) {
    return false;
  }
  const existsSync = options.existsSync ?? fsSync.existsSync;
  return (
    existsSync(path.join(workspace, "apps", "root-worker-prototype", "package.json")) &&
    existsSync(path.join(workspace, "codex-rs", "Cargo.toml"))
  );
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
  if (!workspace) {
    return null;
  }
  await ensureWorkspaceExists(workspace);
  if (
    !env.ROOT_WORKER_WORKSPACE &&
    isPackagedApp(options) &&
    path.basename(workspace) === PACKAGED_SOURCE_WORKSPACE_DIR_NAME
  ) {
    ensureMorpheusSourceInstructionSync(env, workspace, options);
  }
  return workspace;
}

function ensureDefaultWorkspaceSync(env = process.env, options = {}) {
  const workspace = resolveDefaultWorkspace(env, options);
  if (!workspace) {
    return null;
  }
  fsSync.mkdirSync(workspace, { recursive: true });
  if (
    !env.ROOT_WORKER_WORKSPACE &&
    isPackagedApp(options) &&
    path.basename(workspace) === PACKAGED_SOURCE_WORKSPACE_DIR_NAME
  ) {
    ensureMorpheusSourceInstructionSync(env, workspace, options);
  }
  return workspace;
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

function removeMorpheusSourceInstructionIfManagedSync(
  env = process.env,
  options = {},
) {
  const instructionPath = morpheusSourceInstructionPath(env);
  const existsSync = options.existsSync ?? fsSync.existsSync;
  if (!existsSync(instructionPath)) {
    return false;
  }
  const readFileSync = options.readFileSync ?? fsSync.readFileSync;
  if (
    !readFileSync(instructionPath, "utf8").includes(
      MORPHEUS_SOURCE_INSTRUCTION_MARKER,
    )
  ) {
    return false;
  }
  const unlinkSync = options.unlinkSync ?? fsSync.unlinkSync;
  unlinkSync(instructionPath);
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

When modifying Morpheus runtime, client, server, frontend, or backend code in this workspace, complete the relevant tests first. After those tests pass, call \`request_runtime_restart\`. Installed desktop builds produce and activate one complete versioned Runtime Capsule from this workspace without modifying the signed application bundle. If this workspace is absent, the installed Seed or current external Runtime Capsule still runs, but producing a new candidate is unsupported.
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
  ensureMorpheusSourceInstructionSync,
  isPackagedApp,
  MORPHEUS_HOME_INSTRUCTIONS_DIR_NAME,
  MORPHEUS_SOURCE_INSTRUCTION_FILE_NAME,
  morpheusSourceInstructionContent,
  morpheusSourceInstructionPath,
  removeMorpheusSourceInstructionIfManagedSync,
  isSourceWorkspaceSync,
  resolveDefaultWorkspace,
  resolveOperationalWorkspace,
  resolvePrototypeMorpheusHome,
};
