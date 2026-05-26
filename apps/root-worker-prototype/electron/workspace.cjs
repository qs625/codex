const path = require("node:path");
const fs = require("node:fs/promises");
const os = require("node:os");

function resolvePrototypeCodexHome(env = process.env) {
  return env.CODEX_HOME ?? path.join(os.homedir(), ".codex-home");
}

function resolveDefaultWorkspace(env = process.env) {
  return env.ROOT_WORKER_WORKSPACE ?? path.join(resolvePrototypeCodexHome(env), "root_workspace");
}

async function ensureWorkspaceExists(workspacePath) {
  await fs.mkdir(workspacePath, { recursive: true });
  return workspacePath;
}

async function ensureDefaultWorkspace(env = process.env) {
  return ensureWorkspaceExists(resolveDefaultWorkspace(env));
}

module.exports = {
  resolvePrototypeCodexHome,
  resolveDefaultWorkspace,
  ensureWorkspaceExists,
  ensureDefaultWorkspace,
};
