const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  resolveInstalledArtifactUpdatePlan,
  updateInstalledArtifacts,
} = require("./installedArtifactUpdate.cjs");

test("resolves installed update plan from current app resources path", () => {
  const plan = resolveInstalledArtifactUpdatePlan({
    env: { MORPHEUS_HOME: "/Users/example/.morpheus" },
    platform: "darwin",
    resourcesPath:
      "/Applications/Root Worker Prototype.app/Contents/Resources",
    isPackaged: true,
  });

  assert.equal(
    plan.appBundlePath,
    "/Applications/Root Worker Prototype.app",
  );
  assert.equal(
    plan.workspace,
    "/Users/example/.morpheus/source_workspace",
  );
  assert.equal(
    plan.sourceAppDir,
    "/Users/example/.morpheus/source_workspace/apps/root-worker-prototype",
  );
  assert.equal(
    plan.stagedResourcesPath,
    "/Users/example/.morpheus/source_workspace/apps/root-worker-prototype/dist-app/Root Worker Prototype-darwin-arm64/Root Worker Prototype.app/Contents/Resources",
  );
});

test("resolves installed update plan from explicit workspace", () => {
  const plan = resolveInstalledArtifactUpdatePlan({
    env: {
      MORPHEUS_HOME: "/Users/example/.morpheus",
      ROOT_WORKER_WORKSPACE: "/Volumes/Work/Morpheus Source",
    },
    platform: "darwin",
    resourcesPath:
      "/Volumes/Apps/Root Worker Prototype.app/Contents/Resources",
    isPackaged: true,
  });

  assert.equal(plan.workspace, "/Volumes/Work/Morpheus Source");
  assert.equal(plan.appBundlePath, "/Volumes/Apps/Root Worker Prototype.app");
});

test("does not plan installed artifact update outside packaged mac app", () => {
  assert.equal(
    resolveInstalledArtifactUpdatePlan({
      platform: "darwin",
      resourcesPath: "/repo/apps/root-worker-prototype",
      isPackaged: false,
    }),
    null,
  );
  assert.equal(
    resolveInstalledArtifactUpdatePlan({
      platform: "linux",
      resourcesPath: "/app/Contents/Resources",
      isPackaged: true,
    }),
    null,
  );
});

test("build failure leaves installed artifacts unchanged", () => {
  const fixture = createUpdateFixture();
  const calls = [];

  assert.throws(
    () =>
      updateInstalledArtifacts(fixture.plan, {
        spawnSync: (command, args) => {
          calls.push([command, args]);
          return { status: 1, stderr: "build failed" };
        },
      }),
    /package:mac:app exited with 1: build failed/,
  );

  assert.equal(read(fixture.targetAppAsar), "old asar");
  assert.equal(read(fixture.targetAppServer), "old server");
  assert.equal(read(fixture.targetCompact), "old compact");
  assert.equal(calls.length, 1);
});

test("successful update replaces runnable artifacts and codesigns installed app", () => {
  const fixture = createUpdateFixture();
  const calls = [];

  const result = updateInstalledArtifacts(fixture.plan, {
    updateId: "unit",
    spawnSync: (command, args) => {
      calls.push([command, args]);
      return { status: 0 };
    },
  });

  assert.equal(result.ok, true);
  assert.equal(result.updated, true);
  assert.equal(read(fixture.targetAppAsar), "new asar");
  assert.equal(read(fixture.targetAppServer), "new server");
  assert.equal(read(fixture.targetCompact), "new compact");
  assert.deepEqual(calls, [
    [
      "rtk",
      ["pnpm", "--dir", fixture.plan.sourceAppDir, "package:mac:app"],
    ],
    [
      "rtk",
      [
        "codesign",
        "--force",
        "--deep",
        "--sign",
        "-",
        fixture.plan.appBundlePath,
      ],
    ],
  ]);
});

test("codesign failure restores old installed artifacts", () => {
  const fixture = createUpdateFixture();

  assert.throws(
    () =>
      updateInstalledArtifacts(fixture.plan, {
        updateId: "codesign-failure",
        spawnSync: (command, args) => {
          if (args.includes("codesign")) {
            return { status: 1, stderr: "signature failed" };
          }
          return { status: 0 };
        },
      }),
    /codesign.*signature failed/,
  );

  assert.equal(read(fixture.targetAppAsar), "old asar");
  assert.equal(read(fixture.targetAppServer), "old server");
  assert.equal(read(fixture.targetCompact), "old compact");
  assert.equal(read(fixture.targetSignature), "old signature");
});

test("backup rename failure restores already moved artifacts", () => {
  const fixture = createUpdateFixture();
  let renameCount = 0;
  const realRenameSync = fs.renameSync;

  assert.throws(
    () =>
      updateInstalledArtifacts(fixture.plan, {
        updateId: "backup-failure",
        spawnSync: () => ({ status: 0 }),
        renameSync: (from, to) => {
          renameCount += 1;
          if (renameCount === 2 && to.includes("backup-failure")) {
            throw new Error("backup rename failed");
          }
          realRenameSync(from, to);
        },
      }),
    /backup rename failed/,
  );

  assert.equal(read(fixture.targetAppAsar), "old asar");
  assert.equal(read(fixture.targetAppServer), "old server");
  assert.equal(read(fixture.targetCompact), "old compact");
});

test("install rename failure restores old installed artifacts", () => {
  const fixture = createUpdateFixture();
  let installRenameCount = 0;
  const realRenameSync = fs.renameSync;

  assert.throws(
    () =>
      updateInstalledArtifacts(fixture.plan, {
        updateId: "install-failure",
        spawnSync: () => ({ status: 0 }),
        renameSync: (from, to) => {
          if (from.includes(".morpheus-update-staging-install-failure")) {
            installRenameCount += 1;
            if (installRenameCount === 2) {
              throw new Error("install rename failed");
            }
          }
          realRenameSync(from, to);
        },
      }),
    /install rename failed/,
  );

  assert.equal(read(fixture.targetAppAsar), "old asar");
  assert.equal(read(fixture.targetAppServer), "old server");
  assert.equal(read(fixture.targetCompact), "old compact");
});

function createUpdateFixture() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "morpheus-update-"));
  const workspace = path.join(root, "source workspace");
  const sourceAppDir = path.join(workspace, "apps/root-worker-prototype");
  const installedResources = path.join(
    root,
    "Moved Root Worker Prototype.app/Contents/Resources",
  );
  const stagedResources = path.join(
    sourceAppDir,
    "dist-app/Root Worker Prototype-darwin-arm64/Root Worker Prototype.app/Contents/Resources",
  );
  const plan = resolveInstalledArtifactUpdatePlan({
    env: { ROOT_WORKER_WORKSPACE: workspace },
    platform: "darwin",
    resourcesPath: installedResources,
    isPackaged: true,
  });

  write(path.join(installedResources, "app.asar"), "old asar");
  write(path.join(installedResources, "bin/app-server"), "old server");
  write(path.join(installedResources, "default-config/compact/COMPACT.md"), "old compact");
  write(
    path.join(root, "Moved Root Worker Prototype.app/Contents/_CodeSignature/CodeResources"),
    "old signature",
  );
  write(path.join(stagedResources, "app.asar"), "new asar");
  write(path.join(stagedResources, "bin/app-server"), "new server");
  write(path.join(stagedResources, "default-config/compact/COMPACT.md"), "new compact");
  fs.mkdirSync(sourceAppDir, { recursive: true });

  return {
    plan,
    targetAppAsar: path.join(installedResources, "app.asar"),
    targetAppServer: path.join(installedResources, "bin/app-server"),
    targetCompact: path.join(
      installedResources,
      "default-config/compact/COMPACT.md",
    ),
    targetSignature: path.join(
      root,
      "Moved Root Worker Prototype.app/Contents/_CodeSignature/CodeResources",
    ),
  };
}

function write(filePath, content) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, content);
}

function read(filePath) {
  return fs.readFileSync(filePath, "utf8");
}
