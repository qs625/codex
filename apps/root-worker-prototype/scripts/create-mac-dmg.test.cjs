const test = require("node:test");
const assert = require("node:assert/strict");
const path = require("node:path");

const {
  assertMacPackagingPlatform,
  buildHdiutilCreateArgs,
  buildMacDmgPaths,
} = require("./create-mac-dmg.cjs");

test("buildMacDmgPaths keeps app names with spaces intact", () => {
  const paths = buildMacDmgPaths({ cwd: "/tmp/root worker" });

  assert.equal(
    paths.appBundlePath,
    path.join(
      "/tmp/root worker",
      "dist-app",
      "Root Worker Prototype-darwin-arm64",
      "Root Worker Prototype.app",
    ),
  );
  assert.equal(
    paths.dmgPath,
    path.join("/tmp/root worker", "dist-app", "Root Worker Prototype-arm64.dmg"),
  );
  assert.equal(
    paths.stagingDir,
    path.join("/tmp/root worker", "dist-app", "dmg-staging"),
  );
});

test("buildHdiutilCreateArgs builds a compressed image command", () => {
  assert.deepEqual(
    buildHdiutilCreateArgs({
      appName: "Root Worker Prototype",
      dmgPath: "/tmp/dist/Root Worker Prototype-arm64.dmg",
      stagingDir: "/tmp/dist/dmg-staging",
    }),
    [
      "create",
      "-volname",
      "Root Worker Prototype",
      "-srcfolder",
      "/tmp/dist/dmg-staging",
      "-ov",
      "-format",
      "UDZO",
      "/tmp/dist/Root Worker Prototype-arm64.dmg",
    ],
  );
});

test("assertMacPackagingPlatform rejects non-macOS package runs", () => {
  assert.throws(
    () => assertMacPackagingPlatform("linux"),
    /requires hdiutil and must run on macOS/,
  );
  assert.doesNotThrow(() => assertMacPackagingPlatform("darwin"));
});
