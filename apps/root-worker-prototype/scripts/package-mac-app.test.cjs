const test = require("node:test");
const assert = require("node:assert/strict");
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
  });
});

test("electron packager args include app-server and default config extra resources", () => {
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
  assert.ok(args.includes("--ignore=^/dist-package-resources($|/)"));
  assert.ok(args.includes("--no-prune"));
});
