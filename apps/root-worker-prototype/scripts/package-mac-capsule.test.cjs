"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const {
  defaultActivationId,
  normalizeActivationId,
  packageMacCapsule,
} = require("./package-mac-capsule.cjs");

test("mac capsule-only package stages a complete Capsule under launcher incoming", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "mac-capsule-only-"));
  try {
    const cwd = path.join(root, "apps", "root-worker-prototype");
    const stateRoot = path.join(root, "runtime-launcher");
    fs.mkdirSync(cwd, { recursive: true });
    const releaseId = `sha256:${"b".repeat(64)}`;
    const result = packageMacCapsule({
      activationId: "activation-test",
      cwd,
      fsOps: fs,
      packageRuntimeCapsule(plan) {
        fs.mkdirSync(path.join(plan.seedStagingDir, "payload", "Runtime.app"), {
          recursive: true,
        });
        fs.writeFileSync(
          path.join(plan.seedStagingDir, "payload", "Runtime.app", "runtime"),
          "#!/bin/sh\n",
          { mode: 0o755 },
        );
        fs.writeFileSync(
          path.join(plan.seedStagingDir, "capsule.json"),
          `${JSON.stringify({ releaseId })}\n`,
        );
        return {
          manifest: { releaseId },
          sourceCommit: "deadbeef",
        };
      },
      platform: "darwin",
      stateRoot,
    });

    assert.equal(result.activationId, "activation-test");
    assert.equal(result.releaseId, releaseId);
    assert.equal(result.sourceCommit, "deadbeef");
    assert.equal(
      result.incomingRoot,
      path.join(stateRoot, "incoming", "activation-test"),
    );
    assert.equal(
      fs.readFileSync(path.join(result.incomingRoot, "capsule.json"), "utf8"),
      `${JSON.stringify({ releaseId })}\n`,
    );
    assert.equal(
      fs
        .statSync(path.join(result.incomingRoot, "payload", "Runtime.app", "runtime"))
        .isFile(),
      true,
    );
    assert.equal(fs.existsSync(result.plan.launcherExecutablePath), false);
    assert.equal(fs.existsSync(result.plan.appBundlePath), false);
    assert.equal(fs.existsSync(result.plan.payloadStagingDir), false);
    assert.equal(fs.existsSync(result.plan.resourceStagingDir), false);
    assert.equal(fs.existsSync(result.plan.seedStagingDir), true);
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("mac capsule-only package rejects non-mac platforms before building", () => {
  assert.throws(
    () =>
      packageMacCapsule({
        packageRuntimeCapsule() {
          throw new Error("should not build");
        },
        platform: "linux",
      }),
    /requires codesign/,
  );
});

test("capsule-only activation ids are stable and bounded", () => {
  const releaseId = `sha256:${"a".repeat(64)}`;
  assert.match(
    defaultActivationId(releaseId, 123),
    /^capsule-123-[0-9a-f]{16}$/,
  );
  assert.equal(normalizeActivationId(" capsule_123.ok "), "capsule_123.ok");
  assert.equal(normalizeActivationId(""), null);
  assert.throws(
    () => normalizeActivationId("."),
    /Invalid Runtime Capsule activation id/,
  );
  assert.throws(
    () => normalizeActivationId(".."),
    /Invalid Runtime Capsule activation id/,
  );
  assert.throws(
    () => normalizeActivationId("bad/id"),
    /Invalid Runtime Capsule activation id/,
  );
});
