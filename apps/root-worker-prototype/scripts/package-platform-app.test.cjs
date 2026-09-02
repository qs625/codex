const test = require("node:test");
const assert = require("node:assert/strict");
const path = require("node:path");

const {
  assertSupportedPackagingPlatform,
  buildArchiveCommand,
  buildElectronPackagerArgs,
  buildPlatformPackagePlan,
  buildRunInvocation,
  escapePowerShellPath,
} = require("./package-platform-app.cjs");

test("linux package plan uses linux app-server resource and tar artifact", () => {
  const cwd = "/repo/apps/root-worker-prototype";
  const plan = buildPlatformPackagePlan({ cwd, platform: "linux" });

  assert.equal(plan.electronPlatform, "linux");
  assert.equal(plan.electronArch, "x64");
  assert.equal(plan.appServerBinaryPath, "/repo/codex-rs/target/release/app-server");
  assert.equal(
    plan.appServerResourcePath,
    "/repo/apps/root-worker-prototype/dist-package-resources/bin/app-server",
  );
  assert.equal(
    plan.artifactPath,
    "/repo/apps/root-worker-prototype/dist-app/Root Worker Prototype-linux-x64.tar.gz",
  );
});

test("windows package plan uses app-server.exe resource and zip artifact", () => {
  const cwd = "C:\\repo\\apps\\root-worker-prototype";
  const plan = buildPlatformPackagePlan({ cwd, platform: "win32" });

  assert.equal(plan.electronPlatform, "win32");
  assert.equal(plan.electronArch, "x64");
  assert.ok(plan.appServerBinaryPath.endsWith("codex-rs/target/release/app-server.exe"));
  assert.ok(
    plan.appServerResourcePath.endsWith(
      "dist-package-resources/bin/app-server.exe",
    ),
  );
  assert.ok(plan.artifactPath.endsWith("Root Worker Prototype-win32-x64.zip"));
});

test("platform packager args include runtime resources without source snapshot", () => {
  const cwd = "/repo/apps/root-worker-prototype";
  const args = buildElectronPackagerArgs({
    cwd,
    electronPlatform: "linux",
    electronArch: "x64",
    binResourceDir: path.join(cwd, "dist-package-resources/bin"),
    defaultConfigResourceDir: path.join(
      cwd,
      "dist-package-resources/default-config",
    ),
  });

  assert.ok(args.includes("--platform=linux"));
  assert.ok(args.includes("--arch=x64"));
  assert.ok(args.includes("--extra-resource=dist-package-resources/bin"));
  assert.ok(
    args.includes("--extra-resource=dist-package-resources/default-config"),
  );
  assert.equal(
    args.some((arg) => arg === "--extra-resource=dist-package-resources/source"),
    false,
  );
});

test("linux archive command builds tar.gz from packaged app directory", () => {
  const plan = buildPlatformPackagePlan({
    cwd: "/repo/apps/root-worker-prototype",
    platform: "linux",
  });

  assert.deepEqual(buildArchiveCommand(plan), {
    command: "tar",
    args: [
      "-czf",
      "/repo/apps/root-worker-prototype/dist-app/Root Worker Prototype-linux-x64.tar.gz",
      "-C",
      "/repo/apps/root-worker-prototype/dist-app",
      "Root Worker Prototype-linux-x64",
    ],
  });
});

test("windows archive command builds zip from packaged app directory", () => {
  const plan = buildPlatformPackagePlan({
    cwd: "/repo/apps/root-worker-prototype",
    platform: "win32",
  });

  const command = buildArchiveCommand(plan);
  assert.equal(command.command, "powershell");
  assert.deepEqual(command.args.slice(0, 2), ["-NoProfile", "-Command"]);
  assert.match(command.args[2], /Compress-Archive/);
  assert.match(command.args[2], /Root Worker Prototype-win32-x64\.zip/);
});

test("platform packaging requires matching native runner", () => {
  assert.doesNotThrow(() => assertSupportedPackagingPlatform("linux", "linux"));
  assert.throws(
    () => assertSupportedPackagingPlatform("linux", "darwin"),
    /linux desktop packaging must run on a linux runner/,
  );
  assert.throws(
    () => assertSupportedPackagingPlatform("freebsd", "freebsd"),
    /Unsupported desktop packaging platform/,
  );
});

test("escapePowerShellPath doubles single quotes", () => {
  assert.equal(escapePowerShellPath("C:\\It's\\App"), "C:\\It''s\\App");
});

test("windows rtk subprocesses run through the shell for cmd shims", () => {
  assert.deepEqual(buildRunInvocation("rtk", ["pnpm", "build"], "win32"), {
    command: "rtk",
    args: ["pnpm", "build"],
    spawnOptions: { shell: true },
  });
  assert.deepEqual(buildRunInvocation("rtk", ["pnpm", "build"], "linux"), {
    command: "rtk",
    args: ["pnpm", "build"],
    spawnOptions: {},
  });
  assert.deepEqual(buildRunInvocation("tar", ["-czf"], "win32"), {
    command: "tar",
    args: ["-czf"],
    spawnOptions: {},
  });
});
