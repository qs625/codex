const path = require("node:path");
const fs = require("node:fs/promises");
const os = require("node:os");

function resolvePrototypeMorpheusHome(env = process.env) {
  return env.MORPHEUS_HOME ?? path.join(os.homedir(), ".morpheus");
}

function resolveDefaultWorkspace(env = process.env) {
  return env.ROOT_WORKER_WORKSPACE ?? path.join(resolvePrototypeMorpheusHome(env), "root_workspace");
}

async function ensureWorkspaceExists(workspacePath) {
  await fs.mkdir(workspacePath, { recursive: true });
  return workspacePath;
}

async function ensureDefaultWorkspace(env = process.env) {
  return ensureWorkspaceExists(resolveDefaultWorkspace(env));
}

module.exports = {
  resolvePrototypeMorpheusHome,
  resolveDefaultWorkspace,
  ensureWorkspaceExists,
  ensureDefaultWorkspace,
};
