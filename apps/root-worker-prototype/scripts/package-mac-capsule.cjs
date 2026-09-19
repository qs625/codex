"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const {
  buildMacAppPackagePlan,
  packageMacRuntimeCapsule,
} = require("./package-mac-app.cjs");
const {
  resolveRuntimeLauncherStateRoot,
} = require("../electron/installedArtifactUpdate.cjs");

function packageMacCapsule({
  activationId,
  cwd = process.cwd(),
  env = process.env,
  fsOps = fs,
  now = () => Date.now(),
  packageRuntimeCapsule = packageMacRuntimeCapsule,
  platform = process.platform,
  stateRoot = resolveRuntimeLauncherStateRoot(env),
} = {}) {
  if (platform !== "darwin") {
    throw new Error("macOS Runtime Capsule packaging requires codesign and must run on macOS.");
  }
  const plan = buildMacAppPackagePlan({ cwd });
  for (const target of [
    plan.payloadStagingDir,
    plan.seedStagingDir,
    plan.resourceStagingDir,
  ]) {
    fsOps.rmSync(target, { force: true, recursive: true });
  }
  const normalizedActivationId = normalizeActivationId(
    activationId ?? env.MORPHEUS_RUNTIME_CAPSULE_ACTIVATION_ID,
  );
  let keepSeedStaging = false;
  try {
    const { manifest, sourceCommit } = packageRuntimeCapsule(plan);
    const resolvedActivationId =
      normalizedActivationId ?? defaultActivationId(manifest.releaseId, now());
    const incomingRoot = path.join(stateRoot, "incoming", resolvedActivationId);
    fsOps.mkdirSync(path.dirname(incomingRoot), { recursive: true, mode: 0o755 });
    fsOps.mkdirSync(incomingRoot, { recursive: false, mode: 0o755 });
    try {
      fsOps.cpSync(plan.seedStagingDir, incomingRoot, {
        recursive: true,
        dereference: false,
        verbatimSymlinks: true,
      });
    } catch (error) {
      try {
        fsOps.rmSync(incomingRoot, { force: true, recursive: true });
      } catch {}
      throw error;
    }
    const result = {
      activationId: resolvedActivationId,
      incomingRoot,
      releaseId: manifest.releaseId,
      sourceCommit,
      stateRoot,
    };
    keepSeedStaging = true;
    printCapsuleResult(result);
    return { ...result, manifest, plan };
  } finally {
    fsOps.rmSync(plan.payloadStagingDir, { force: true, recursive: true });
    fsOps.rmSync(plan.resourceStagingDir, { force: true, recursive: true });
    if (!keepSeedStaging) {
      fsOps.rmSync(plan.seedStagingDir, { force: true, recursive: true });
    }
  }
}

function defaultActivationId(releaseId, timestamp) {
  const digest = crypto
    .createHash("sha256")
    .update(`${releaseId}\0${String(timestamp)}`)
    .digest("hex")
    .slice(0, 16);
  return `capsule-${timestamp}-${digest}`;
}

function normalizeActivationId(value) {
  if (value === undefined || value === null || value === "") {
    return null;
  }
  if (typeof value !== "string") {
    throw new Error(`Invalid Runtime Capsule activation id: ${String(value)}`);
  }
  const normalized = value.trim();
  if (
    normalized.length === 0 ||
    normalized.length > 96 ||
    normalized === "." ||
    normalized === ".." ||
    !/^[A-Za-z0-9_.-]+$/.test(normalized)
  ) {
    throw new Error(`Invalid Runtime Capsule activation id: ${String(value)}`);
  }
  return normalized;
}

function printCapsuleResult(result) {
  console.log("Runtime Capsule ready for launcher selection:");
  console.log(`  activationId: ${result.activationId}`);
  console.log(`  releaseId: ${result.releaseId}`);
  console.log(`  sourceCommit: ${result.sourceCommit}`);
  console.log(`  stateRoot: ${result.stateRoot}`);
  console.log(`  incomingRoot: ${result.incomingRoot}`);
}

if (require.main === module) {
  packageMacCapsule();
}

module.exports = {
  defaultActivationId,
  normalizeActivationId,
  packageMacCapsule,
};
