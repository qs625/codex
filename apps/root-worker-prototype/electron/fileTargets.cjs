const os = require("node:os");
const path = require("node:path");
const { fileURLToPath } = require("node:url");

function isLocalLinkTarget(target) {
  return (
    target.startsWith("file://") ||
    target.startsWith("/") ||
    target.startsWith("~/") ||
    target.startsWith("./") ||
    target.startsWith("../") ||
    target.startsWith("\\\\") ||
    /^[A-Za-z]:[\\/]/.test(target)
  );
}

function localFilePathFromTarget(target, defaultWorkspace) {
  return parseLocalFileTarget(target, defaultWorkspace).path;
}

function parseLocalFileTarget(target, defaultWorkspace) {
  const normalizedTarget = target.trim();
  const targetWithoutHash = normalizedTarget.split("#", 1)[0];

  if (targetWithoutHash.startsWith("file://")) {
    return {
      path: fileURLToPath(new URL(targetWithoutHash)),
      line: null,
      column: null,
    };
  }

  const locationMatch = targetWithoutHash.match(/:(\d+)(?::(\d+))?(?:-\d+(?::\d+)?)?$/);
  const pathWithMaybeLocation = locationMatch
    ? targetWithoutHash.slice(0, Math.max(0, targetWithoutHash.length - locationMatch[0].length))
    : targetWithoutHash;
  const hasFileExtension = Boolean(path.extname(pathWithMaybeLocation));
  const resolvedPath = resolveLocalPath(hasFileExtension ? pathWithMaybeLocation : targetWithoutHash, defaultWorkspace);

  if (!locationMatch || !hasFileExtension) {
    return {
      path: resolvedPath,
      line: null,
      column: null,
    };
  }

  return {
    path: resolvedPath,
    line: Number.parseInt(locationMatch[1], 10),
    column: locationMatch[2] ? Number.parseInt(locationMatch[2], 10) : null,
  };
}

function resolveLocalPath(targetPath, defaultWorkspace) {
  if (targetPath.startsWith("~/")) {
    return path.join(os.homedir(), targetPath.slice(2));
  }

  if (targetPath.startsWith("./") || targetPath.startsWith("../")) {
    return path.resolve(defaultWorkspace, targetPath);
  }

  return targetPath;
}

module.exports = {
  isLocalLinkTarget,
  localFilePathFromTarget,
  parseLocalFileTarget,
};
