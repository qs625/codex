const test = require("node:test");
const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const {
  listElectronShellSourceRelativePaths,
  prepareInstalledArtifacts,
  resolveElectronShellUpdate,
  resolveInstalledArtifactUpdatePlan,
} = require("./installedArtifactUpdate.cjs");

function temporaryDirectory() {
  return fs.mkdtempSync(path.join(os.tmpdir(), "installed-update-test-"));
}

function write(filePath, contents) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, contents);
}

function sha256(filePath) {
  return crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
}

function packRealAsar(sourcePath, archivePath) {
  const result = spawnSync(
    "pnpm",
    ["dlx", "@electron/asar", "pack", sourcePath, archivePath],
    { encoding: "utf8", stdio: "pipe" },
  );
  if (result.error) {
    throw result.error;
  }
  assert.equal(result.status, 0, result.stderr);
}

function preparedPlan(root, electronShell = { changed: false, changedPaths: [] }) {
  const sourceAppDir = path.join(root, "workspace/apps/root-worker-prototype");
  const frontendDistPath = path.join(sourceAppDir, "dist");
  const appServerBinaryPath = path.join(root, "workspace/codex-rs/target/release/app-server");
  const defaultCompactPromptSourcePath = path.join(
    root,
    "workspace/codex-rs/thread-service/templates/compact/prompt.md",
  );
  write(path.join(sourceAppDir, "package.json"), '{"name":"test"}');
  write(path.join(sourceAppDir, "electron/main.cjs"), "main");
  write(path.join(sourceAppDir, "electron/preload.cjs"), "preload");
  write(path.join(frontendDistPath, "index.html"), "<main>ready</main>");
  write(appServerBinaryPath, "app-server");
  write(defaultCompactPromptSourcePath, "compact prompt");
  return {
    appBundlePath: "/Applications/Root Worker Prototype.app",
    appServerBinaryPath,
    commandEnv: { PATH: "/test/bin" },
    defaultCompactPromptSourcePath,
    frontendDistPath,
    runtimeUpdate: { electronShell },
    sourceAppDir,
    workspace: path.join(root, "workspace"),
  };
}

test("installed update plan exists only for a packaged macOS app", () => {
  const workspace = "/Users/example/.morpheus/source_workspace";
  const plan = resolveInstalledArtifactUpdatePlan({
    commandEnv: { CARGO_TARGET_DIR: "/shared/target" },
    env: { ROOT_WORKER_WORKSPACE: workspace },
    isPackaged: true,
    platform: "darwin",
    resourcesPath: "/Applications/Root Worker Prototype.app/Contents/Resources",
    spawnSync(command, args) {
      assert.equal(command, "pnpm");
      assert.deepEqual(args.slice(0, 3), [
        "dlx",
        "@electron/asar",
        "extract",
      ]);
      fs.mkdirSync(args[4], { recursive: true });
      return { status: 0, stdout: "", stderr: "" };
    },
  });

  assert.equal(plan.workspace, workspace);
  assert.equal(plan.appBundlePath, "/Applications/Root Worker Prototype.app");
  assert.equal(plan.appServerBinaryPath, "/shared/target/release/app-server");
  assert.equal(
    resolveInstalledArtifactUpdatePlan({
      isPackaged: false,
      platform: "darwin",
    }),
    null,
  );
  assert.equal(
    resolveInstalledArtifactUpdatePlan({
      isPackaged: true,
      platform: "linux",
    }),
    null,
  );
});

test("electron shell comparison reads a real app.asar and detects file hashes and presence", () => {
  const root = temporaryDirectory();
  try {
    const sourceAppDir = path.join(root, "source");
    const resourcesPath = path.join(root, "installed");
    const installedAppDir = path.join(root, "installed-app");
    write(path.join(sourceAppDir, "electron/main.cjs"), "new main");
    write(path.join(sourceAppDir, "electron/preload.cjs"), "same preload");
    write(path.join(sourceAppDir, "electron/new.cjs"), "new file");
    write(path.join(sourceAppDir, "electron/main.test.cjs"), "ignored");
    write(path.join(installedAppDir, "electron/main.cjs"), "old main");
    write(path.join(installedAppDir, "electron/preload.cjs"), "same preload");
    write(path.join(installedAppDir, "electron/deleted.cjs"), "old file");
    fs.mkdirSync(resourcesPath, { recursive: true });
    packRealAsar(installedAppDir, path.join(resourcesPath, "app.asar"));

    assert.deepEqual(listElectronShellSourceRelativePaths(sourceAppDir), [
      "electron/main.cjs",
      "electron/new.cjs",
      "electron/preload.cjs",
    ]);
    assert.deepEqual(
      resolveElectronShellUpdate({ resourcesPath, sourceAppDir }),
      {
        category: "electronShell",
        changed: true,
        changedPaths: [
          "electron/deleted.cjs",
          "electron/main.cjs",
          "electron/new.cjs",
        ],
      },
    );
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("electron shell comparison keeps hot mode eligible for an unchanged real app.asar", () => {
  const root = temporaryDirectory();
  try {
    const sourceAppDir = path.join(root, "source");
    const resourcesPath = path.join(root, "installed");
    const installedAppDir = path.join(root, "installed-app");
    for (const [relativePath, contents] of [
      ["electron/main.cjs", "same main"],
      ["electron/preload.cjs", "same preload"],
    ]) {
      write(path.join(sourceAppDir, relativePath), contents);
      write(path.join(installedAppDir, relativePath), contents);
    }
    fs.mkdirSync(resourcesPath, { recursive: true });
    packRealAsar(installedAppDir, path.join(resourcesPath, "app.asar"));

    assert.deepEqual(
      resolveElectronShellUpdate({ resourcesPath, sourceAppDir }),
      {
        category: "electronShell",
        changed: false,
        changedPaths: [],
      },
    );
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("candidate contains only controlled resources plus a matching manifest", () => {
  const root = temporaryDirectory();
  const candidateParent = path.join(root, "candidates");
  fs.mkdirSync(candidateParent);
  const calls = [];
  try {
    const plan = preparedPlan(root, {
      changed: true,
      changedPaths: ["electron/preload.cjs"],
    });
    const result = prepareInstalledArtifacts(plan, {
      buildId: "build-1",
      candidateParent,
      sourceCommit: "abc123",
      transactionId: "tx-1",
      spawnSync(command, args, options) {
        calls.push({ command, args, cwd: options.cwd });
        if (command === "pnpm") {
          assert.deepEqual(args.slice(0, 3), ["dlx", "@electron/asar", "pack"]);
          write(args[4], "packed app");
        } else {
          assert.equal(command, "codesign");
          assert.deepEqual(args.slice(0, 3), ["--force", "--sign", "-"]);
          fs.appendFileSync(args[3], "-signed");
        }
        return { status: 0, stdout: "", stderr: "" };
      },
    });

    assert.equal(result.ok, true);
    assert.deepEqual(result.changes, { main: false, preload: true });
    assert.deepEqual(calls.map(({ command }) => command), [
      "pnpm",
      "codesign",
    ]);
    const resources = path.join(result.preparedRoot, "resources");
    const files = [
      "app.asar",
      path.join("bin", "app-server"),
      path.join("default-config", "compact", "COMPACT.md"),
    ];
    assert.deepEqual(
      result.manifest.artifacts.map(({ relativePath }) => relativePath),
      files,
    );
    assert.equal(result.manifest.entrypoint, "app.asar");
    for (const artifact of result.manifest.artifacts) {
      assert.equal(
        artifact.sha256,
        sha256(path.join(resources, artifact.relativePath)),
      );
    }
    assert.equal(
      fs.readFileSync(path.join(resources, "bin/app-server"), "utf8"),
      "app-server-signed",
    );
    assert.equal(fs.existsSync(path.join(result.preparedRoot, "app-source")), false);
    assert.equal(
      fs.existsSync(path.join(resources, "MorpheusLauncher")),
      false,
    );
    assert.equal(
      fs.existsSync(path.join(resources, "Root Worker Runtime")),
      false,
    );
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("candidate preparation failure removes the producer-owned directory", () => {
  const root = temporaryDirectory();
  const candidateParent = path.join(root, "candidates");
  fs.mkdirSync(candidateParent);
  try {
    const plan = preparedPlan(root);
    assert.throws(
      () =>
        prepareInstalledArtifacts(plan, {
          candidateParent,
          sourceCommit: "abc123",
          spawnSync: () => ({
            status: 1,
            stdout: "",
            stderr: "asar failed",
          }),
        }),
      /asar failed/,
    );
    assert.deepEqual(fs.readdirSync(candidateParent), []);
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("candidate codesign failure removes the producer-owned directory", () => {
  const root = temporaryDirectory();
  const candidateParent = path.join(root, "candidates");
  fs.mkdirSync(candidateParent);
  try {
    const plan = preparedPlan(root);
    assert.throws(
      () =>
        prepareInstalledArtifacts(plan, {
          candidateParent,
          sourceCommit: "abc123",
          spawnSync(command, args) {
            if (command === "pnpm") {
              write(args[4], "packed app");
              return { status: 0, stdout: "", stderr: "" };
            }
            return { status: 1, stdout: "", stderr: "sign failed" };
          },
        }),
      /sign failed/,
    );
    assert.deepEqual(fs.readdirSync(candidateParent), []);
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("identical signed candidate inputs produce a stable build hash", () => {
  const root = temporaryDirectory();
  const candidateParent = path.join(root, "candidates");
  fs.mkdirSync(candidateParent);
  try {
    const plan = preparedPlan(root);
    const prepare = () =>
      prepareInstalledArtifacts(plan, {
        candidateParent,
        sourceCommit: "abc123",
        spawnSync(command, args) {
          if (command === "pnpm") {
            write(args[4], "packed app");
          } else {
            fs.appendFileSync(args[3], "-signed");
          }
          return { status: 0, stdout: "", stderr: "" };
        },
      });
    const first = prepare();
    const second = prepare();
    assert.equal(first.buildId, second.buildId);
    assert.deepEqual(first.manifest.artifacts, second.manifest.artifacts);
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("missing existing build outputs fail before invoking packaging commands", () => {
  const root = temporaryDirectory();
  try {
    const plan = preparedPlan(root);
    fs.rmSync(plan.appServerBinaryPath);
    let invoked = false;
    assert.throws(
      () =>
        prepareInstalledArtifacts(plan, {
          spawnSync: () => {
            invoked = true;
            return { status: 0 };
          },
        }),
      /Missing release app-server/,
    );
    assert.equal(invoked, false);
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
});
