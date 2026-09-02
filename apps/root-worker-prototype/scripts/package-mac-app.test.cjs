const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  buildElectronPackagerArgs,
  buildMacAppPackagePlan,
  listSourceSnapshotFiles,
  shouldIncludeSourceSnapshotPath,
  stageSourceSnapshot,
} = require("./package-mac-app.cjs");

test("mac app package plan stages release app-server and compact seed resources", () => {
  const cwd = "/repo/apps/root-worker-prototype";

  assert.deepEqual(buildMacAppPackagePlan({ cwd }), {
    appBundlePath:
      "/repo/apps/root-worker-prototype/dist-app/Root Worker Prototype-darwin-arm64/Root Worker Prototype.app",
    appServerBinaryPath: "/repo/codex-rs/target/release/app-server",
    binResourceDir: "/repo/apps/root-worker-prototype/dist-package-resources/bin",
    codexRsCargoManifestPath: "/repo/codex-rs/Cargo.toml",
    defaultCompactPromptResourcePath:
      "/repo/apps/root-worker-prototype/dist-package-resources/default-config/compact/COMPACT.md",
    defaultCompactPromptSourcePath:
      "/repo/codex-rs/thread-service/templates/compact/prompt.md",
    defaultConfigResourceDir:
      "/repo/apps/root-worker-prototype/dist-package-resources/default-config",
    distDir: "/repo/apps/root-worker-prototype/dist-app",
    resourceStagingDir:
      "/repo/apps/root-worker-prototype/dist-package-resources",
    repoRoot: "/repo",
    sourceResourceDir:
      "/repo/apps/root-worker-prototype/dist-package-resources/source",
  });
});

test("electron packager args include app-server, default config, and source extra resources", () => {
  const cwd = "/repo/apps/root-worker-prototype";
  const args = buildElectronPackagerArgs({
    cwd,
    binResourceDir: path.join(cwd, "dist-package-resources/bin"),
    defaultConfigResourceDir: path.join(
      cwd,
      "dist-package-resources/default-config",
    ),
    sourceResourceDir: path.join(cwd, "dist-package-resources/source"),
  });

  assert.ok(args.includes("--extra-resource=dist-package-resources/bin"));
  assert.ok(
    args.includes("--extra-resource=dist-package-resources/default-config"),
  );
  assert.ok(args.includes("--extra-resource=dist-package-resources/source"));
  assert.ok(args.includes("--ignore=^/dist-package-resources($|/)"));
  assert.ok(args.includes("--no-prune"));
});

test("source snapshot excludes heavy build outputs and local secrets", () => {
  for (const excluded of [
    ".git/config",
    "codex-rs/target/debug/app-server",
    "apps/root-worker-prototype/node_modules/react/index.js",
    "apps/root-worker-prototype/dist-app/app/index.html",
    "apps/root-worker-prototype/dist-package-resources/source/README.md",
    "apps/android-companion/local.properties",
    ".env",
    ".env.local",
    ".envrc",
    "../outside",
    "certs/developer.pem",
    "secrets/service.secret.json",
  ]) {
    assert.equal(shouldIncludeSourceSnapshotPath(excluded), false, excluded);
  }

  for (const included of [
    "Cargo.toml",
    "codex-rs/thread-service/src/lib.rs",
    "apps/root-worker-prototype/src/App.tsx",
    ".codex/agents/project-pm.agent.md",
  ]) {
    assert.equal(shouldIncludeSourceSnapshotPath(included), true, included);
  }
});

test("source snapshot file list uses tracked files and denylist", () => {
  assert.deepEqual(
    listSourceSnapshotFiles("/repo", {
      trackedFiles: [
        "apps/root-worker-prototype/src/App.tsx",
        "codex-rs/target/release/app-server",
        "apps/android-companion/local.properties",
        "Cargo.toml",
      ],
    }),
    ["Cargo.toml", "apps/root-worker-prototype/src/App.tsx"],
  );
});

test("source snapshot file list enumerates tracked files through rtk git", () => {
  const calls = [];
  const files = listSourceSnapshotFiles("/repo", {
    spawnSync: (command, args, options) => {
      calls.push({ command, args, cwd: options.cwd, encoding: options.encoding });
      return {
        status: 0,
        stdout: Buffer.from("Cargo.toml\0apps/root-worker-prototype/src/App.tsx\0"),
      };
    },
  });

  assert.deepEqual(calls, [
    {
      command: "rtk",
      args: ["git", "ls-files", "-z"],
      cwd: "/repo",
      encoding: "buffer",
    },
  ]);
  assert.deepEqual(files, [
    "Cargo.toml",
    "apps/root-worker-prototype/src/App.tsx",
  ]);
});

test("stageSourceSnapshot copies only included tracked files", () => {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "rw-source-stage-"));
  const repoRoot = path.join(tempRoot, "repo");
  const sourceResourceDir = path.join(tempRoot, "resources/source");
  fs.mkdirSync(path.join(repoRoot, "apps/root-worker-prototype/src"), {
    recursive: true,
  });
  fs.mkdirSync(path.join(repoRoot, "apps/android-companion"), {
    recursive: true,
  });
  fs.writeFileSync(path.join(repoRoot, "Cargo.toml"), "[workspace]\n");
  fs.writeFileSync(
    path.join(repoRoot, "apps/root-worker-prototype/src/App.tsx"),
    "export {}\n",
  );
  fs.writeFileSync(
    path.join(repoRoot, "apps/android-companion/local.properties"),
    "sdk.dir=/private\n",
  );

  stageSourceSnapshot(
    {
      repoRoot,
      sourceResourceDir,
    },
    {
      trackedFiles: [
        "Cargo.toml",
        "apps/root-worker-prototype/src/App.tsx",
        "apps/android-companion/local.properties",
      ],
    },
  );

  assert.equal(
    fs.readFileSync(path.join(sourceResourceDir, "Cargo.toml"), "utf8"),
    "[workspace]\n",
  );
  assert.equal(
    fs.existsSync(
      path.join(sourceResourceDir, "apps/android-companion/local.properties"),
    ),
    false,
  );

  fs.rmSync(tempRoot, { recursive: true, force: true });
});

test("mac app Info.plist declares permission usage descriptions", () => {
  const plist = fs.readFileSync(
    path.join(__dirname, "..", "electron", "Info.plist"),
    "utf8",
  );

  assert.match(plist, /<key>NSMicrophoneUsageDescription<\/key>/);
  assert.match(plist, /<key>NSScreenCaptureUsageDescription<\/key>/);
  assert.match(plist, /<key>NSAppleEventsUsageDescription<\/key>/);
});
