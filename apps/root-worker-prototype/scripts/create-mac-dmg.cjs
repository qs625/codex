const fs = require("node:fs/promises");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const APP_NAME = "Root Worker Prototype";
const APP_PLATFORM_DIR = "Root Worker Prototype-darwin-arm64";
const DMG_FILE_NAME = "Root Worker Prototype-arm64.dmg";
const DIST_DIR_NAME = "dist-app";

function buildMacDmgPaths({
  appName = APP_NAME,
  appPlatformDir = APP_PLATFORM_DIR,
  cwd = process.cwd(),
  distDirName = DIST_DIR_NAME,
  dmgFileName = DMG_FILE_NAME,
} = {}) {
  const distDir = path.join(cwd, distDirName);
  return {
    appBundlePath: path.join(distDir, appPlatformDir, `${appName}.app`),
    dmgPath: path.join(distDir, dmgFileName),
    distDir,
    stagingDir: path.join(distDir, "dmg-staging"),
  };
}

function buildHdiutilCreateArgs({ appName = APP_NAME, dmgPath, stagingDir }) {
  return [
    "create",
    "-volname",
    appName,
    "-srcfolder",
    stagingDir,
    "-ov",
    "-format",
    "UDZO",
    dmgPath,
  ];
}

function assertMacPackagingPlatform(platform = process.platform) {
  if (platform !== "darwin") {
    throw new Error("macOS DMG packaging requires hdiutil and must run on macOS.");
  }
}

async function createMacDmg({
  appName = APP_NAME,
  appPlatformDir = APP_PLATFORM_DIR,
  cwd = process.cwd(),
  distDirName = DIST_DIR_NAME,
  dmgFileName = DMG_FILE_NAME,
  platform = process.platform,
} = {}) {
  assertMacPackagingPlatform(platform);

  const paths = buildMacDmgPaths({
    appName,
    appPlatformDir,
    cwd,
    distDirName,
    dmgFileName,
  });
  await assertDirectoryExists(paths.appBundlePath, "Packaged app bundle");

  await fs.rm(paths.stagingDir, { force: true, recursive: true });
  await fs.rm(paths.dmgPath, { force: true });
  await fs.mkdir(paths.stagingDir, { recursive: true });
  await fs.cp(
    paths.appBundlePath,
    path.join(paths.stagingDir, `${appName}.app`),
    { recursive: true },
  );
  await fs.symlink("/Applications", path.join(paths.stagingDir, "Applications"));

  try {
    runCommand("hdiutil", buildHdiutilCreateArgs({
      appName,
      dmgPath: paths.dmgPath,
      stagingDir: paths.stagingDir,
    }));
  } finally {
    await fs.rm(paths.stagingDir, { force: true, recursive: true });
  }

  await assertFileExists(paths.dmgPath, "DMG artifact");
  console.log(`Created ${paths.dmgPath}`);
  return paths;
}

async function assertDirectoryExists(targetPath, label) {
  let stat;
  try {
    stat = await fs.stat(targetPath);
  } catch (error) {
    throw new Error(`${label} not found at ${targetPath}`, { cause: error });
  }
  if (!stat.isDirectory()) {
    throw new Error(`${label} is not a directory: ${targetPath}`);
  }
}

async function assertFileExists(targetPath, label) {
  let stat;
  try {
    stat = await fs.stat(targetPath);
  } catch (error) {
    throw new Error(`${label} not found at ${targetPath}`, { cause: error });
  }
  if (!stat.isFile()) {
    throw new Error(`${label} is not a file: ${targetPath}`);
  }
}

function runCommand(command, args) {
  const result = spawnSync(command, args, { stdio: "inherit" });
  if (result.error) {
    throw new Error(`Failed to invoke ${command}: ${result.error.message}`, {
      cause: result.error,
    });
  }
  if (result.status !== 0) {
    throw new Error(`${command} failed with exit code ${result.status}`);
  }
}

if (require.main === module) {
  createMacDmg().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}

module.exports = {
  assertMacPackagingPlatform,
  buildHdiutilCreateArgs,
  buildMacDmgPaths,
  createMacDmg,
};
