const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  prepareDirectArtifacts,
  resolveInstalledArtifactUpdatePlan,
  replaceInstalledArtifactsSync,
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
    plan.frontendDistPath,
    "/Users/example/.morpheus/source_workspace/apps/root-worker-prototype/dist",
  );
  assert.equal(
    plan.appServerBinaryPath,
    "/Users/example/.morpheus/source_workspace/codex-rs/target/release/app-server",
  );
  assert.equal(
    plan.defaultCompactPromptSourcePath,
    "/Users/example/.morpheus/source_workspace/codex-rs/thread-service/templates/compact/prompt.md",
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

test("missing frontend dist fails before replacing installed artifacts", () => {
  const fixture = createUpdateFixture();
  fs.rmSync(fixture.sourceDist, { force: true, recursive: true });

  assert.throws(
    () =>
      updateInstalledArtifacts(fixture.plan, {
        directStagingRoot: fixture.directStagingRoot,
        spawnSync: failUnexpectedSpawn,
      }),
    /Missing frontend dist/,
  );

  assert.equal(read(fixture.targetAppAsar), "old asar");
  assert.equal(read(fixture.targetAppServer), "old server");
  assert.equal(read(fixture.targetCompact), "old compact");
  assert.equal(fs.existsSync(fixture.directStagingRoot), false);
});

test("asar pack failure leaves installed artifacts unchanged and includes output", () => {
  const fixture = createUpdateFixture();
  const calls = [];

  assert.throws(
    () =>
      updateInstalledArtifacts(fixture.plan, {
        directStagingRoot: fixture.directStagingRoot,
        spawnSync: (command, args) => {
          calls.push([command, args]);
          return { status: 1, stdout: "packing stdout", stderr: "pack failed" };
        },
      }),
    /@electron\/asar pack.*stdout=packing stdout.*stderr=pack failed/,
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
    directStagingRoot: fixture.directStagingRoot,
    updateId: "unit",
    spawnSync: (command, args) => {
      calls.push([command, args]);
      if (args.includes("@electron/asar")) {
        write(args.at(-1), "new asar");
      }
      return { status: 0 };
    },
  });

  assert.equal(result.ok, true);
  assert.equal(result.updated, true);
  assert.equal(read(fixture.targetAppAsar), "new asar");
  assert.equal(read(fixture.targetAppServer), "new server");
  assert.equal(read(fixture.targetCompact), "new compact");
  assert.equal(
    calls.some(([_command, args]) => args.includes("package:mac:app")),
    false,
  );
  assert.deepEqual(calls, [
    [
      "rtk",
      [
        "pnpm",
        "dlx",
        "@electron/asar",
        "pack",
        path.join(fixture.directStagingRoot, "app-source"),
        path.join(fixture.directStagingRoot, "resources/app.asar"),
      ],
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
        directStagingRoot: fixture.directStagingRoot,
        updateId: "codesign-failure",
        spawnSync: (command, args) => {
          if (args.includes("codesign")) {
            return { status: 1, stderr: "signature failed" };
          }
          if (args.includes("@electron/asar")) {
            write(args.at(-1), "new asar");
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
        directStagingRoot: fixture.directStagingRoot,
        updateId: "backup-failure",
        spawnSync: (_command, args) => {
          if (args.includes("@electron/asar")) {
            write(args.at(-1), "new asar");
          }
          return { status: 0 };
        },
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
        directStagingRoot: fixture.directStagingRoot,
        updateId: "install-failure",
        spawnSync: (_command, args) => {
          if (args.includes("@electron/asar")) {
            write(args.at(-1), "new asar");
          }
          return { status: 0 };
        },
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

test("postcondition failure restores old installed artifacts", () => {
  const fixture = createUpdateFixture();

  assert.throws(
    () =>
      updateInstalledArtifacts(fixture.plan, {
        directStagingRoot: fixture.directStagingRoot,
        updateId: "postcondition-failure",
        spawnSync: (_command, args) => {
          if (args.includes("@electron/asar")) {
            write(args.at(-1), "new asar");
          }
          return { status: 0 };
        },
        replaceArtifacts(plan, options) {
          const replacement = replaceInstalledArtifactsSync(plan, options);
          fs.writeFileSync(fixture.targetAppAsar, "corrupted asar");
          return replacement;
        },
      }),
    /Installed artifact does not match staged artifact/,
  );

  assert.equal(read(fixture.targetAppAsar), "old asar");
  assert.equal(read(fixture.targetAppServer), "old server");
  assert.equal(read(fixture.targetCompact), "old compact");
});

test("prepareDirectArtifacts copies built outputs without full app packaging", () => {
  const fixture = createUpdateFixture();
  const calls = [];

  const prepared = prepareDirectArtifacts(fixture.plan, {
    directStagingRoot: fixture.directStagingRoot,
    spawnSync: (command, args) => {
      calls.push([command, args]);
      write(args.at(-1), "new asar");
      return { status: 0 };
    },
  });

  assert.equal(
    read(path.join(prepared.stagedResourcesPath, "app.asar")),
    "new asar",
  );
  assert.equal(
    read(path.join(prepared.stagedResourcesPath, "bin/app-server")),
    "new server",
  );
  assert.equal(
    read(
      path.join(
        prepared.stagedResourcesPath,
        "default-config/compact/COMPACT.md",
      ),
    ),
    "new compact",
  );
  assert.equal(calls[0][1].includes("package:mac:app"), false);
});

function createUpdateFixture() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "morpheus-update-"));
  const workspace = path.join(root, "source workspace");
  const sourceAppDir = path.join(workspace, "apps/root-worker-prototype");
  const sourceDist = path.join(sourceAppDir, "dist");
  const installedResources = path.join(
    root,
    "Moved Root Worker Prototype.app/Contents/Resources",
  );
  const directStagingRoot = path.join(root, "direct-staging");
  const plan = resolveInstalledArtifactUpdatePlan({
    env: { ROOT_WORKER_WORKSPACE: workspace },
    platform: "darwin",
    resourcesPath: installedResources,
    isPackaged: true,
  });

  write(path.join(installedResources, "app.asar"), "old asar");
  write(path.join(installedResources, "bin/app-server"), "old server");
  write(
    path.join(installedResources, "default-config/compact/COMPACT.md"),
    "old compact",
  );
  write(
    path.join(
      root,
      "Moved Root Worker Prototype.app/Contents/_CodeSignature/CodeResources",
    ),
    "old signature",
  );
  write(path.join(sourceDist, "index.html"), "<main>new renderer</main>");
  write(
    path.join(workspace, "codex-rs/target/release/app-server"),
    "new server",
  );
  write(
    path.join(
      workspace,
      "codex-rs/thread-service/templates/compact/prompt.md",
    ),
    "new compact",
  );
  fs.mkdirSync(sourceAppDir, { recursive: true });

  return {
    directStagingRoot,
    plan,
    sourceDist,
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

function failUnexpectedSpawn(command, args) {
  throw new Error(`unexpected spawn: ${command} ${args.join(" ")}`);
}

function write(filePath, content) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, content);
}

function read(filePath) {
  return fs.readFileSync(filePath, "utf8");
}
