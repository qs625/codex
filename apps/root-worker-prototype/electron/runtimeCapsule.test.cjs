"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const {
  PROCESS_SUPERVISION_CONTRACT,
  PROHIBITED_PROCESS_BEHAVIORS,
  computeRuntimeCapsulePreimage,
  computeRuntimeCapsuleReleaseId,
  createRuntimeCapsule,
  validateRuntimeCapsuleManifest,
} = require("./runtimeCapsule.cjs");

function cooperativeProcessSupervision() {
  return {
    contract: PROCESS_SUPERVISION_CONTRACT,
    prohibitedBehaviors: [...PROHIBITED_PROCESS_BEHAVIORS],
  };
}

test("release preimage matches the Rust Capsule v1 golden vector", () => {
  const manifest = {
    schemaVersion: 1,
    releaseId: `sha256:${"0".repeat(64)}`,
    target: { os: "toy-os", arch: "toy-arch" },
    launch: {
      executable: "bin/runtime",
      arguments: ["--mode", "toy"],
      cwd: "work",
      readiness: { protocol: "launcher-ready-v1", timeoutMs: 5_000 },
    },
    processSupervision: cooperativeProcessSupervision(),
    entries: [
      { type: "symlink", path: "current", target: "work" },
      {
        type: "file",
        path: "bin/runtime",
        sha256:
          "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        executable: true,
      },
      { type: "directory", path: "work" },
      { type: "directory", path: "bin" },
    ],
    metadata: { opaque: ["ignored", 1] },
  };
  assert.equal(
    computeRuntimeCapsulePreimage(manifest).toString("hex"),
    [
      "72756e74696d652d63617073756c652d763100",
      "0006736368656d61000000000000000400000001",
      "00097461726765742e6f730000000000000006746f792d6f73",
      "000b7461726765742e617263680000000000000008746f792d61726368",
      "00116c61756e63682e65786563757461626c65000000000000000b62696e2f72756e74696d65",
      "00126c61756e63682e6377642e70726573656e74000000000000000101",
      "000a6c61756e63682e6377640000000000000004776f726b",
      "00166c61756e63682e617267756d656e74732e636f756e7400000000000000080000000000000002",
      "00116c61756e63682e617267756d656e742e3000000000000000062d2d6d6f6465",
      "00116c61756e63682e617267756d656e742e310000000000000003746f79",
      "001272656164696e6573732e70726f746f636f6c00000000000000116c61756e636865722d72656164792d7631",
      "001472656164696e6573732e74696d656f75745f6d7300000000000000080000000000001388",
      "001c70726f636573735f7375706572766973696f6e2e636f6e74726163740000000000000017636f6f70657261746976652d6f627365727665642d7631",
      "002470726f636573735f7375706572766973696f6e2e70726f686962697465642e636f756e7400000000000000080000000000000003",
      "001e70726f636573735f7375706572766973696f6e2e70726f6869626974656400000000000000096461656d6f6e697a65",
      "001e70726f636573735f7375706572766973696f6e2e70726f68696269746564000000000000000b646f75626c652d666f726b",
      "001e70726f636573735f7375706572766973696f6e2e70726f686962697465640000000000000006736574736964",
      "000d656e74726965732e636f756e7400000000000000080000000000000004",
      "000c656e7472792e302e70617468000000000000000362696e",
      "000c656e7472792e302e7479706500000000000000096469726563746f7279",
      "000c656e7472792e312e70617468000000000000000b62696e2f72756e74696d65",
      "000c656e7472792e312e74797065000000000000000466696c65",
      "000e656e7472792e312e7368613235360000000000000020000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
      "0012656e7472792e312e65786563757461626c65000000000000000101",
      "000c656e7472792e322e70617468000000000000000763757272656e74",
      "000c656e7472792e322e74797065000000000000000773796d6c696e6b",
      "000e656e7472792e322e7461726765740000000000000004776f726b",
      "000c656e7472792e332e706174680000000000000004776f726b",
      "000c656e7472792e332e7479706500000000000000096469726563746f7279",
    ].join(""),
  );
});

test("creates a deterministic Capsule manifest with directory symlinks", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "runtime-capsule-"));
  try {
    fs.mkdirSync(path.join(root, "payload", "Versions", "A"), {
      recursive: true,
    });
    fs.mkdirSync(path.join(root, "payload", "Versions", "A", "Resources"));
    const executable = path.join(
      root,
      "payload",
      "Versions",
      "A",
      "runtime",
    );
    fs.writeFileSync(executable, "#!/bin/sh\nexit 0\n", { mode: 0o755 });
    fs.symlinkSync("A", path.join(root, "payload", "Versions", "Current"));
    fs.symlinkSync(
      "Versions/Current/Resources",
      path.join(root, "payload", "Resources"),
    );
    const manifest = createRuntimeCapsule(root, {
      arch: "arm64",
      executable: "payload/Versions/A/runtime",
      metadata: { sourceCommit: "ignored-by-release-id" },
      os: "darwin",
      readinessTimeoutMs: 12_000,
    });
    assert.match(manifest.releaseId, /^sha256:[0-9a-f]{64}$/);
    assert.equal(
      manifest.entries.find(
        (entry) => entry.path === "payload/Versions/Current",
      )?.type,
      "symlink",
    );
    assert.equal(
      manifest.entries.find((entry) => entry.path === "payload/Resources")
        ?.target,
      "Versions/Current/Resources",
    );
    assert.equal(
      computeRuntimeCapsuleReleaseId({
        ...manifest,
        metadata: { sourceCommit: "also-ignored" },
      }),
      manifest.releaseId,
    );
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("rejects a launch executable that is a symlink", () => {
  assert.throws(
    () =>
      validateRuntimeCapsuleManifest({
        schemaVersion: 1,
        releaseId: `sha256:${"0".repeat(64)}`,
        target: { os: "darwin", arch: "arm64" },
        launch: {
          executable: "runtime",
          arguments: [],
          readiness: {
            protocol: "launcher-ready-v1",
            timeoutMs: 10_000,
          },
        },
        processSupervision: cooperativeProcessSupervision(),
        entries: [
          { type: "symlink", path: "runtime", target: "bin/runtime" },
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
    /launch executable must name a declared executable file/,
  );
});

test("requires the trusted cooperative process supervision contract", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "runtime-capsule-"));
  try {
    fs.mkdirSync(path.join(root, "bin"));
    fs.writeFileSync(path.join(root, "bin", "runtime"), "#!/bin/sh\n", {
      mode: 0o755,
    });
    const manifest = createRuntimeCapsule(root, {
      executable: "bin/runtime",
      os: "darwin",
      arch: "arm64",
    });
    assert.throws(
      () =>
        validateRuntimeCapsuleManifest({
          ...manifest,
          processSupervision: {
            ...manifest.processSupervision,
            prohibitedBehaviors: ["daemonize"],
          },
        }),
      /cooperative observed supervision/,
    );
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("rejects nested capsule.json sidecars", () => {
  const base = {
    schemaVersion: 1,
    releaseId: `sha256:${"0".repeat(64)}`,
    target: { os: "darwin", arch: "arm64" },
    launch: {
      executable: "bin/runtime",
      arguments: [],
      readiness: { protocol: "launcher-ready-v1", timeoutMs: 10_000 },
    },
    processSupervision: cooperativeProcessSupervision(),
    entries: [
      { type: "directory", path: "bin" },
      {
        type: "file",
        path: "bin/runtime",
        sha256: "0".repeat(64),
        executable: true,
      },
      {
        type: "file",
        path: "nested/capsule.json",
        sha256: "0".repeat(64),
        executable: false,
      },
    ],
    metadata: {},
  };
  assert.throws(
    () => validateRuntimeCapsuleManifest(base),
    /reserved for the Capsule root sidecar/,
  );
});

test("rejects symlink escape, undeclared targets, and cycles", () => {
  const base = {
    schemaVersion: 1,
    releaseId: `sha256:${"0".repeat(64)}`,
    target: { os: "darwin", arch: "arm64" },
    launch: {
      executable: "bin/runtime",
      arguments: [],
      readiness: { protocol: "launcher-ready-v1", timeoutMs: 10_000 },
    },
    processSupervision: cooperativeProcessSupervision(),
    metadata: {},
  };
  const executable = {
    type: "file",
    path: "bin/runtime",
    sha256: "0".repeat(64),
    executable: true,
  };
  assert.throws(
    () =>
      validateRuntimeCapsuleManifest({
        ...base,
        entries: [
          { type: "directory", path: "bin" },
          executable,
          { type: "symlink", path: "bad", target: "../outside" },
        ],
      }),
    /escapes the Capsule root/,
  );
  assert.throws(
    () =>
      validateRuntimeCapsuleManifest({
        ...base,
        entries: [
          { type: "directory", path: "bin" },
          executable,
          { type: "symlink", path: "bad", target: "missing" },
        ],
      }),
    /undeclared entry missing/,
  );
  assert.throws(
    () =>
      validateRuntimeCapsuleManifest({
        ...base,
        entries: [
          { type: "directory", path: "bin" },
          executable,
          { type: "symlink", path: "a", target: "b" },
          { type: "symlink", path: "b", target: "a" },
        ],
      }),
    /symlink cycle/,
  );
});

test("rejects symlink chains longer than the Launcher protocol limit", () => {
  const symlinks = Array.from({ length: 65 }, (_, index) => ({
    type: "symlink",
    path: `link-${index}`,
    target: index === 64 ? "target" : `link-${index + 1}`,
  }));
  assert.throws(
    () =>
      validateRuntimeCapsuleManifest({
        schemaVersion: 1,
        releaseId: `sha256:${"0".repeat(64)}`,
        target: { os: "darwin", arch: "arm64" },
        launch: {
          executable: "bin/runtime",
          arguments: [],
          readiness: {
            protocol: "launcher-ready-v1",
            timeoutMs: 10_000,
          },
        },
        processSupervision: cooperativeProcessSupervision(),
        entries: [
          { type: "directory", path: "bin" },
          {
            type: "file",
            path: "bin/runtime",
            sha256: "0".repeat(64),
            executable: true,
          },
          ...symlinks,
          { type: "directory", path: "target" },
        ],
        metadata: {},
      }),
    /symlink hop limit exceeded/,
  );
});

test("rejects raw entries below a symlink alias", () => {
  assert.throws(
    () =>
      validateRuntimeCapsuleManifest({
        schemaVersion: 1,
        releaseId: `sha256:${"0".repeat(64)}`,
        target: { os: "darwin", arch: "arm64" },
        launch: {
          executable: "bin/runtime",
          arguments: [],
          readiness: {
            protocol: "launcher-ready-v1",
            timeoutMs: 10_000,
          },
        },
        processSupervision: cooperativeProcessSupervision(),
        entries: [
          { type: "directory", path: "bin" },
          {
            type: "file",
            path: "bin/runtime",
            sha256: "0".repeat(64),
            executable: true,
          },
          { type: "directory", path: "Versions" },
          { type: "directory", path: "Versions/A" },
          { type: "symlink", path: "Versions/Current", target: "A" },
          {
            type: "file",
            path: "Versions/Current/duplicate",
            sha256: "0".repeat(64),
            executable: false,
          },
        ],
        metadata: {},
      }),
    /undeclared directory parent Versions\/Current/,
  );
});
