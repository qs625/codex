const path = require("node:path");
const fs = require("node:fs/promises");

async function resolveWorkspaceRoot(adapter, filePath) {
  const workspaceRoot = await adapter.resolveWorkspaceRoot(filePath, findClosestMarker, {
    findCargoWorkspaceRoot,
  });
  if (!workspaceRoot) {
    return {
      workspaceRoot: null,
      reason: "No supported workspace marker was found for this file.",
    };
  }

  return {
    workspaceRoot,
    reason: null,
  };
}

async function findClosestMarker(filePath, markerNames) {
  let currentDir = path.dirname(filePath);
  let previousDir = "";

  while (currentDir !== previousDir) {
    for (const markerName of markerNames) {
      const markerPath = path.join(currentDir, markerName);
      if (await pathExists(markerPath)) {
        return currentDir;
      }
    }

    previousDir = currentDir;
    currentDir = path.dirname(currentDir);
  }

  return null;
}

async function findCargoWorkspaceRoot(crateRoot) {
  let currentDir = crateRoot;
  let previousDir = "";

  while (currentDir !== previousDir) {
    const manifestPath = path.join(currentDir, "Cargo.toml");
    if (await isCargoWorkspaceManifest(manifestPath)) {
      return currentDir;
    }

    previousDir = currentDir;
    currentDir = path.dirname(currentDir);
  }

  return null;
}

async function isCargoWorkspaceManifest(manifestPath) {
  try {
    const manifest = await fs.readFile(manifestPath, "utf8");
    return /^\s*\[workspace\]\s*$/m.test(manifest);
  } catch {
    return false;
  }
}

async function pathExists(candidatePath) {
  try {
    await fs.access(candidatePath);
    return true;
  } catch {
    return false;
  }
}

module.exports = {
  findCargoWorkspaceRoot,
  resolveWorkspaceRoot,
};
