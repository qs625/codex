"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

const CAPSULE_MANIFEST_FILE = "capsule.json";
const CAPSULE_SCHEMA_VERSION = 1;
const RELEASE_ID_PREFIX = "sha256:";
const RELEASE_PREIMAGE_DOMAIN = Buffer.from("runtime-capsule-v1\0", "ascii");
const READINESS_PROTOCOL = "launcher-ready-v1";
const PROCESS_SUPERVISION_CONTRACT = "cooperative-observed-v1";
const PROHIBITED_PROCESS_BEHAVIORS = Object.freeze([
  "daemonize",
  "double-fork",
  "setsid",
]);
const MIN_READINESS_TIMEOUT_MS = 1_000;
const MAX_READINESS_TIMEOUT_MS = 120_000;
const MAX_SYMLINK_HOPS = 64;

function createRuntimeCapsule(
  root,
  {
    arch = process.arch,
    arguments: launchArguments = [],
    cwd = null,
    executable,
    metadata = {},
    os = process.platform,
    readinessTimeoutMs = 30_000,
    fsOps = fs,
  } = {},
) {
  const launch = {
    executable: requirePortablePath(executable, "launch executable"),
    arguments: launchArguments.map((argument, index) =>
      requireLaunchArgument(argument, index),
    ),
    readiness: {
      protocol: READINESS_PROTOCOL,
      timeoutMs: requireReadinessTimeout(readinessTimeoutMs),
    },
  };
  if (cwd !== null && cwd !== undefined) {
    launch.cwd = requirePortablePath(cwd, "launch cwd");
  }
  const manifest = {
    schemaVersion: CAPSULE_SCHEMA_VERSION,
    releaseId: `${RELEASE_ID_PREFIX}${"0".repeat(64)}`,
    target: {
      os: requireTargetComponent(os, "target os"),
      arch: requireTargetComponent(arch, "target arch"),
    },
    launch,
    processSupervision: {
      contract: PROCESS_SUPERVISION_CONTRACT,
      prohibitedBehaviors: [...PROHIBITED_PROCESS_BEHAVIORS],
    },
    entries: collectRuntimeCapsuleEntries(root, { fsOps }),
    metadata,
  };
  validateRuntimeCapsuleManifest(manifest);
  manifest.releaseId = computeRuntimeCapsuleReleaseId(manifest);
  fsOps.writeFileSync(
    path.join(root, CAPSULE_MANIFEST_FILE),
    `${JSON.stringify(manifest, null, 2)}\n`,
    { encoding: "utf8", mode: 0o644 },
  );
  return manifest;
}

function collectRuntimeCapsuleEntries(root, { fsOps = fs } = {}) {
  const result = [];
  visitDirectory(root, "", result, fsOps);
  return result.sort(compareAsciiPath);
}

function visitDirectory(root, relativeDirectory, result, fsOps) {
  const directory = relativeDirectory
    ? path.join(root, ...relativeDirectory.split("/"))
    : root;
  const children = fsOps
    .readdirSync(directory, { withFileTypes: true })
    .sort((left, right) => compareAscii(left.name, right.name));
  for (const child of children) {
    const relativePath = relativeDirectory
      ? `${relativeDirectory}/${child.name}`
      : child.name;
    requirePortablePath(relativePath, "capsule entry");
    if (
      relativeDirectory === "" &&
      relativePath === CAPSULE_MANIFEST_FILE
    ) {
      continue;
    }
    const absolutePath = path.join(root, ...relativePath.split("/"));
    const metadata = fsOps.lstatSync(absolutePath);
    if (metadata.isDirectory()) {
      result.push({ type: "directory", path: relativePath });
      visitDirectory(root, relativePath, result, fsOps);
      continue;
    }
    if (metadata.isFile()) {
      if (metadata.nlink !== 1) {
        throw new Error(`Capsule file must not be a hard link: ${relativePath}`);
      }
      const executable = (metadata.mode & 0o111) !== 0;
      result.push({
        type: "file",
        path: relativePath,
        sha256: sha256(fsOps.readFileSync(absolutePath)),
        executable,
      });
      continue;
    }
    if (metadata.isSymbolicLink()) {
      const target = fsOps.readlinkSync(absolutePath);
      requireSymlinkTarget(target, relativePath);
      result.push({ type: "symlink", path: relativePath, target });
      continue;
    }
    throw new Error(`Unsupported Capsule entry type: ${relativePath}`);
  }
}

function validateRuntimeCapsuleManifest(manifest) {
  if (manifest?.schemaVersion !== CAPSULE_SCHEMA_VERSION) {
    throw new Error(
      `Unsupported Capsule schema: ${String(manifest?.schemaVersion)}`,
    );
  }
  requireTargetComponent(manifest.target?.os, "target os");
  requireTargetComponent(manifest.target?.arch, "target arch");
  const executable = requirePortablePath(
    manifest.launch?.executable,
    "launch executable",
  );
  if (manifest.launch?.cwd !== undefined) {
    requirePortablePath(manifest.launch.cwd, "launch cwd");
  }
  if (!Array.isArray(manifest.launch?.arguments)) {
    throw new Error("Capsule launch arguments must be an array");
  }
  manifest.launch.arguments.forEach(requireLaunchArgument);
  if (manifest.launch?.readiness?.protocol !== READINESS_PROTOCOL) {
    throw new Error("Unsupported Capsule readiness protocol");
  }
  requireReadinessTimeout(manifest.launch.readiness.timeoutMs);
  if (
    manifest.processSupervision?.contract !== PROCESS_SUPERVISION_CONTRACT ||
    !Array.isArray(manifest.processSupervision?.prohibitedBehaviors) ||
    manifest.processSupervision.prohibitedBehaviors.length !==
      PROHIBITED_PROCESS_BEHAVIORS.length ||
    manifest.processSupervision.prohibitedBehaviors.some(
      (behavior, index) => behavior !== PROHIBITED_PROCESS_BEHAVIORS[index],
    )
  ) {
    throw new Error(
      "Capsule must declare cooperative observed supervision and prohibit daemonize, double-fork, setsid, and process-group escape",
    );
  }
  if (!Array.isArray(manifest.entries) || manifest.entries.length === 0) {
    throw new Error("Capsule must declare at least one entry");
  }

  const entries = new Map();
  const caseFolded = new Map();
  for (const entry of manifest.entries) {
    const entryPath = requirePortablePath(entry?.path, "capsule entry");
    if (entryPath.split("/").at(-1) === CAPSULE_MANIFEST_FILE) {
      throw new Error(
        `${CAPSULE_MANIFEST_FILE} is reserved for the Capsule root sidecar`,
      );
    }
    if (entries.has(entryPath)) {
      throw new Error(`Duplicate Capsule entry: ${entryPath}`);
    }
    const folded = entryPath.toLowerCase();
    if (caseFolded.has(folded)) {
      throw new Error(
        `Case-folding collision between ${caseFolded.get(folded)} and ${entryPath}`,
      );
    }
    caseFolded.set(folded, entryPath);
    entries.set(entryPath, entry);
    if (entry.type === "file") {
      requireSha256(entry.sha256, entryPath);
      if (typeof entry.executable !== "boolean") {
        throw new Error(`Capsule file executable flag is invalid: ${entryPath}`);
      }
    } else if (entry.type === "symlink") {
      requireSymlinkTarget(entry.target, entryPath);
    } else if (entry.type !== "directory") {
      throw new Error(`Unsupported Capsule entry type: ${String(entry.type)}`);
    }
  }

  for (const [entryPath] of entries) {
    let parent = portableParent(entryPath);
    while (parent) {
      if (entries.get(parent)?.type !== "directory") {
        throw new Error(
          `Capsule entry ${entryPath} has undeclared directory parent ${parent}`,
        );
      }
      parent = portableParent(parent);
    }
  }
  const executableEntry = entries.get(executable);
  if (
    executableEntry?.type !== "file" ||
    executableEntry.executable !== true
  ) {
    throw new Error(
      "Capsule launch executable must name a declared executable file",
    );
  }
  if (
    manifest.launch.cwd !== undefined &&
    entries.get(manifest.launch.cwd)?.type !== "directory"
  ) {
    throw new Error("Capsule launch cwd must name a declared directory");
  }
  validateSymlinkGraph(entries);
  return manifest;
}

function validateSymlinkGraph(entries) {
  for (const [entryPath, entry] of entries) {
    if (entry.type !== "symlink") {
      continue;
    }
    resolveDeclaredPath(
      resolvePortableSymlinkTarget(entryPath, entry.target),
      entries,
      entryPath,
    );
  }
}

function resolveDeclaredPath(initialPath, entries, sourceSymlink) {
  let remaining = initialPath.split("/");
  let resolved = [];
  const visitedSymlinks = new Set([sourceSymlink]);
  let hops = 1;
  while (remaining.length > 0) {
    const component = remaining.shift();
    const candidate = [...resolved, component].join("/");
    const entry = entries.get(candidate);
    if (!entry) {
      throw new Error(
        `Capsule symlink ${sourceSymlink} resolves to undeclared entry ${candidate}`,
      );
    }
    if (entry.type === "directory") {
      resolved.push(component);
      continue;
    }
    if (entry.type === "file") {
      if (remaining.length > 0) {
        throw new Error(
          `Capsule symlink ${sourceSymlink} traverses through file ${candidate}`,
        );
      }
      return candidate;
    }
    if (visitedSymlinks.has(candidate)) {
      throw new Error(`Capsule symlink cycle includes ${candidate}`);
    }
    visitedSymlinks.add(candidate);
    hops += 1;
    if (hops > MAX_SYMLINK_HOPS) {
      throw new Error(`Capsule symlink hop limit exceeded at ${candidate}`);
    }
    const expanded = resolvePortableSymlinkTarget(candidate, entry.target);
    remaining = [...expanded.split("/"), ...remaining];
    resolved = [];
  }
  const finalPath = resolved.join("/");
  if (entries.get(finalPath)?.type !== "directory") {
    throw new Error(
      `Capsule symlink ${sourceSymlink} did not resolve to a declared entry`,
    );
  }
  return finalPath;
}

function computeRuntimeCapsuleReleaseId(manifest) {
  return `${RELEASE_ID_PREFIX}${sha256(computeRuntimeCapsulePreimage(manifest))}`;
}

function computeRuntimeCapsulePreimage(manifest) {
  validateRuntimeCapsuleManifest(manifest);
  const chunks = [RELEASE_PREIMAGE_DOMAIN];
  pushFrame(chunks, "schema", encodeU32(manifest.schemaVersion));
  pushFrame(chunks, "target.os", Buffer.from(manifest.target.os, "ascii"));
  pushFrame(chunks, "target.arch", Buffer.from(manifest.target.arch, "ascii"));
  pushFrame(
    chunks,
    "launch.executable",
    Buffer.from(manifest.launch.executable, "ascii"),
  );
  if (manifest.launch.cwd !== undefined) {
    pushFrame(chunks, "launch.cwd.present", Buffer.from([1]));
    pushFrame(chunks, "launch.cwd", Buffer.from(manifest.launch.cwd, "ascii"));
  } else {
    pushFrame(chunks, "launch.cwd.present", Buffer.from([0]));
  }
  pushFrame(
    chunks,
    "launch.arguments.count",
    encodeU64(manifest.launch.arguments.length),
  );
  manifest.launch.arguments.forEach((argument, index) => {
    pushFrame(
      chunks,
      `launch.argument.${index}`,
      Buffer.from(argument, "utf8"),
    );
  });
  pushFrame(
    chunks,
    "readiness.protocol",
    Buffer.from(manifest.launch.readiness.protocol, "ascii"),
  );
  pushFrame(
    chunks,
    "readiness.timeout_ms",
    encodeU64(manifest.launch.readiness.timeoutMs),
  );
  pushFrame(
    chunks,
    "process_supervision.contract",
    Buffer.from(manifest.processSupervision.contract, "ascii"),
  );
  pushFrame(
    chunks,
    "process_supervision.prohibited.count",
    encodeU64(manifest.processSupervision.prohibitedBehaviors.length),
  );
  for (const behavior of manifest.processSupervision.prohibitedBehaviors) {
    pushFrame(
      chunks,
      "process_supervision.prohibited",
      Buffer.from(behavior, "ascii"),
    );
  }
  const entries = [...manifest.entries].sort(compareAsciiPath);
  pushFrame(chunks, "entries.count", encodeU64(entries.length));
  entries.forEach((entry, index) => {
    pushFrame(chunks, `entry.${index}.path`, Buffer.from(entry.path, "ascii"));
    pushFrame(chunks, `entry.${index}.type`, Buffer.from(entry.type, "ascii"));
    if (entry.type === "file") {
      pushFrame(
        chunks,
        `entry.${index}.sha256`,
        Buffer.from(entry.sha256, "hex"),
      );
      pushFrame(
        chunks,
        `entry.${index}.executable`,
        Buffer.from([entry.executable ? 1 : 0]),
      );
    } else if (entry.type === "symlink") {
      pushFrame(
        chunks,
        `entry.${index}.target`,
        Buffer.from(entry.target, "ascii"),
      );
    }
  });
  return Buffer.concat(chunks);
}

function pushFrame(chunks, tag, value) {
  const tagBytes = Buffer.from(tag, "ascii");
  if (tagBytes.length > 0xffff) {
    throw new Error(`Capsule frame tag is too long: ${tag}`);
  }
  const header = Buffer.alloc(2 + tagBytes.length + 8);
  header.writeUInt16BE(tagBytes.length, 0);
  tagBytes.copy(header, 2);
  header.writeBigUInt64BE(BigInt(value.length), 2 + tagBytes.length);
  chunks.push(header, value);
}

function encodeU32(value) {
  const buffer = Buffer.alloc(4);
  buffer.writeUInt32BE(value);
  return buffer;
}

function encodeU64(value) {
  const buffer = Buffer.alloc(8);
  buffer.writeBigUInt64BE(BigInt(value));
  return buffer;
}

function requirePortablePath(value, label) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length > 0xffff ||
    !isAscii(value) ||
    value.startsWith("/") ||
    value.endsWith("/") ||
    value.includes("\\") ||
    value.includes("\0")
  ) {
    throw new Error(`${label} is not portable ASCII: ${String(value)}`);
  }
  for (const component of value.split("/")) {
    requirePortableComponent(component, value);
  }
  return value;
}

function requirePortableComponent(component, fullPath) {
  if (
    component.length === 0 ||
    component === "." ||
    component === ".." ||
    component.endsWith(" ") ||
    component.endsWith(".") ||
    [...component].some((character) => {
      const code = character.charCodeAt(0);
      return code < 0x20 || code > 0x7e || '<>:"|?*'.includes(character);
    }) ||
    isWindowsReservedComponent(component)
  ) {
    throw new Error(
      `Capsule path has non-portable component ${JSON.stringify(component)}: ${fullPath}`,
    );
  }
}

function requireSymlinkTarget(value, entryPath) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length > 0xffff ||
    !isAscii(value) ||
    value.startsWith("/") ||
    value.endsWith("/") ||
    value.includes("\\") ||
    value.includes("\0")
  ) {
    throw new Error(
      `Symlink target for ${entryPath} is not portable relative ASCII`,
    );
  }
  for (const component of value.split("/")) {
    if (component === "." || component === "..") {
      continue;
    }
    if (
      component.length === 0 ||
      component.endsWith(" ") ||
      component.endsWith(".") ||
      [...component].some((character) => {
        const code = character.charCodeAt(0);
        return code < 0x20 || code > 0x7e || '<>:"|?*'.includes(character);
      })
    ) {
      throw new Error(`Symlink target for ${entryPath} is not portable`);
    }
  }
  return value;
}

function resolvePortableSymlinkTarget(entryPath, target) {
  const components = portableParent(entryPath)?.split("/") ?? [];
  for (const component of target.split("/")) {
    if (component === ".") {
      continue;
    }
    if (component === "..") {
      if (components.length === 0) {
        throw new Error(`Capsule symlink ${entryPath} escapes the Capsule root`);
      }
      components.pop();
      continue;
    }
    components.push(component);
  }
  if (components.length === 0) {
    throw new Error(`Capsule symlink ${entryPath} resolves to the Capsule root`);
  }
  return components.join("/");
}

function requireLaunchArgument(value, index) {
  if (typeof value !== "string" || value.includes("\0")) {
    throw new Error(`Capsule launch argument ${index} contains NUL`);
  }
  return value;
}

function requireReadinessTimeout(value) {
  if (
    !Number.isSafeInteger(value) ||
    value < MIN_READINESS_TIMEOUT_MS ||
    value > MAX_READINESS_TIMEOUT_MS
  ) {
    throw new Error(
      `Capsule readiness timeout must be between ${MIN_READINESS_TIMEOUT_MS} and ${MAX_READINESS_TIMEOUT_MS} milliseconds`,
    );
  }
  return value;
}

function requireTargetComponent(value, label) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length > 64 ||
    !/^[A-Za-z0-9_.-]+$/.test(value)
  ) {
    throw new Error(`Invalid Capsule ${label}: ${String(value)}`);
  }
  return value;
}

function requireSha256(value, entryPath) {
  if (typeof value !== "string" || !/^[0-9a-f]{64}$/.test(value)) {
    throw new Error(`Invalid sha256 for Capsule entry ${entryPath}`);
  }
  return value;
}

function portableParent(value) {
  const index = value.lastIndexOf("/");
  return index === -1 ? null : value.slice(0, index);
}

function compareAsciiPath(left, right) {
  return compareAscii(left.path, right.path);
}

function compareAscii(left, right) {
  return Buffer.compare(Buffer.from(left, "ascii"), Buffer.from(right, "ascii"));
}

function isAscii(value) {
  return /^[\x00-\x7f]*$/.test(value);
}

function isWindowsReservedComponent(component) {
  const stem = component.split(".", 1)[0].toLowerCase();
  return /^(con|prn|aux|nul|com[1-9]|lpt[1-9])$/.test(stem);
}

function sha256(value) {
  return crypto.createHash("sha256").update(value).digest("hex");
}

module.exports = {
  CAPSULE_MANIFEST_FILE,
  CAPSULE_SCHEMA_VERSION,
  PROCESS_SUPERVISION_CONTRACT,
  PROHIBITED_PROCESS_BEHAVIORS,
  READINESS_PROTOCOL,
  collectRuntimeCapsuleEntries,
  computeRuntimeCapsulePreimage,
  computeRuntimeCapsuleReleaseId,
  createRuntimeCapsule,
  resolveDeclaredPath,
  resolvePortableSymlinkTarget,
  validateRuntimeCapsuleManifest,
};
