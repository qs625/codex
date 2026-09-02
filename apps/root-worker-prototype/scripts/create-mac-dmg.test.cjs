const test = require("node:test");
const assert = require("node:assert/strict");
const path = require("node:path");

const {
  assertMacPackagingPlatform,
  buildFinderLayoutScriptArgs,
  buildHdiutilAttachArgs,
  buildHdiutilConvertArgs,
  buildHdiutilCreateWritableArgs,
  buildHdiutilDetachArgs,
  buildMacDmgPaths,
  isMountedVolumeOutput,
  shouldUseFinderLayout,
  toAppleScriptString,
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
  assert.equal(
    paths.mountRootDir,
    path.join("/tmp/root worker", "dist-app", "dmg-mount"),
  );
  assert.equal(
    paths.volumePath,
    path.join("/tmp/root worker", "dist-app", "dmg-mount", "Root Worker Prototype"),
  );
  assert.equal(
    paths.tempDmgPath,
    path.join("/tmp/root worker", "dist-app", "Root Worker Prototype-arm64.temp.dmg"),
  );
});

test("buildHdiutilCreateWritableArgs builds a writable image command", () => {
  assert.deepEqual(
    buildHdiutilCreateWritableArgs({
      appName: "Root Worker Prototype",
      stagingDir: "/tmp/dist/dmg-staging",
      tempDmgPath: "/tmp/dist/Root Worker Prototype-arm64.temp.dmg",
    }),
    [
      "create",
      "-volname",
      "Root Worker Prototype",
      "-srcfolder",
      "/tmp/dist/dmg-staging",
      "-ov",
      "-format",
      "UDRW",
      "/tmp/dist/Root Worker Prototype-arm64.temp.dmg",
    ],
  );
});

test("buildHdiutilAttachArgs mounts the writable image under a controlled root", () => {
  assert.deepEqual(
    buildHdiutilAttachArgs({
      mountRootDir: "/tmp/dist/dmg-mount",
      tempDmgPath: "/tmp/dist/Root Worker Prototype-arm64.temp.dmg",
    }),
    [
      "attach",
      "/tmp/dist/Root Worker Prototype-arm64.temp.dmg",
      "-readwrite",
      "-noverify",
      "-noautoopen",
      "-mountroot",
      "/tmp/dist/dmg-mount",
    ],
  );
});

test("buildHdiutilDetachArgs detaches the mounted volume path", () => {
  assert.deepEqual(
    buildHdiutilDetachArgs({
      volumePath: "/tmp/dist/dmg-mount/Root Worker Prototype",
    }),
    [
      "detach",
      "/tmp/dist/dmg-mount/Root Worker Prototype",
      "-force",
    ],
  );
});

test("buildHdiutilConvertArgs builds the final compressed image command", () => {
  assert.deepEqual(
    buildHdiutilConvertArgs({
      dmgPath: "/tmp/dist/Root Worker Prototype-arm64.dmg",
      tempDmgPath: "/tmp/dist/Root Worker Prototype-arm64.temp.dmg",
    }),
    [
      "convert",
      "/tmp/dist/Root Worker Prototype-arm64.temp.dmg",
      "-format",
      "UDZO",
      "-o",
      "/tmp/dist/Root Worker Prototype-arm64.dmg",
    ],
  );
});

test("buildFinderLayoutScriptArgs lays out app and Applications icons", () => {
  const args = buildFinderLayoutScriptArgs({
    appName: "Root Worker Prototype",
    volumePath: "/tmp/root worker/dist-app/dmg-mount/Root Worker Prototype",
  });

  assert.equal(args[0], "-e");
  assert.ok(
    args.includes(
      "set dmgFolder to POSIX file \"/tmp/root worker/dist-app/dmg-mount/Root Worker Prototype/\" as alias",
    ),
  );
  assert.ok(args.includes("set current view of container window of dmgFolder to icon view"));
  assert.ok(args.includes("set icon size of viewOptions to 96"));
  assert.ok(
    args.includes(
      "set position of item \"Root Worker Prototype.app\" of dmgFolder to {150, 165}",
    ),
  );
  assert.ok(
    args.includes(
      "set position of item \"Applications\" of dmgFolder to {390, 165}",
    ),
  );
});

test("buildFinderLayoutScriptArgs requires a mounted volume path", () => {
  assert.throws(
    () => buildFinderLayoutScriptArgs({ appName: "Root Worker Prototype" }),
    /requires a mounted volume path/,
  );
});

test("toAppleScriptString escapes quoted paths", () => {
  assert.equal(
    toAppleScriptString("Root \"Worker\" Prototype"),
    "\"Root \\\"Worker\\\" Prototype\"",
  );
});

test("isMountedVolumeOutput matches mounted volumes with spaces", () => {
  const output = [
    "/dev/disk1s5 on / (apfs, local, read-only, journaled)",
    "/dev/disk4s1 on /tmp/root worker/dist-app/dmg-mount/Root Worker Prototype (hfs, local)",
  ].join("\n");

  assert.equal(
    isMountedVolumeOutput(
      output,
      "/tmp/root worker/dist-app/dmg-mount/Root Worker Prototype",
    ),
    true,
  );
  assert.equal(
    isMountedVolumeOutput(
      output,
      "/tmp/root worker/dist-app/dmg-mount/Other Volume",
    ),
    false,
  );
});

test("isMountedVolumeOutput does not treat stale ordinary directories as mounts", () => {
  const output = "/dev/disk1s5 on / (apfs, local, read-only, journaled)";

  assert.equal(
    isMountedVolumeOutput(
      output,
      "/tmp/root worker/dist-app/dmg-mount/Root Worker Prototype",
    ),
    false,
  );
});

test("assertMacPackagingPlatform rejects non-macOS package runs", () => {
  assert.throws(
    () => assertMacPackagingPlatform("linux"),
    /requires hdiutil and must run on macOS/,
  );
  assert.doesNotThrow(() => assertMacPackagingPlatform("darwin"));
});

test("Finder DMG layout is skipped in CI or when explicitly disabled", () => {
  assert.equal(shouldUseFinderLayout({ env: {} }), true);
  assert.equal(shouldUseFinderLayout({ env: { CI: "true" } }), false);
  assert.equal(
    shouldUseFinderLayout({
      env: { ROOT_WORKER_DMG_SKIP_FINDER_LAYOUT: "1" },
    }),
    false,
  );
  assert.equal(
    shouldUseFinderLayout({
      env: { CI: "false", ROOT_WORKER_DMG_SKIP_FINDER_LAYOUT: "0" },
    }),
    true,
  );
});
