const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const {
  buildElectronPackagerArgs,
  buildMacAppPackagePlan,
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
  });
});

test("electron packager args include app-server and default config resources", () => {
  const cwd = "/repo/apps/root-worker-prototype";
  const args = buildElectronPackagerArgs({
    cwd,
    binResourceDir: path.join(cwd, "dist-package-resources/bin"),
    defaultConfigResourceDir: path.join(
      cwd,
      "dist-package-resources/default-config",
    ),
  });

  assert.ok(args.includes("--extra-resource=dist-package-resources/bin"));
  assert.ok(
    args.includes("--extra-resource=dist-package-resources/default-config"),
  );
  assert.equal(
    args.some((arg) => arg === "--extra-resource=dist-package-resources/source"),
    false,
  );
  assert.ok(args.includes("--ignore=^/dist-package-resources($|/)"));
  assert.ok(args.includes("--no-prune"));
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
