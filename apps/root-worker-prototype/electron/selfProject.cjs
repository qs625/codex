const fs = require("node:fs");
const path = require("node:path");
const { resolvePrototypeMorpheusHome } = require("./workspace.cjs");

const SELF_PROJECT_ID = "/self";
const SELF_PROJECT_FILE_NAME = "self-project.json";

function selfProjectPath(env = process.env) {
  return path.join(resolvePrototypeMorpheusHome(env), SELF_PROJECT_FILE_NAME);
}

function ensureSelfProjectSync(env = process.env, workspace, options = {}) {
  if (typeof workspace !== "string" || !workspace.trim()) {
    throw new Error("self project requires a source workspace path");
  }
  const projectPath = selfProjectPath(env);
  const readFileSync = options.readFileSync ?? fs.readFileSync;
  const writeFileSync = options.writeFileSync ?? fs.writeFileSync;
  const mkdirSync = options.mkdirSync ?? fs.mkdirSync;
  const existsSync = options.existsSync ?? fs.existsSync;
  const existing = readExistingSelfProject(projectPath, {
    existsSync,
    readFileSync,
  });
  const project = {
    ...existing,
    id: SELF_PROJECT_ID,
    path: SELF_PROJECT_ID,
    workspace: workspace.trim(),
    hidden: true,
    system: true,
    managedBy: "morpheus",
  };

  mkdirSync(path.dirname(projectPath), { recursive: true });
  writeFileSync(projectPath, `${JSON.stringify(project, null, 2)}\n`);
  return project;
}

function readExistingSelfProject(projectPath, options = {}) {
  const existsSync = options.existsSync ?? fs.existsSync;
  if (!existsSync(projectPath)) {
    return {};
  }
  const readFileSync = options.readFileSync ?? fs.readFileSync;
  try {
    const value = JSON.parse(readFileSync(projectPath, "utf8"));
    return value && typeof value === "object" && !Array.isArray(value)
      ? value
      : {};
  } catch {
    return {};
  }
}

function removeSelfProjectIfManagedSync(env = process.env, options = {}) {
  const projectPath = selfProjectPath(env);
  const project = readExistingSelfProject(projectPath, options);
  if (
    project.id !== SELF_PROJECT_ID ||
    project.path !== SELF_PROJECT_ID ||
    project.managedBy !== "morpheus"
  ) {
    return false;
  }
  const unlinkSync = options.unlinkSync ?? fs.unlinkSync;
  unlinkSync(projectPath);
  return true;
}

module.exports = {
  SELF_PROJECT_FILE_NAME,
  SELF_PROJECT_ID,
  ensureSelfProjectSync,
  removeSelfProjectIfManagedSync,
  selfProjectPath,
};
