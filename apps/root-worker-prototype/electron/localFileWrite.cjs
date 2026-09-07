const fs = require("node:fs/promises");

const { isLocalLinkTarget, parseLocalFileTarget } = require("./fileTargets.cjs");

async function writeLocalFileTarget(target, content, defaultWorkspace) {
  if (typeof target !== "string" || !target.trim()) {
    throw new Error("Cannot write empty link target");
  }
  if (!isLocalLinkTarget(target.trim())) {
    throw new Error("Only local file links can be written");
  }
  if (typeof content !== "string") {
    throw new Error("File content must be a string");
  }

  const { path: filePath } = parseLocalFileTarget(
    target.trim(),
    defaultWorkspace,
  );
  const beforeStat = await fs.stat(filePath);
  if (!beforeStat.isFile()) {
    throw new Error("Only files can be written");
  }

  await fs.writeFile(filePath, content, "utf8");
  const afterStat = await fs.stat(filePath);
  if (!afterStat.isFile()) {
    throw new Error("Only files can be written");
  }

  return {
    ok: true,
    path: filePath,
    byteSize: afterStat.size,
  };
}

module.exports = {
  writeLocalFileTarget,
};
