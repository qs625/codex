"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const {
  PROCESS_SUPERVISION_CONTRACT,
  PROHIBITED_PROCESS_BEHAVIORS,
  createRuntimeCapsule,
  validateRuntimeCapsuleManifest,
} = require("./runtimeCapsule.cjs");

test("creates a v2 Capsule without a payload readiness contract", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "runtime-capsule-"));
  try {
    fs.mkdirSync(path.join(root, "bin"));
    fs.writeFileSync(path.join(root, "bin", "runtime"), "#!/bin/sh\n", {
      mode: 0o755,
    });
    const manifest = createRuntimeCapsule(root, {
      arch: "arm64",
      executable: "bin/runtime",
      os: "darwin",
    });

    assert.equal(manifest.schemaVersion, 2);
    assert.equal(Object.hasOwn(manifest.launch, "readiness"), false);
    assert.match(manifest.releaseId, /^sha256:[0-9a-f]{64}$/);
    assert.doesNotThrow(() => validateRuntimeCapsuleManifest(manifest));
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("Capsule validation continues to require cooperative process supervision", () => {
  assert.throws(
    () =>
      validateRuntimeCapsuleManifest({
        schemaVersion: 2,
        releaseId: `sha256:${"0".repeat(64)}`,
        target: { os: "darwin", arch: "arm64" },
        launch: { executable: "bin/runtime", arguments: [] },
        processSupervision: {
          contract: PROCESS_SUPERVISION_CONTRACT,
          prohibitedBehaviors: ["daemonize"],
        },
        entries: [
          { type: "directory", path: "bin" },
          {
            type: "file",
            path: "bin/runtime",
            sha256: "0".repeat(64),
            executable: true,
          },
        ],
        metadata: {},
      }),
    /cooperative observed supervision/,
  );
  assert.equal(PROHIBITED_PROCESS_BEHAVIORS.includes("setsid"), true);
});
