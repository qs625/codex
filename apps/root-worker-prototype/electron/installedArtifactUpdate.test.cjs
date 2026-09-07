const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  buildDirectArtifactSources,
  listElectronShellSourceRelativePaths,
  prepareDirectArtifacts,
  resolveElectronShellUpdate,
  resolveCargoTargetDirectory,
  resolveInstalledArtifactUpdatePlan,
  replaceInstalledArtifactsSync,
  updateInstalledArtifacts,
  updateInstalledArtifactsInWorker,
} = require("./installedArtifactUpdate.cjs");

test("resolves installed update plan from current app resources path", () => {
  const workspace = "/Users/example/.morpheus/source_workspace";
  const plan = resolveInstalledArtifactUpdatePlan({
    env: { MORPHEUS_HOME: "/Users/example/.morpheus" },
    platform: "darwin",
    resourcesPath:
      "/Applications/Root Worker Prototype.app/Contents/Resources",
    isPackaged: true,
    spawnSync: fakeCargoMetadataSpawn({
      target_directory: path.join(workspace, "target"),
    }),
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
    "/Users/example/.morpheus/source_workspace/target/release/app-server",
  );
  assert.equal(
    plan.defaultCompactPromptSourcePath,
    "/Users/example/.morpheus/source_workspace/codex-rs/thread-service/templates/compact/prompt.md",
  );
});

test("resolves installed update plan from explicit workspace", () => {
  const workspace = "/Volumes/Work/Morpheus Source";
  const plan = resolveInstalledArtifactUpdatePlan({
    env: {
      MORPHEUS_HOME: "/Users/example/.morpheus",
      ROOT_WORKER_WORKSPACE: workspace,
    },
    platform: "darwin",
    resourcesPath:
      "/Volumes/Apps/Root Worker Prototype.app/Contents/Resources",
    isPackaged: true,
    spawnSync: fakeCargoMetadataSpawn({
      target_directory: path.join(workspace, ".shared-target"),
    }),
  });

  assert.equal(plan.workspace, "/Volumes/Work/Morpheus Source");
  assert.equal(plan.appBundlePath, "/Volumes/Apps/Root Worker Prototype.app");
  assert.equal(
    plan.appServerBinaryPath,
    "/Volumes/Work/Morpheus Source/.shared-target/release/app-server",
  );
});

test("resolves release app-server from cargo metadata target directory", () => {
  const workspace = "/repo/source";
  const calls = [];
  const commandEnv = {
    CARGO_TARGET_DIR: "/repo/target",
    PATH: "/test/bin",
  };
  const plan = resolveInstalledArtifactUpdatePlan({
    commandEnv,
    env: { ROOT_WORKER_WORKSPACE: workspace },
    platform: "darwin",
    resourcesPath: "/Applications/Root Worker Prototype.app/Contents/Resources",
    isPackaged: true,
    spawnSync: fakeCargoMetadataSpawn(
      { target_directory: "/repo/target" },
      calls,
    ),
  });

  assert.equal(plan.appServerBinaryPath, "/repo/target/release/app-server");
  assert.deepEqual(calls, [
    {
      command: "cargo",
      args: [
        "metadata",
        "--format-version=1",
        "--no-deps",
        "--manifest-path",
        "/repo/source/codex-rs/Cargo.toml",
      ],
      cwd: "/repo/source/codex-rs",
      env: commandEnv,
    },
  ]);
  assert.equal(plan.commandEnv, commandEnv);
});

test("cargo target directory parsing rejects missing metadata field", () => {
  assert.throws(
    () =>
      resolveCargoTargetDirectory({
        codexRsCargoManifestPath: "/repo/source/codex-rs/Cargo.toml",
        codexRsDir: "/repo/source/codex-rs",
        spawnSync: fakeCargoMetadataSpawn({}),
      }),
    /Cargo metadata did not include target_directory/,
  );
});

test("cargo metadata failure reports the direct command and workspace", () => {
  assert.throws(
    () =>
      resolveCargoTargetDirectory({
        codexRsCargoManifestPath: "/repo/source/codex-rs/Cargo.toml",
        codexRsDir: "/repo/source/codex-rs",
        spawnSync: () => ({
          status: 1,
          stderr: "metadata failed",
        }),
      }),
    /cargo metadata.*cwd=\/repo\/source\/codex-rs.*metadata failed/,
  );
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

test("electron shell update metadata tracks shell runtime digests", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "morpheus-shell-digest-"));
  const resourcesPath = path.join(root, "app/Contents/Resources");
  const sourceAppDir = path.join(root, "source/apps/root-worker-prototype");
  const relativePaths = ["electron/main.cjs", "electron/preload.cjs"];
  for (const relativePath of relativePaths) {
    write(path.join(sourceAppDir, relativePath), `${relativePath}:same`);
    write(
      path.join(resourcesPath, "app.asar", relativePath),
      `${relativePath}:same`,
    );
  }

  assert.deepEqual(
    resolveElectronShellUpdate({
      resourcesPath,
      sourceAppDir,
      relativePaths,
    }),
    {
      category: "electronShell",
      changed: false,
      changedPaths: [],
      missingInstalledPaths: [],
      missingSourcePaths: [],
    },
  );

  write(path.join(sourceAppDir, "electron/preload.cjs"), "new preload");

  assert.deepEqual(
    resolveElectronShellUpdate({
      resourcesPath,
      sourceAppDir,
      relativePaths,
    }),
    {
      category: "electronShell",
      changed: true,
      changedPaths: ["electron/preload.cjs"],
      missingInstalledPaths: [],
      missingSourcePaths: [],
    },
  );
});

test("installed update plan marks full relaunch when shell runtime changes", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "morpheus-plan-shell-"));
  const workspace = path.join(root, "source");
  const resourcesPath = path.join(
    root,
    "Root Worker Prototype.app/Contents/Resources",
  );
  const sourceAppDir = path.join(workspace, "apps/root-worker-prototype");
  const relativePaths = ["electron/main.cjs", "electron/preload.cjs"];
  for (const relativePath of relativePaths) {
    write(path.join(sourceAppDir, relativePath), `${relativePath}:old`);
    write(
      path.join(resourcesPath, "app.asar", relativePath),
      `${relativePath}:old`,
    );
  }
  write(path.join(sourceAppDir, "electron/preload.cjs"), "new preload");

  const plan = resolveInstalledArtifactUpdatePlan({
    env: { ROOT_WORKER_WORKSPACE: workspace },
    platform: "darwin",
    resourcesPath,
    isPackaged: true,
    spawnSync: fakeCargoMetadataSpawn({
      target_directory: path.join(workspace, "target"),
    }),
  });

  assert.equal(plan.requiresFullRelaunch, true);
  assert.deepEqual(plan.runtimeUpdate.electronShell.changedPaths, [
    "electron/preload.cjs",
  ]);
});

test("installed update plan keeps hot reload when shell runtime is unchanged", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "morpheus-plan-renderer-"));
  const workspace = path.join(root, "source");
  const resourcesPath = path.join(
    root,
    "Root Worker Prototype.app/Contents/Resources",
  );
  const sourceAppDir = path.join(workspace, "apps/root-worker-prototype");
  for (const relativePath of ["electron/main.cjs", "electron/preload.cjs"]) {
    write(path.join(sourceAppDir, relativePath), `${relativePath}:same`);
    write(
      path.join(resourcesPath, "app.asar", relativePath),
      `${relativePath}:same`,
    );
  }

  const plan = resolveInstalledArtifactUpdatePlan({
    env: { ROOT_WORKER_WORKSPACE: workspace },
    platform: "darwin",
    resourcesPath,
    isPackaged: true,
    spawnSync: fakeCargoMetadataSpawn({
      target_directory: path.join(workspace, "target"),
    }),
  });

  assert.equal(plan.requiresFullRelaunch, false);
  assert.deepEqual(plan.runtimeUpdate.electronShell.changedPaths, []);
});

test("installed update plan full relaunches when source deletes shell runtime", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "morpheus-plan-delete-"));
  const workspace = path.join(root, "source");
  const resourcesPath = path.join(
    root,
    "Root Worker Prototype.app/Contents/Resources",
  );
  const sourceAppDir = path.join(workspace, "apps/root-worker-prototype");
  write(path.join(sourceAppDir, "electron/main.cjs"), "main");
  write(path.join(resourcesPath, "app.asar/electron/main.cjs"), "main");
  write(path.join(resourcesPath, "app.asar/electron/preload.cjs"), "old preload");

  const plan = resolveInstalledArtifactUpdatePlan({
    env: { ROOT_WORKER_WORKSPACE: workspace },
    platform: "darwin",
    resourcesPath,
    isPackaged: true,
    spawnSync: fakeCargoMetadataSpawn({
      target_directory: path.join(workspace, "target"),
    }),
  });

  assert.equal(plan.requiresFullRelaunch, true);
  assert.deepEqual(plan.runtimeUpdate.electronShell.changedPaths, [
    "electron/preload.cjs",
  ]);
  assert.deepEqual(plan.runtimeUpdate.electronShell.missingSourcePaths, [
    "electron/preload.cjs",
  ]);
});

test("electron shell source manifest includes runtime cjs files and excludes tests", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "morpheus-shell-list-"));
  const sourceAppDir = path.join(root, "apps/root-worker-prototype");
  write(path.join(sourceAppDir, "electron/main.cjs"), "main");
  write(path.join(sourceAppDir, "electron/preload.cjs"), "preload");
  write(path.join(sourceAppDir, "electron/main.test.cjs"), "test");
  write(path.join(sourceAppDir, "electron/Info.plist"), "plist");
  write(path.join(sourceAppDir, "electron/lsp/client.cjs"), "client");
  write(path.join(sourceAppDir, "electron/lsp/client.test.cjs"), "test");

  assert.deepEqual(listElectronShellSourceRelativePaths(sourceAppDir), [
    "electron/lsp/client.cjs",
    "electron/main.cjs",
    "electron/preload.cjs",
  ]);
});

test("builds current workspace frontend and release app-server before preparing artifacts", () => {
  const fixture = createUpdateFixture();
  const calls = [];
  const invocationEnv = { PATH: "/override/bin" };

  buildDirectArtifactSources(fixture.plan, {
    env: invocationEnv,
    spawnSync: (command, args, options = {}) => {
      calls.push({ command, args, cwd: options.cwd, env: options.env });
      return { status: 0 };
    },
  });

  assert.deepEqual(calls, [
    {
      command: "pnpm",
      args: ["--filter", "@my-codex/root-worker-prototype", "build"],
      cwd: fixture.plan.workspace,
      env: fixture.plan.commandEnv,
    },
    {
      command: "cargo",
      args: [
        "build",
        "--release",
        "--package",
        "app-server",
        "--bin",
        "app-server",
        "--manifest-path",
        fixture.plan.codexRsCargoManifestPath,
      ],
      cwd: path.dirname(fixture.plan.codexRsCargoManifestPath),
      env: fixture.plan.commandEnv,
    },
  ]);
});

test("frontend build failure leaves installed artifacts unchanged", () => {
  const fixture = createUpdateFixture();
  const calls = [];

  assert.throws(
    () =>
      updateInstalledArtifacts(fixture.plan, {
        directStagingRoot: fixture.directStagingRoot,
        spawnSync: (command, args, options = {}) => {
          calls.push({ command, args, cwd: options.cwd });
          return { status: 1, stderr: "frontend build failed" };
        },
        replaceArtifacts: failUnexpectedReplacement,
        codesign: failUnexpectedCodesign,
      }),
    /pnpm --filter.*frontend build failed/,
  );

  assert.deepEqual(calls, [
    {
      command: "pnpm",
      args: ["--filter", "@my-codex/root-worker-prototype", "build"],
      cwd: fixture.plan.workspace,
    },
  ]);
  assertInstalledArtifactsUnchanged(fixture);
  assert.equal(fs.existsSync(fixture.directStagingRoot), false);
});

test("missing frontend build output fails before cargo build or replacement", () => {
  const fixture = createUpdateFixture();
  fs.rmSync(fixture.sourceDist, { force: true, recursive: true });
  const calls = [];

  assert.throws(
    () =>
      updateInstalledArtifacts(fixture.plan, {
        directStagingRoot: fixture.directStagingRoot,
        spawnSync: (command, args, options = {}) => {
          calls.push({ command, args, cwd: options.cwd });
          return { status: 0 };
        },
        replaceArtifacts: failUnexpectedReplacement,
        codesign: failUnexpectedCodesign,
      }),
    /Missing frontend dist/,
  );

  assert.equal(calls.length, 1);
  assert.equal(calls[0].command, "pnpm");
  assertInstalledArtifactsUnchanged(fixture);
  assert.equal(fs.existsSync(fixture.directStagingRoot), false);
});

test("release app-server build failure leaves installed artifacts unchanged", () => {
  const fixture = createUpdateFixture();
  const calls = [];

  assert.throws(
    () =>
      updateInstalledArtifacts(fixture.plan, {
        directStagingRoot: fixture.directStagingRoot,
        spawnSync: (command, args, options = {}) => {
          calls.push({ command, args, cwd: options.cwd });
          if (command === "cargo") {
            return { status: 1, stderr: "app-server build failed" };
          }
          return { status: 0 };
        },
        replaceArtifacts: failUnexpectedReplacement,
        codesign: failUnexpectedCodesign,
      }),
    /cargo build --release.*app-server build failed/,
  );

  assert.deepEqual(
    calls.map(({ command }) => command),
    ["pnpm", "cargo"],
  );
  assertInstalledArtifactsUnchanged(fixture);
  assert.equal(fs.existsSync(fixture.directStagingRoot), false);
});

test("missing release app-server build output fails before packing or replacement", () => {
  const fixture = createUpdateFixture();
  fs.rmSync(fixture.plan.appServerBinaryPath, { force: true });
  const calls = [];

  assert.throws(
    () =>
      updateInstalledArtifacts(fixture.plan, {
        directStagingRoot: fixture.directStagingRoot,
        spawnSync: (command, args, options = {}) => {
          calls.push({ command, args, cwd: options.cwd });
          return { status: 0 };
        },
        replaceArtifacts: failUnexpectedReplacement,
        codesign: failUnexpectedCodesign,
      }),
    /Missing release app-server binary/,
  );

  assert.deepEqual(
    calls.map(({ command }) => command),
    ["pnpm", "cargo"],
  );
  assertInstalledArtifactsUnchanged(fixture);
  assert.equal(fs.existsSync(fixture.directStagingRoot), false);
});

test("asar pack failure leaves installed artifacts unchanged and includes output", () => {
  const fixture = createUpdateFixture();
  const calls = [];

  assert.throws(
    () =>
      updateInstalledArtifacts(fixture.plan, {
        directStagingRoot: fixture.directStagingRoot,
        spawnSync: (command, args, options = {}) => {
          calls.push({ command, args, cwd: options.cwd });
          if (!args.includes("@electron/asar")) {
            return { status: 0 };
          }
          return { status: 1, stdout: "packing stdout", stderr: "pack failed" };
        },
        replaceArtifacts: failUnexpectedReplacement,
        codesign: failUnexpectedCodesign,
      }),
    /@electron\/asar pack.*stdout=packing stdout.*stderr=pack failed/,
  );

  assertInstalledArtifactsUnchanged(fixture);
  assert.deepEqual(
    calls.map(({ command }) => command),
    ["pnpm", "cargo", "pnpm"],
  );
});

test("successful update replaces runnable artifacts and codesigns installed app", () => {
  const fixture = createUpdateFixture();
  const calls = [];
  write(path.join(fixture.sourceDist, "index.html"), "stale renderer");
  write(fixture.plan.appServerBinaryPath, "stale server");

  const result = updateInstalledArtifacts(fixture.plan, {
    directStagingRoot: fixture.directStagingRoot,
    updateId: "unit",
    env: { PATH: "/override/bin" },
    spawnSync: (command, args, options = {}) => {
      calls.push({
        command,
        args,
        cwd: options.cwd,
        env: options.env,
      });
      if (command === "pnpm" && args[0] === "--filter") {
        write(path.join(fixture.sourceDist, "index.html"), "fresh renderer");
      }
      if (command === "cargo" && args[0] === "build") {
        write(fixture.plan.appServerBinaryPath, "fresh server");
      }
      if (args.includes("@electron/asar")) {
        assert.equal(
          read(
            path.join(
              fixture.directStagingRoot,
              "app-source/dist/index.html",
            ),
          ),
          "fresh renderer",
        );
        write(args.at(-1), "new asar");
      }
      if (command === "codesign") {
        assertNoBundleUpdateTemporaryDirs(fixture);
      }
      return { status: 0 };
    },
  });

  assert.equal(result.ok, true);
  assert.equal(result.updated, true);
  assert.equal(read(fixture.targetAppAsar), "new asar");
  assert.equal(read(fixture.targetAppServer), "fresh server");
  assert.equal(read(fixture.targetCompact), "new compact");
  assert.equal(
    calls.some(({ args }) => args.includes("package:mac:app")),
    false,
  );
  assert.deepEqual(calls, [
    {
      command: "pnpm",
      args: ["--filter", "@my-codex/root-worker-prototype", "build"],
      cwd: fixture.plan.workspace,
      env: fixture.plan.commandEnv,
    },
    {
      command: "cargo",
      args: [
        "build",
        "--release",
        "--package",
        "app-server",
        "--bin",
        "app-server",
        "--manifest-path",
        fixture.plan.codexRsCargoManifestPath,
      ],
      cwd: path.dirname(fixture.plan.codexRsCargoManifestPath),
      env: fixture.plan.commandEnv,
    },
    {
      command: "pnpm",
      args: [
        "dlx",
        "@electron/asar",
        "pack",
        path.join(fixture.directStagingRoot, "app-source"),
        path.join(fixture.directStagingRoot, "resources/app.asar"),
      ],
      cwd: fixture.plan.workspace,
      env: fixture.plan.commandEnv,
    },
    {
      command: "codesign",
      args: [
        "--force",
        "--deep",
        "--sign",
        "-",
        fixture.plan.appBundlePath,
      ],
      cwd: fixture.plan.workspace,
      env: fixture.plan.commandEnv,
    },
  ]);
});

test("installed artifact worker keeps the Electron event loop responsive", async () => {
  const fixture = createUpdateFixture();
  const fakeBin = path.join(path.dirname(fixture.plan.workspace), "fake-bin");
  writeExecutable(
    path.join(fakeBin, "pnpm"),
    `#!/bin/sh
if [ "$1" = "--filter" ]; then
  sleep 0.2
fi
if [ "$1" = "dlx" ]; then
  for last_arg do :; done
  mkdir -p "$(dirname "$last_arg")"
  printf 'new asar' > "$last_arg"
fi
`,
  );
  writeExecutable(path.join(fakeBin, "cargo"), "#!/bin/sh\nexit 0\n");
  writeExecutable(path.join(fakeBin, "codesign"), "#!/bin/sh\nexit 0\n");
  fixture.plan.commandEnv = {
    ...process.env,
    PATH: `${fakeBin}:${process.env.PATH ?? ""}`,
  };

  let updateSettled = false;
  const update = updateInstalledArtifactsInWorker(fixture.plan).finally(() => {
    updateSettled = true;
  });
  let timerFired = false;
  await new Promise((resolve) => {
    setTimeout(() => {
      timerFired = true;
      resolve();
    }, 25);
  });

  assert.equal(timerFired, true);
  assert.equal(updateSettled, false);
  assert.equal((await update).ok, true);
  assert.equal(read(fixture.targetAppAsar), "new asar");
});

test("codesign failure restores old installed artifacts", () => {
  const fixture = createUpdateFixture();

  assert.throws(
    () =>
      updateInstalledArtifacts(fixture.plan, {
        directStagingRoot: fixture.directStagingRoot,
        updateId: "codesign-failure",
        spawnSync: (command, args) => {
          if (command === "codesign") {
            assertNoBundleUpdateTemporaryDirs(fixture);
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
  assertNoBundleUpdateTemporaryDirs(fixture);
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
            assert.equal(pathStartsWith(from, fixture.appContentsPath), true);
            assert.equal(pathStartsWith(to, fixture.appContentsPath), false);
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
            assert.equal(pathStartsWith(from, fixture.appContentsPath), false);
            assert.equal(pathStartsWith(to, fixture.appContentsPath), true);
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
    spawnSync: (command, args, options = {}) => {
      calls.push({ command, args, cwd: options.cwd });
      if (args.includes("@electron/asar")) {
        write(args.at(-1), "new asar");
      }
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
  assert.deepEqual(
    calls.map(({ command }) => command),
    ["pnpm", "cargo", "pnpm"],
  );
  assert.equal(
    calls.some(({ args }) => args.includes("package:mac:app")),
    false,
  );
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
  const appContentsPath = path.join(
    root,
    "Moved Root Worker Prototype.app/Contents",
  );
  const directStagingRoot = path.join(root, "direct-staging");
  const commandEnv = {
    CARGO_TARGET_DIR: path.join(workspace, "target"),
    PATH: "/test/bin",
  };
  const plan = resolveInstalledArtifactUpdatePlan({
    commandEnv,
    env: { ROOT_WORKER_WORKSPACE: workspace },
    platform: "darwin",
    resourcesPath: installedResources,
    isPackaged: true,
    spawnSync: fakeCargoMetadataSpawn({
      target_directory: path.join(workspace, "target"),
    }),
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
  write(path.join(workspace, "target/release/app-server"), "new server");
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
    appContentsPath,
    plan,
    resourcesPath: installedResources,
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

function failUnexpectedReplacement() {
  throw new Error("replacement should not run");
}

function failUnexpectedCodesign() {
  throw new Error("codesign should not run");
}

function assertInstalledArtifactsUnchanged(fixture) {
  assert.equal(read(fixture.targetAppAsar), "old asar");
  assert.equal(read(fixture.targetAppServer), "old server");
  assert.equal(read(fixture.targetCompact), "old compact");
}

function fakeCargoMetadataSpawn(metadata, calls = []) {
  return (command, args, options = {}) => {
    calls.push({ command, args, cwd: options.cwd, env: options.env });
    return {
      status: 0,
      stdout: JSON.stringify(metadata),
    };
  };
}

function hasBundleSignatureBackup(appContentsPath) {
  if (!fs.existsSync(appContentsPath)) {
    return false;
  }
  return fs
    .readdirSync(appContentsPath)
    .some((entry) => entry.startsWith(".morpheus-signature-backup-"));
}

function assertNoBundleUpdateTemporaryDirs(fixture) {
  assert.equal(hasBundleSignatureBackup(fixture.appContentsPath), false);
  assert.equal(hasUpdaterTemporaryDir(fixture.appContentsPath), false);
  assert.equal(hasUpdaterTemporaryDir(fixture.resourcesPath), false);
}

function hasUpdaterTemporaryDir(directoryPath) {
  if (!fs.existsSync(directoryPath)) {
    return false;
  }
  return fs.readdirSync(directoryPath).some((entry) => {
    return (
      entry.startsWith(".morpheus-update-staging-") ||
      entry.startsWith(".morpheus-update-backup-") ||
      entry.startsWith(".morpheus-signature-backup-")
    );
  });
}

function pathStartsWith(targetPath, parentPath) {
  const relative = path.relative(parentPath, targetPath);
  return Boolean(relative) && !relative.startsWith("..") && !path.isAbsolute(relative);
}

function write(filePath, content) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, content);
}

function writeExecutable(filePath, content) {
  write(filePath, content);
  fs.chmodSync(filePath, 0o755);
}

function read(filePath) {
  return fs.readFileSync(filePath, "utf8");
}
