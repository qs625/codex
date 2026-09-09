const test = require("node:test");
const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  PREPARED_ARTIFACT_OWNER_FILE,
  cleanupOwnedPreparedArtifactsWithLauncher,
  garbageCollectOwnedPreparedArtifacts,
  listInstalledElectronShellRelativePaths,
  listElectronShellSourceRelativePaths,
  materializeInstalledArtifactWorkerBundle,
  packAppAsar,
  prepareInstalledArtifacts,
  releaseOwnedPreparedArtifactLease,
  removeOwnedPreparedArtifact,
  resolvePreparedArtifactsRoot,
  resolveCargoTargetDirectory,
  resolveElectronShellUpdate,
  resolveInstalledArtifactUpdatePlan,
  resolveInstalledArtifactUpdatePlanInWorker,
  updateInstalledArtifactsInWorker,
} = require("./installedArtifactUpdate.cjs");

test("installed update plan consumes existing dist and release app-server", () => {
  const workspace = "/repo";
  const plan = resolveInstalledArtifactUpdatePlan({
    env: {
      ROOT_WORKER_WORKSPACE: workspace,
      CARGO_TARGET_DIR: "/shared/target",
    },
    platform: "darwin",
    resourcesPath: "/Applications/Morpheus.app/Contents/Resources",
    isPackaged: true,
  });

  assert.equal(plan.frontendDistPath, "/repo/apps/root-worker-prototype/dist");
  assert.equal(plan.appServerBinaryPath, "/shared/target/release/app-server");
  assert.equal(plan.appBundlePath, "/Applications/Morpheus.app");
});

test("installed update plan resolves the packaged default source workspace", () => {
  const plan = resolveInstalledArtifactUpdatePlan({
    env: { MORPHEUS_HOME: "/Users/example/.morpheus" },
    platform: "darwin",
    resourcesPath: "/Applications/Morpheus.app/Contents/Resources",
    isPackaged: true,
  });

  assert.equal(plan.workspace, "/Users/example/.morpheus/source_workspace");
  assert.equal(
    plan.appServerBinaryPath,
    "/Users/example/.morpheus/source_workspace/codex-rs/target/release/app-server",
  );
});

test("relative Cargo target directory follows workspace-root build cwd", () => {
  const calls = [];
  const targetDirectory = resolveCargoTargetDirectory({
    cargoCwd: "/repo",
    codexRsDir: "/repo/codex-rs",
    env: { CARGO_TARGET_DIR: "custom-target" },
    spawnSync: (_command, _args, options) => {
      calls.push(options.cwd);
      return {
        status: 0,
        stdout: JSON.stringify({
          target_directory: "/repo/custom-target",
        }),
      };
    },
  });

  assert.equal(targetDirectory, "/repo/custom-target");
  assert.deepEqual(calls, ["/repo"]);
});

test("installed plan resolves relative Cargo target from the workspace build cwd", () => {
  const calls = [];
  const plan = resolveInstalledArtifactUpdatePlan({
    env: {
      ROOT_WORKER_WORKSPACE: "/repo",
      CARGO_TARGET_DIR: "custom-target",
    },
    platform: "darwin",
    resourcesPath: "/Applications/Morpheus.app/Contents/Resources",
    isPackaged: true,
    spawnSync: (_command, _args, options) => {
      calls.push(options.cwd);
      return {
        status: 0,
        stdout: JSON.stringify({
          target_directory: "/repo/custom-target",
        }),
      };
    },
  });

  assert.equal(
    plan.appServerBinaryPath,
    "/repo/custom-target/release/app-server",
  );
  assert.deepEqual(calls, ["/repo"]);
});

test("Finder-style PATH is enhanced once for Cargo metadata and artifact packing", () => {
  const spawnCalls = [];
  const plan = resolveInstalledArtifactUpdatePlan({
    desktopEnvironmentOptions: {
      home: "/Users/example",
      includeShellPath: false,
      platform: "darwin",
    },
    env: {
      CARGO_TARGET_DIR: "custom-target",
      HOME: "/Users/example",
      PATH: "/usr/bin:/bin",
      ROOT_WORKER_WORKSPACE: "/repo",
    },
    platform: "darwin",
    resourcesPath: "/Applications/Morpheus.app/Contents/Resources",
    isPackaged: true,
    spawnSync: (command, args, options) => {
      spawnCalls.push({ command, args, options });
      return {
        status: 0,
        stdout: JSON.stringify({ target_directory: "/repo/custom-target" }),
      };
    },
  });

  packAppAsar(plan, "/tmp/app-source", "/tmp/app.asar", {
    spawnSync: (command, args, options) => {
      spawnCalls.push({ command, args, options });
      return { status: 0 };
    },
  });

  assert.equal(plan.commandEnv.PATH.includes("/opt/homebrew/bin"), true);
  assert.equal(plan.commandEnv.PATH.includes("/Users/example/.local/bin"), true);
  assert.equal(spawnCalls[0].command, "cargo");
  assert.equal(spawnCalls[0].options.env, plan.commandEnv);
  assert.equal(spawnCalls[1].command, "pnpm");
  assert.equal(spawnCalls[1].options.env, plan.commandEnv);
});

test("explicit desktop command environment overrides automatic construction", () => {
  const commandEnv = {
    CARGO_TARGET_DIR: "/explicit/target",
    PATH: "/explicit/bin",
    ROOT_WORKER_WORKSPACE: "/repo",
  };
  const plan = resolveInstalledArtifactUpdatePlan({
    commandEnv,
    env: {
      HOME: "/Users/example",
      PATH: "/usr/bin:/bin",
      ROOT_WORKER_WORKSPACE: "/repo",
    },
    platform: "darwin",
    resourcesPath: "/Applications/Morpheus.app/Contents/Resources",
    isPackaged: true,
  });

  assert.deepEqual(plan.commandEnv, commandEnv);
  assert.notEqual(plan.commandEnv, commandEnv);
  assert.equal(plan.commandEnv.PATH, "/explicit/bin");
  assert.equal(plan.appServerBinaryPath, "/explicit/target/release/app-server");
});

test("relative Cargo target directory follows an explicit codex-rs build cwd", () => {
  const targetDirectory = resolveCargoTargetDirectory({
    cargoCwd: "/repo/codex-rs",
    codexRsDir: "/repo/codex-rs",
    env: { CARGO_TARGET_DIR: "custom-target" },
    spawnSync: (_command, _args, options) => ({
      status: 0,
      stdout: JSON.stringify({
        target_directory: path.join(options.cwd, "custom-target"),
      }),
    }),
  });

  assert.equal(targetDirectory, "/repo/codex-rs/custom-target");
});

test("relative Cargo target fallback remains relative to the build cwd", () => {
  assert.equal(
    resolveCargoTargetDirectory({
      cargoCwd: "/repo",
      codexRsDir: "/repo/codex-rs",
      env: { CARGO_TARGET_DIR: "custom-target" },
      spawnSync: () => ({ status: 1, stderr: "metadata unavailable" }),
    }),
    "/repo/custom-target",
  );
});

test("installed update remains explicitly unsupported on Windows and Linux", () => {
  for (const platform of ["win32", "linux"]) {
    assert.equal(
      resolveInstalledArtifactUpdatePlan({
        env: { ROOT_WORKER_WORKSPACE: "/repo" },
        platform,
        resourcesPath: "/app/resources",
        isPackaged: true,
      }),
      null,
    );
  }
  assert.equal(
    resolveInstalledArtifactUpdatePlan({
      env: { ROOT_WORKER_WORKSPACE: "/repo" },
      platform: "darwin",
      resourcesPath: "/repo/apps/root-worker-prototype",
      isPackaged: false,
    }),
    null,
  );
});

test("prepared artifact contains manifest and hashed resources without builds", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "prepared-runtime-test-"));
  const workspace = path.join(root, "repo");
  const sourceAppDir = path.join(workspace, "apps/root-worker-prototype");
  const preparedArtifactsRoot = path.join(root, "prepared-artifacts");
  const preparedRoot = path.join(
    preparedArtifactsRoot,
    "morpheus-prepared-runtime-test",
  );
  write(path.join(sourceAppDir, "dist/index.html"), "renderer");
  write(path.join(sourceAppDir, "electron/main.cjs"), "main");
  write(path.join(sourceAppDir, "electron/preload.cjs"), "preload");
  write(path.join(sourceAppDir, "package.json"), "{}");
  write(path.join(workspace, "codex-rs/target/release/app-server"), "server");
  write(
    path.join(
      workspace,
      "codex-rs/thread-service/templates/compact/prompt.md",
    ),
    "compact",
  );
  const commandEnv = { PATH: "/test/bin" };
  const spawnCalls = [];
  const result = prepareInstalledArtifacts(
    {
      appBundlePath: "/Applications/Morpheus.app",
      appServerBinaryPath: path.join(
        workspace,
        "codex-rs/target/release/app-server",
      ),
      defaultCompactPromptSourcePath: path.join(
        workspace,
        "codex-rs/thread-service/templates/compact/prompt.md",
      ),
      frontendDistPath: path.join(sourceAppDir, "dist"),
      runtimeUpdate: {
        electronShell: { changedPaths: ["electron/preload.cjs"] },
      },
      commandEnv,
      sourceAppDir,
      workspace,
    },
    {
      now: 0,
      preparedArtifactsRoot,
      preparedRoot,
      publishNow: 700_000,
      sourceCommit: "abc123",
      transactionId: "transaction-1",
      spawnSync: (command, args, options) => {
        spawnCalls.push({ command, args, options });
        assert.equal(fs.existsSync(preparedRoot), false);
        assert.deepEqual(
          garbageCollectOwnedPreparedArtifacts({
            preparedArtifactsRoot,
          }).removed,
          [],
        );
        write(args.at(-1), "asar");
        return { status: 0, stdout: "" };
      },
    },
  );
  const canonicalPreparedRoot = fs.realpathSync(preparedRoot);

  assert.equal(result.transactionId, "transaction-1");
  assert.equal(result.preparedRoot, canonicalPreparedRoot);
  assert.equal(
    result.preparedArtifactOwner.transactionId,
    result.transactionId,
  );
  const publishedMarker = JSON.parse(
    fs.readFileSync(
      path.join(preparedRoot, PREPARED_ARTIFACT_OWNER_FILE),
      "utf8",
    ),
  );
  assert.equal(
    Date.parse(publishedMarker.producerLeaseExpiresAt),
    1_300_000,
  );
  assert.deepEqual(
    garbageCollectOwnedPreparedArtifacts({
      isProcessAlive: () => true,
      now: 700_001,
      preparedArtifactsRoot,
    }).preserved,
    [canonicalPreparedRoot],
  );
  assert.equal(result.sourceCommit, "abc123");
  assert.equal(result.manifest.changes.main, false);
  assert.equal(result.manifest.changes.preload, true);
  assert.deepEqual(
    result.manifest.artifacts.map((artifact) => artifact.kind),
    ["file", "executable", "file"],
  );
  assert.deepEqual(
    result.manifest.artifacts.map((artifact) => artifact.relativePath),
    [
      "app.asar",
      path.join("bin", "app-server"),
      path.join("default-config", "compact", "COMPACT.md"),
    ],
  );
  assert.equal(
    result.manifest.artifacts.some((artifact) =>
      artifact.relativePath.includes(
        path.join("Contents", "MacOS", "MorpheusLauncher"),
      ),
    ),
    false,
  );
  assert.equal(
    fs.existsSync(
      path.join(preparedRoot, "Contents", "MacOS", "MorpheusLauncher"),
    ),
    false,
  );
  assert.equal(
    fs.readFileSync(path.join(preparedRoot, "resources/app.asar"), "utf8"),
    "asar",
  );
  assert.equal(
    fs.readFileSync(
      path.join(preparedRoot, "resources/bin/app-server"),
      "utf8",
    ),
    "server",
  );
  assert.deepEqual(spawnCalls[0].args.slice(0, 3), [
    "dlx",
    "@electron/asar",
    "pack",
  ]);
  assert.equal(
    path.basename(path.dirname(spawnCalls[0].args[3])).startsWith(
      "morpheus-prepared-building-",
    ),
    true,
  );
  assert.equal(
    path.basename(path.dirname(path.dirname(spawnCalls[0].args[4]))).startsWith(
      "morpheus-prepared-building-",
    ),
    true,
  );
  assert.equal(spawnCalls[0].command, "pnpm");
  assert.equal(spawnCalls[0].options.cwd, workspace);
  assert.equal(spawnCalls[0].options.env, commandEnv);
  for (const artifact of result.manifest.artifacts) {
    assert.equal(
      artifact.sha256,
      sha256(
        fs.readFileSync(
          path.join(preparedRoot, "resources", artifact.relativePath),
        ),
      ),
    );
  }
  assert.equal(
    JSON.parse(fs.readFileSync(path.join(preparedRoot, "manifest.json"))).buildId,
    result.buildId,
  );
});

test("Electron shell manifest includes nested cjs dependencies and excludes tests", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "shell-manifest-test-"));
  write(path.join(root, "electron/main.cjs"), "main");
  write(path.join(root, "electron/preload.cjs"), "preload");
  write(path.join(root, "electron/lsp/client.cjs"), "client");
  write(path.join(root, "electron/lsp/client.test.cjs"), "test");
  write(path.join(root, "electron/Info.plist"), "plist");

  assert.deepEqual(listElectronShellSourceRelativePaths(root), [
    path.join("electron", "lsp", "client.cjs"),
    path.join("electron", "main.cjs"),
    path.join("electron", "preload.cjs"),
  ]);
});

test("Electron shell changes include nested additions, deletions and raw app.asar files", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "shell-update-test-"));
  const sourceAppDir = path.join(root, "source");
  const resourcesPath = path.join(root, "bundle/Contents/Resources");
  write(path.join(sourceAppDir, "electron/main.cjs"), "same");
  write(path.join(sourceAppDir, "electron/preload.cjs"), "same");
  write(path.join(sourceAppDir, "electron/newDependency.cjs"), "new");
  write(path.join(resourcesPath, "app.asar/electron/main.cjs"), "same");
  write(path.join(resourcesPath, "app.asar/electron/preload.cjs"), "same");
  write(path.join(resourcesPath, "app.asar/electron/removed.cjs"), "old");

  const update = resolveElectronShellUpdate({ resourcesPath, sourceAppDir });

  assert.equal(update.changed, true);
  assert.deepEqual(update.changedPaths, [
    path.join("electron", "newDependency.cjs"),
    path.join("electron", "removed.cjs"),
  ]);
  assert.deepEqual(update.missingInstalledPaths, [
    path.join("electron", "newDependency.cjs"),
  ]);
  assert.deepEqual(update.missingSourcePaths, [
    path.join("electron", "removed.cjs"),
  ]);
});

test("installed shell enumeration reads the app.asar virtual filesystem root", () => {
  const calls = [];
  const resourcesPath = "/Applications/Morpheus.app/Contents/Resources";
  const entries = new Map([
    [
      path.join(resourcesPath, "app.asar", "electron"),
      [
        directoryEntry("nested", "directory"),
        directoryEntry("main.cjs", "file"),
        directoryEntry("main.test.cjs", "file"),
      ],
    ],
    [
      path.join(resourcesPath, "app.asar", "electron", "nested"),
      [directoryEntry("client.cjs", "file")],
    ],
  ]);

  assert.deepEqual(
    listInstalledElectronShellRelativePaths(resourcesPath, {
      readdirSync: (directoryPath) => {
        calls.push(directoryPath);
        return entries.get(directoryPath) ?? [];
      },
    }),
    [
      path.join("electron", "main.cjs"),
      path.join("electron", "nested", "client.cjs"),
    ],
  );
  assert.equal(
    calls[0],
    path.join(resourcesPath, "app.asar", "electron"),
  );
});

test("plan requires full activation for any Electron main-process dependency change", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "shell-plan-test-"));
  const workspace = path.join(root, "repo");
  const resourcesPath = path.join(root, "Morpheus.app/Contents/Resources");
  write(
    path.join(workspace, "apps/root-worker-prototype/electron/main.cjs"),
    "main",
  );
  write(
    path.join(
      workspace,
      "apps/root-worker-prototype/electron/runtimeLauncher.cjs",
    ),
    "new launcher adapter",
  );
  write(path.join(resourcesPath, "app.asar/electron/main.cjs"), "main");

  const plan = resolveInstalledArtifactUpdatePlan({
    env: { ROOT_WORKER_WORKSPACE: workspace },
    platform: "darwin",
    resourcesPath,
    isPackaged: true,
  });

  assert.equal(plan.requiresFullRelaunch, true);
  assert.deepEqual(plan.runtimeUpdate.electronShell.changedPaths, [
    path.join("electron", "runtimeLauncher.cjs"),
  ]);
});

test("plan keeps hot activation when every Electron shell dependency is unchanged", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "shell-hot-plan-test-"));
  const workspace = path.join(root, "repo");
  const sourceAppDir = path.join(workspace, "apps/root-worker-prototype");
  const resourcesPath = path.join(root, "Morpheus.app/Contents/Resources");
  for (const relativePath of [
    "electron/main.cjs",
    "electron/preload.cjs",
    "electron/lsp/client.cjs",
  ]) {
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
  });

  assert.equal(plan.requiresFullRelaunch, false);
  assert.deepEqual(plan.runtimeUpdate.electronShell.changedPaths, []);
});

test("missing prepared inputs fail before invoking the artifact packer", () => {
  for (const missing of ["frontendDistPath", "appServerBinaryPath", "compact"]) {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "missing-input-test-"));
    const workspace = path.join(root, "repo");
    const sourceAppDir = path.join(workspace, "apps/root-worker-prototype");
    const plan = {
      appBundlePath: "/Applications/Morpheus.app",
      appServerBinaryPath: path.join(workspace, "target/release/app-server"),
      defaultCompactPromptSourcePath: path.join(workspace, "compact.md"),
      frontendDistPath: path.join(sourceAppDir, "dist"),
      sourceAppDir,
      workspace,
    };
    write(path.join(sourceAppDir, "electron/main.cjs"), "main");
    write(path.join(plan.frontendDistPath, "index.html"), "renderer");
    write(plan.appServerBinaryPath, "server");
    write(plan.defaultCompactPromptSourcePath, "compact");
    const missingPath =
      missing === "compact"
        ? plan.defaultCompactPromptSourcePath
        : plan[missing];
    fs.rmSync(missingPath, { force: true, recursive: true });
    let spawned = false;

    assert.throws(
      () =>
        prepareInstalledArtifacts(plan, {
          sourceCommit: "commit",
          spawnSync: () => {
            spawned = true;
            return { status: 0 };
          },
        }),
      /Missing|Expected/,
    );
    assert.equal(spawned, false);
  }
});

test("prepared artifact failure removes the incomplete candidate root", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "prepared-cleanup-test-"));
  const workspace = path.join(root, "repo");
  const sourceAppDir = path.join(workspace, "apps/root-worker-prototype");
  const preparedArtifactsRoot = path.join(root, "prepared-artifacts");
  const preparedRoot = path.join(
    preparedArtifactsRoot,
    "morpheus-prepared-runtime-failure",
  );
  write(path.join(sourceAppDir, "dist/index.html"), "renderer");
  write(path.join(sourceAppDir, "electron/main.cjs"), "main");
  write(path.join(sourceAppDir, "package.json"), "{}");
  write(path.join(workspace, "target/release/app-server"), "server");
  write(path.join(workspace, "compact.md"), "compact");

  assert.throws(
    () =>
      prepareInstalledArtifacts(
        {
          appBundlePath: "/Applications/Morpheus.app",
          appServerBinaryPath: path.join(
            workspace,
            "target/release/app-server",
          ),
          defaultCompactPromptSourcePath: path.join(workspace, "compact.md"),
          frontendDistPath: path.join(sourceAppDir, "dist"),
          sourceAppDir,
          workspace,
        },
        {
          preparedArtifactsRoot,
          preparedRoot,
          sourceCommit: "commit",
          spawnSync: () => ({ status: 1, stderr: "pack failed" }),
        },
      ),
    /pack failed/,
  );
  assert.equal(fs.existsSync(preparedRoot), false);
});

test("startup cleanup removes producer-owned full success and crash residue", () => {
  for (const residue of ["full-success", "host-crash"]) {
    const root = fs.mkdtempSync(
      path.join(os.tmpdir(), `prepared-${residue}-test-`),
    );
    const launcherHome = path.join(root, "runtime-launcher");
    const preparedArtifactsRoot = path.join(
      launcherHome,
      "producer-artifacts",
    );
    const candidate = createOwnedPreparedArtifact(
      preparedArtifactsRoot,
      `morpheus-prepared-runtime-${residue}`,
      residue,
    );
    const canonicalCandidate = fs.realpathSync(candidate);

    const result = cleanupOwnedPreparedArtifactsWithLauncher({
      appBundlePath: "/Applications/Morpheus.app",
      env: { MORPHEUS_RUNTIME_LAUNCHER_HOME: launcherHome },
      runtimeLauncher: {
        supported: true,
        status: () => ({
          ok: true,
          result: { transaction: null },
        }),
      },
    });

    assert.deepEqual(result.removed, [canonicalCandidate]);
    assert.equal(fs.existsSync(candidate), false);
  }
});

test("startup cleanup preserves the root referenced by an active Prepared transaction", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "prepared-active-test-"));
  const launcherHome = path.join(root, "runtime-launcher");
  const preparedArtifactsRoot = path.join(
    launcherHome,
    "producer-artifacts",
  );
  const active = createOwnedPreparedArtifact(
    preparedArtifactsRoot,
    "morpheus-prepared-runtime-active",
    "active",
  );
  const stale = createOwnedPreparedArtifact(
    preparedArtifactsRoot,
    "morpheus-prepared-runtime-stale",
    "stale",
  );
  const canonicalActive = fs.realpathSync(active);
  const canonicalStale = fs.realpathSync(stale);

  const result = cleanupOwnedPreparedArtifactsWithLauncher({
    appBundlePath: "/Applications/Morpheus.app",
    env: { MORPHEUS_RUNTIME_LAUNCHER_HOME: launcherHome },
    runtimeLauncher: {
      supported: true,
      status: () => ({
        ok: true,
        result: {
          transaction: {
            phase: "prepared",
            request: { preparedRoot: active },
          },
        },
      }),
    },
  });

  assert.deepEqual(result.preserved, [canonicalActive]);
  assert.deepEqual(result.removed, [canonicalStale]);
  assert.equal(fs.existsSync(active), true);
  assert.equal(fs.existsSync(stale), false);
});

test("producer lease closes the publish-to-launcher handoff race across hosts", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "prepared-lease-test-"));
  const launcherHome = path.join(root, "runtime-launcher");
  const preparedArtifactsRoot = path.join(
    launcherHome,
    "producer-artifacts",
  );
  const now = Date.parse("2026-09-08T00:00:00.000Z");
  const candidate = createOwnedPreparedArtifact(
    preparedArtifactsRoot,
    "morpheus-prepared-runtime-leased",
    "leased",
    {
      state: "published",
      producerLeaseExpiresAt: new Date(now + 60_000).toISOString(),
    },
  );
  const canonicalCandidate = fs.realpathSync(candidate);
  const runtimeLauncher = {
    supported: true,
    status: () => ({ ok: true, result: { transaction: null } }),
  };

  const whilePublished = cleanupOwnedPreparedArtifactsWithLauncher({
    appBundlePath: "/Applications/Morpheus.app",
    env: { MORPHEUS_RUNTIME_LAUNCHER_HOME: launcherHome },
    now,
    runtimeLauncher,
  });
  assert.deepEqual(whilePublished.preserved, [canonicalCandidate]);
  assert.equal(fs.existsSync(candidate), true);

  releaseOwnedPreparedArtifactLease(candidate, {
    now: now + 1,
    ownerCapability: readPreparedArtifactOwnerCapability(candidate),
    preparedArtifactsRoot,
  });
  let handoffStatusCalls = 0;
  runtimeLauncher.status = () => {
    handoffStatusCalls += 1;
    return {
      ok: true,
      result: {
        transaction:
          handoffStatusCalls === 1
            ? null
            : {
                phase: "Prepared",
                request: { preparedRoot: candidate },
              },
      },
    };
  };
  const afterHandoff = cleanupOwnedPreparedArtifactsWithLauncher({
    appBundlePath: "/Applications/Morpheus.app",
    env: { MORPHEUS_RUNTIME_LAUNCHER_HOME: launcherHome },
    now: now + 2,
    runtimeLauncher,
  });
  assert.deepEqual(afterHandoff.preserved, [canonicalCandidate]);
  assert.equal(handoffStatusCalls, 2);
  assert.equal(fs.existsSync(candidate), true);

  runtimeLauncher.status = () => ({
    ok: true,
    result: { transaction: null },
  });
  const afterCommit = cleanupOwnedPreparedArtifactsWithLauncher({
    appBundlePath: "/Applications/Morpheus.app",
    env: { MORPHEUS_RUNTIME_LAUNCHER_HOME: launcherHome },
    now: now + 3,
    runtimeLauncher,
  });
  assert.deepEqual(afterCommit.removed, [canonicalCandidate]);
  assert.equal(fs.existsSync(candidate), false);
});

test("producer lease is reclaimed after its host crashes", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "prepared-expired-test-"));
  const preparedArtifactsRoot = path.join(root, "producer-artifacts");
  const now = Date.parse("2026-09-08T00:00:00.000Z");
  const candidate = createOwnedPreparedArtifact(
    preparedArtifactsRoot,
    "morpheus-prepared-runtime-expired",
    "expired",
    {
      state: "published",
      producerLeaseExpiresAt: new Date(now + 60_000).toISOString(),
    },
  );
  const canonicalCandidate = fs.realpathSync(candidate);

  const result = garbageCollectOwnedPreparedArtifacts({
    isProcessAlive: () => false,
    now,
    preparedArtifactsRoot,
  });

  assert.deepEqual(result.removed, [canonicalCandidate]);
  assert.equal(fs.existsSync(candidate), false);
});

test("prepared artifact handoff requires the exact owner capability tuple", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "prepared-owner-test-"));
  const preparedArtifactsRoot = path.join(root, "producer-artifacts");
  const candidate = createOwnedPreparedArtifact(
    preparedArtifactsRoot,
    "morpheus-prepared-runtime-owned",
    "owned-transaction",
    {
      state: "published",
      producerLeaseExpiresAt: "2999-01-01T00:00:00.000Z",
    },
  );
  const ownerCapability = readPreparedArtifactOwnerCapability(candidate);

  for (const mismatch of [
    { ...ownerCapability, ownerToken: crypto.randomUUID() },
    { ...ownerCapability, transactionId: "wrong-transaction" },
    { ...ownerCapability, producerPid: ownerCapability.producerPid + 1 },
  ]) {
    assert.throws(
      () =>
        releaseOwnedPreparedArtifactLease(candidate, {
          ownerCapability: mismatch,
          preparedArtifactsRoot,
        }),
      /owner capability does not match/,
    );
  }
  assert.throws(
    () =>
      removeOwnedPreparedArtifact(candidate, {
        ownerCapability: {
          ...ownerCapability,
          ownerToken: crypto.randomUUID(),
        },
        preparedArtifactsRoot,
      }),
    /owner capability does not match/,
  );
  assert.equal(fs.existsSync(candidate), true);

  assert.equal(
    releaseOwnedPreparedArtifactLease(candidate, {
      ownerCapability,
      preparedArtifactsRoot,
    }).ok,
    true,
  );
});

test("prepared artifact handoff never follows or truncates preplanted temp leaves", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "handoff-temp-test-"));
  const preparedArtifactsRoot = path.join(root, "producer-artifacts");
  const candidate = createOwnedPreparedArtifact(
    preparedArtifactsRoot,
    "morpheus-prepared-runtime-handoff-temp",
    "handoff-temp",
    {
      state: "published",
      producerLeaseExpiresAt: "2999-01-01T00:00:00.000Z",
    },
  );
  const ownerCapability = readPreparedArtifactOwnerCapability(candidate);
  const temporaryPath = path.join(
    candidate,
    `${PREPARED_ARTIFACT_OWNER_FILE}.handoff-${process.pid}-collision`,
  );
  const outside = path.join(root, "outside.txt");
  write(outside, "outside");
  fs.symlinkSync(outside, temporaryPath);

  assert.throws(
    () =>
      releaseOwnedPreparedArtifactLease(candidate, {
        ownerCapability,
        preparedArtifactsRoot,
        randomUUID: () => "collision",
      }),
    /EEXIST/,
  );
  assert.equal(fs.readFileSync(outside, "utf8"), "outside");
  assert.equal(fs.lstatSync(temporaryPath).isSymbolicLink(), true);

  fs.rmSync(temporaryPath);
  write(temporaryPath, "pid reuse residue");
  assert.throws(
    () =>
      releaseOwnedPreparedArtifactLease(candidate, {
        ownerCapability,
        preparedArtifactsRoot,
        randomUUID: () => "collision",
      }),
    /EEXIST/,
  );
  assert.equal(fs.readFileSync(temporaryPath, "utf8"), "pid reuse residue");
});

test("prepared artifact cleanup rejects symlinks and paths outside its controlled parent", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "prepared-safety-test-"));
  const preparedArtifactsRoot = path.join(root, "producer-artifacts");
  const outside = createOwnedPreparedArtifact(
    root,
    "morpheus-prepared-runtime-outside",
    "outside",
  );
  fs.mkdirSync(preparedArtifactsRoot, { recursive: true });
  const symlink = path.join(
    preparedArtifactsRoot,
    "morpheus-prepared-runtime-symlink",
  );
  fs.symlinkSync(outside, symlink);
  const unmarked = path.join(
    preparedArtifactsRoot,
    "morpheus-prepared-runtime-unmarked",
  );
  fs.mkdirSync(unmarked);
  const preplanted = path.join(
    preparedArtifactsRoot,
    "morpheus-prepared-runtime-preplanted",
  );
  fs.mkdirSync(preplanted);
  write(
    path.join(preplanted, PREPARED_ARTIFACT_OWNER_FILE),
    JSON.stringify({
      schemaVersion: 1,
      transactionId: "forged",
      state: "published",
      producerLeaseExpiresAt: "2999-01-01T00:00:00.000Z",
    }),
  );

  const result = garbageCollectOwnedPreparedArtifacts({
    preparedArtifactsRoot,
  });

  assert.equal(result.removed.length, 0);
  assert.equal(result.rejected.length, 3);
  assert.equal(
    result.rejected.some(({ reason }) => /real directory/.test(reason)),
    true,
  );
  assert.equal(
    result.rejected.some(({ reason }) => /ENOENT/.test(reason)),
    true,
  );
  assert.equal(fs.existsSync(outside), true);
  assert.equal(fs.existsSync(unmarked), true);
  assert.equal(fs.existsSync(preplanted), true);
  assert.throws(
    () =>
      removeOwnedPreparedArtifact(outside, {
        preparedArtifactsRoot,
      }),
    /outside the controlled parent/,
  );
  assert.equal(fs.existsSync(outside), true);
});

test("prepared artifact GC scans past rejected entries while bounding removals", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "prepared-scan-test-"));
  const preparedArtifactsRoot = path.join(root, "producer-artifacts");
  fs.mkdirSync(preparedArtifactsRoot);
  for (let index = 0; index < 300; index += 1) {
    fs.mkdirSync(
      path.join(
        preparedArtifactsRoot,
        `morpheus-prepared-runtime-invalid-${String(index).padStart(3, "0")}`,
      ),
    );
  }
  const valid = createOwnedPreparedArtifact(
    preparedArtifactsRoot,
    "morpheus-prepared-runtime-valid",
    "valid",
  );
  const canonicalValid = fs.realpathSync(valid);

  const results = [];
  for (let pass = 0; pass < 6 && fs.existsSync(valid); pass += 1) {
    results.push(
      garbageCollectOwnedPreparedArtifacts({
        limit: 1,
        preparedArtifactsRoot,
        scanLimit: 64,
      }),
    );
  }

  assert.equal(fs.existsSync(valid), false);
  assert.equal(
    results.some(({ removed }) => removed.includes(canonicalValid)),
    true,
  );
});

test("prepared artifact GC cursor rejects a preplanted symlink", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "prepared-cursor-test-"));
  const preparedArtifactsRoot = path.join(root, "producer-artifacts");
  const outside = path.join(root, "outside.txt");
  fs.mkdirSync(preparedArtifactsRoot);
  write(outside, "do not overwrite");
  fs.symlinkSync(
    outside,
    path.join(preparedArtifactsRoot, ".morpheus-prepared-gc-cursor"),
  );
  createOwnedPreparedArtifact(
    preparedArtifactsRoot,
    "morpheus-prepared-runtime-stale",
    "stale",
  );

  assert.throws(
    () =>
      garbageCollectOwnedPreparedArtifacts({
        preparedArtifactsRoot,
      }),
    /GC cursor must be a regular file/,
  );
  assert.equal(fs.readFileSync(outside, "utf8"), "do not overwrite");
});

test("prepared artifact GC does not remove an EEXIST cursor temp collision", () => {
  const root = fs.mkdtempSync(
    path.join(os.tmpdir(), "prepared-cursor-collision-test-"),
  );
  const preparedArtifactsRoot = path.join(root, "producer-artifacts");
  fs.mkdirSync(preparedArtifactsRoot);
  createOwnedPreparedArtifact(
    preparedArtifactsRoot,
    "morpheus-prepared-runtime-stale",
    "stale",
  );
  const collision = path.join(
    preparedArtifactsRoot,
    `.morpheus-prepared-gc-cursor.${process.pid}.collision`,
  );
  write(collision, "belongs to another process");

  assert.throws(
    () =>
      garbageCollectOwnedPreparedArtifacts({
        preparedArtifactsRoot,
        randomUUID: () => "collision",
      }),
    /EEXIST/,
  );
  assert.equal(
    fs.readFileSync(collision, "utf8"),
    "belongs to another process",
  );
});

test("prepared artifact GC cleans only its own cursor temp after write, close, or rename failure", () => {
  for (const failure of ["write", "close", "rename"]) {
    const root = fs.mkdtempSync(
      path.join(os.tmpdir(), `prepared-cursor-${failure}-test-`),
    );
    const preparedArtifactsRoot = path.join(root, "producer-artifacts");
    fs.mkdirSync(preparedArtifactsRoot);
    createOwnedPreparedArtifact(
      preparedArtifactsRoot,
      "morpheus-prepared-runtime-stale",
      "stale",
    );
    const temporaryPath = path.join(
      preparedArtifactsRoot,
      `.morpheus-prepared-gc-cursor.${process.pid}.failure`,
    );
    const realCloseSync = fs.closeSync;
    let closeCalls = 0;

    assert.throws(
      () =>
        garbageCollectOwnedPreparedArtifacts({
          closeSync: (descriptor) => {
            closeCalls += 1;
            if (failure === "close" && closeCalls === 1) {
              throw new Error("close failed");
            }
            return realCloseSync(descriptor);
          },
          preparedArtifactsRoot,
          randomUUID: () => "failure",
          renameSync: (...args) => {
            if (failure === "rename") {
              throw new Error("rename failed");
            }
            return fs.renameSync(...args);
          },
          writeFileSync: (target, ...args) => {
            if (failure === "write" && typeof target === "number") {
              throw new Error("write failed");
            }
            return fs.writeFileSync(target, ...args);
          },
        }),
      new RegExp(`${failure} failed`),
    );
    assert.equal(fs.existsSync(temporaryPath), false);
  }
});

test("prepared artifact root requires a private configured home", () => {
  assert.throws(
    () => resolvePreparedArtifactsRoot({}),
    /MORPHEUS_HOME or HOME is required/,
  );
});

test("installed artifact worker bundle contains every runtime dependency", async () => {
  const bundlePath = materializeInstalledArtifactWorkerBundle();
  assert.deepEqual(fs.readdirSync(bundlePath).sort(), [
    "environment.cjs",
    "installedArtifactUpdate.cjs",
    "installedArtifactUpdateWorker.cjs",
    "workspace.cjs",
  ]);
  assert.equal(
    await resolveInstalledArtifactUpdatePlanInWorker({
      isPackaged: false,
      platform: "linux",
    }),
    null,
  );
});

test("installed artifact worker round-trip preserves the desktop command environment", async () => {
  const commandEnv = {
    CARGO_TARGET_DIR: "/worker/target",
    PATH: "/opt/homebrew/bin:/usr/bin:/bin",
    ROOT_WORKER_WORKSPACE: "/repo",
  };
  const plan = await resolveInstalledArtifactUpdatePlanInWorker({
    commandEnv,
    env: {
      PATH: "/usr/bin:/bin",
      ROOT_WORKER_WORKSPACE: "/repo",
    },
    isPackaged: true,
    platform: "darwin",
    resourcesPath: "/Applications/Morpheus.app/Contents/Resources",
  });

  assert.deepEqual(plan.commandEnv, commandEnv);
  assert.equal(plan.appServerBinaryPath, "/worker/target/release/app-server");
});

test("installed artifact worker keeps the host event loop responsive", async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "artifact-worker-test-"));
  const workspace = path.join(root, "repo");
  const sourceAppDir = path.join(workspace, "apps/root-worker-prototype");
  const fakeBin = path.join(root, "bin");
  write(path.join(sourceAppDir, "dist/index.html"), "renderer");
  write(path.join(sourceAppDir, "electron/main.cjs"), "main");
  write(path.join(sourceAppDir, "package.json"), "{}");
  write(path.join(workspace, "target/release/app-server"), "server");
  write(path.join(workspace, "compact.md"), "compact");
  writeExecutable(
    path.join(fakeBin, "pnpm"),
    "#!/bin/sh\nsleep 0.2\nfor last_arg do :; done\nmkdir -p \"$(dirname \"$last_arg\")\"\nprintf asar > \"$last_arg\"\n",
  );
  writeExecutable(
    path.join(fakeBin, "git"),
    "#!/bin/sh\nprintf test-commit\n",
  );
  const update = updateInstalledArtifactsInWorker({
    appBundlePath: "/Applications/Morpheus.app",
    appServerBinaryPath: path.join(workspace, "target/release/app-server"),
    defaultCompactPromptSourcePath: path.join(workspace, "compact.md"),
    frontendDistPath: path.join(sourceAppDir, "dist"),
    preparedArtifactsRoot: path.join(root, "producer-artifacts"),
    sourceAppDir,
    workspace,
    commandEnv: {
      ...process.env,
      PATH: `${fakeBin}:${process.env.PATH ?? ""}`,
    },
  });
  let timerFired = false;
  await new Promise((resolve) => {
    setTimeout(() => {
      timerFired = true;
      resolve();
    }, 25);
  });

  assert.equal(timerFired, true);
  assert.equal((await update).ok, true);
});

function write(filePath, contents) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, contents);
}

function writeExecutable(filePath, contents) {
  write(filePath, contents);
  fs.chmodSync(filePath, 0o755);
}

function sha256(contents) {
  return crypto.createHash("sha256").update(contents).digest("hex");
}

function createOwnedPreparedArtifact(
  parent,
  name,
  transactionId,
  markerOverrides = {},
) {
  const candidate = path.join(parent, name);
  fs.mkdirSync(candidate, { recursive: true });
  write(
    path.join(candidate, PREPARED_ARTIFACT_OWNER_FILE),
    JSON.stringify({
      schemaVersion: 1,
      transactionId,
      state: "handedOff",
      ownerToken: crypto.randomUUID(),
      producerPid: process.pid,
      producerLeaseExpiresAt: null,
      ...markerOverrides,
    }),
  );
  return candidate;
}

function readPreparedArtifactOwnerCapability(candidate) {
  const marker = JSON.parse(
    fs.readFileSync(
      path.join(candidate, PREPARED_ARTIFACT_OWNER_FILE),
      "utf8",
    ),
  );
  return {
    ownerToken: marker.ownerToken,
    producerPid: marker.producerPid,
    transactionId: marker.transactionId,
  };
}

function directoryEntry(name, type) {
  return {
    name,
    isDirectory: () => type === "directory",
    isFile: () => type === "file",
  };
}
