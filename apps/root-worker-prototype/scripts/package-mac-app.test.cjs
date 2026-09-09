const test = require("node:test");
const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  assertMacRuntimeLayout,
  buildElectronPackagerArgs,
  buildMacAppPackagePlan,
  finalizeMacRuntimeBundle,
  installMacRuntimeExecutables,
  writeInstalledRuntimeManifest,
} = require("./package-mac-app.cjs");

test("mac package plan separates stable Launcher and Electron Host paths", () => {
  const plan = buildMacAppPackagePlan({
    cwd: "/repo/apps/root-worker-prototype",
  });

  assert.equal(
    plan.launcherBinaryPath,
    "/repo/codex-rs/target/release/MorpheusLauncher",
  );
  assert.equal(
    plan.launcherExecutablePath,
    "/repo/apps/root-worker-prototype/dist-app/Root Worker Prototype-darwin-arm64/Root Worker Prototype.app/Contents/MacOS/MorpheusLauncher",
  );
  assert.equal(
    plan.hostExecutablePath,
    "/repo/apps/root-worker-prototype/dist-app/Root Worker Prototype-darwin-arm64/Root Worker Prototype.app/Contents/MacOS/Root Worker Runtime",
  );
  assert.equal(
    plan.runtimeManifestPath,
    "/repo/apps/root-worker-prototype/dist-app/Root Worker Prototype-darwin-arm64/Root Worker Prototype.app/Contents/Resources/.morpheus-runtime-manifest.json",
  );
});

test("electron packager creates app.asar and stages only controlled resources", () => {
  const cwd = "/repo/apps/root-worker-prototype";
  const args = buildElectronPackagerArgs({
    cwd,
    binResourceDir: path.join(cwd, "dist-package-resources/bin"),
    defaultConfigResourceDir: path.join(
      cwd,
      "dist-package-resources/default-config",
    ),
  });

  assert.ok(args.includes("--asar"));
  assert.ok(args.includes("--extra-resource=dist-package-resources/bin"));
  assert.ok(
    args.includes("--extra-resource=dist-package-resources/default-config"),
  );
  assert.equal(
    args.some((arg) => arg === "--extra-resource=dist-package-resources/source"),
    false,
  );
});

test("packaging renames Electron Host, installs Launcher, and writes initial manifest", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "mac-package-test-"));
  try {
    const cwd = path.join(root, "apps/root-worker-prototype");
    const plan = buildMacAppPackagePlan({ cwd });
    fs.mkdirSync(path.dirname(plan.appBundlePackagerExecutablePath), {
      recursive: true,
    });
    fs.mkdirSync(
      path.join(plan.appBundlePath, "Contents/Resources/bin"),
      { recursive: true },
    );
    fs.mkdirSync(
      path.join(
        plan.appBundlePath,
        "Contents/Resources/default-config/compact",
      ),
      { recursive: true },
    );
    fs.mkdirSync(path.dirname(plan.launcherBinaryPath), { recursive: true });
    fs.writeFileSync(plan.appBundlePackagerExecutablePath, "electron");
    fs.writeFileSync(plan.launcherBinaryPath, "launcher");
    fs.writeFileSync(
      path.join(plan.appBundlePath, "Contents/Resources/app.asar"),
      "asar",
    );
    fs.writeFileSync(
      path.join(plan.appBundlePath, "Contents/Resources/bin/app-server"),
      "server",
    );
    fs.writeFileSync(
      path.join(
        plan.appBundlePath,
        "Contents/Resources/default-config/compact/COMPACT.md",
      ),
      "prompt",
    );

    installMacRuntimeExecutables(plan);
    const commands = [];
    const manifest = finalizeMacRuntimeBundle(plan, {
      sourceCommit: "abc123",
      runCommand(command, args) {
        commands.push({ command, args });
        if (
          command === "codesign" &&
          args.at(-1).endsWith(path.join("Resources", "bin", "app-server"))
        ) {
          fs.appendFileSync(args.at(-1), "-signed");
        }
      },
    });

    assert.equal(fs.readFileSync(plan.hostExecutablePath, "utf8"), "electron");
    assert.equal(
      fs.readFileSync(plan.launcherExecutablePath, "utf8"),
      "launcher",
    );
    assert.equal(manifest.entrypoint, "app.asar");
    assert.equal(
      manifest.artifacts.find(
        ({ relativePath }) => relativePath === path.join("bin", "app-server"),
      ).sha256,
      crypto
        .createHash("sha256")
        .update(
          fs.readFileSync(
            path.join(
              plan.appBundlePath,
              "Contents/Resources/bin/app-server",
            ),
          ),
        )
        .digest("hex"),
    );
    assert.deepEqual(
      commands.map(({ command, args }) => [command, ...args]),
      [
        ["codesign", "--force", "--sign", "-", path.join(plan.appBundlePath, "Contents/Resources/bin/app-server")],
        ["/usr/libexec/PlistBuddy", "-c", "Set :CFBundleExecutable MorpheusLauncher", plan.appBundleInfoPlistPath],
        ["codesign", "--force", "--deep", "--sign", "-", plan.appBundlePath],
        ["codesign", "--verify", "--deep", "--strict", plan.appBundlePath],
      ],
    );
    assert.equal(
      writeInstalledRuntimeManifest(plan, { sourceCommit: "abc123" }).buildId,
      manifest.buildId,
    );
    assert.deepEqual(
      manifest.artifacts.map(({ relativePath }) => relativePath),
      [
        "app.asar",
        path.join("bin", "app-server"),
        path.join("default-config", "compact", "COMPACT.md"),
      ],
    );
    assert.equal(fs.existsSync(plan.appBundlePackagerExecutablePath), false);
    assert.equal(fs.statSync(plan.hostExecutablePath).mode & 0o111, 0o111);
    assert.equal(fs.statSync(plan.launcherExecutablePath).mode & 0o111, 0o111);
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
});

test("mac app Info.plist selects Launcher and retains permission descriptions", () => {
  const plist = fs.readFileSync(
    path.join(__dirname, "..", "electron", "Info.plist"),
    "utf8",
  );

  assert.match(
    plist,
    /<key>CFBundleExecutable<\/key>\s*<string>MorpheusLauncher<\/string>/,
  );
  assert.match(plist, /<key>NSMicrophoneUsageDescription<\/key>/);
  assert.match(plist, /<key>NSScreenCaptureUsageDescription<\/key>/);
  assert.match(plist, /<key>NSAppleEventsUsageDescription<\/key>/);
});
