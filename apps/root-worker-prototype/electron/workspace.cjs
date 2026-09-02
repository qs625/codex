const path = require("node:path");
const fsSync = require("node:fs");
const fs = require("node:fs/promises");
const os = require("node:os");

const PACKAGED_SOURCE_RELATIVE_PATH = "source";
const PACKAGED_SOURCE_WORKSPACE_DIR_NAME = "source_workspace";

function resolvePrototypeMorpheusHome(env = process.env) {
  return env.MORPHEUS_HOME ?? path.join(os.homedir(), ".morpheus");
}

function resolveDefaultWorkspace(env = process.env, options = {}) {
  if (env.ROOT_WORKER_WORKSPACE) {
    return env.ROOT_WORKER_WORKSPACE;
  }
  if (findPackagedSourceSnapshotPath(options)) {
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

async function ensureDefaultWorkspace(env = process.env, options = {}) {
  const workspace = resolveDefaultWorkspace(env, options);
  await seedPackagedSourceWorkspaceIfNeeded(workspace, env, options);
  return ensureWorkspaceExists(workspace);
}

function ensureDefaultWorkspaceSync(env = process.env, options = {}) {
  const workspace = resolveDefaultWorkspace(env, options);
  seedPackagedSourceWorkspaceIfNeededSync(workspace, env, options);
  fsSync.mkdirSync(workspace, { recursive: true });
  return workspace;
}

async function seedPackagedSourceWorkspaceIfNeeded(
  workspace,
  env = process.env,
  options = {},
) {
  if (env.ROOT_WORKER_WORKSPACE) {
    return false;
  }
  const sourceSnapshot = findPackagedSourceSnapshotPath(options);
  if (!sourceSnapshot || pathExistsSync(workspace, options)) {
    return false;
  }
  await fs.cp(sourceSnapshot, workspace, { recursive: true });
  return true;
}

function seedPackagedSourceWorkspaceIfNeededSync(
  workspace,
  env = process.env,
  options = {},
) {
  if (env.ROOT_WORKER_WORKSPACE) {
    return false;
  }
  const sourceSnapshot = findPackagedSourceSnapshotPath(options);
  if (!sourceSnapshot || pathExistsSync(workspace, options)) {
    return false;
  }
  fsSync.cpSync(sourceSnapshot, workspace, { recursive: true });
  return true;
}

function findPackagedSourceSnapshotPath(options = {}) {
  const resourcesPath = options.resourcesPath ?? currentResourcesPath();
  if (!resourcesPath) {
    return null;
  }
  const candidate = path.join(resourcesPath, PACKAGED_SOURCE_RELATIVE_PATH);
  const existsSync = options.existsSync ?? fsSync.existsSync;
  const statSync = options.statSync ?? fsSync.statSync;
  if (!existsSync(candidate)) {
    return null;
  }
  try {
    if (!statSync(candidate).isDirectory()) {
      return null;
    }
  } catch {
    return null;
  }
  return candidate;
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
  findPackagedSourceSnapshotPath,
  resolveDefaultWorkspace,
  resolvePrototypeMorpheusHome,
  seedPackagedSourceWorkspaceIfNeeded,
  seedPackagedSourceWorkspaceIfNeededSync,
};
