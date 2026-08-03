const fs = require("node:fs/promises");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const APP_NAME = "Root Worker Prototype";
const APP_PLATFORM_DIR = "Root Worker Prototype-darwin-arm64";
const DMG_FILE_NAME = "Root Worker Prototype-arm64.dmg";
const DIST_DIR_NAME = "dist-app";
const MOUNT_ROOT_DIR_NAME = "dmg-mount";
const STAGING_DIR_NAME = "dmg-staging";
const TEMP_DMG_FILE_NAME = "Root Worker Prototype-arm64.temp.dmg";

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
    mountRootDir: path.join(distDir, MOUNT_ROOT_DIR_NAME),
    stagingDir: path.join(distDir, STAGING_DIR_NAME),
    tempDmgPath: path.join(distDir, TEMP_DMG_FILE_NAME),
    volumePath: path.join(distDir, MOUNT_ROOT_DIR_NAME, appName),
  };
}

function buildHdiutilCreateWritableArgs({
  appName = APP_NAME,
  stagingDir,
  tempDmgPath,
}) {
  return [
    "create",
    "-volname",
    appName,
    "-srcfolder",
    stagingDir,
    "-ov",
    "-format",
    "UDRW",
    tempDmgPath,
  ];
}

function buildHdiutilAttachArgs({ mountRootDir, tempDmgPath }) {
  return [
    "attach",
    tempDmgPath,
    "-readwrite",
    "-noverify",
    "-noautoopen",
    "-mountroot",
    mountRootDir,
  ];
}

function buildHdiutilConvertArgs({ dmgPath, tempDmgPath }) {
  return [
    "convert",
    tempDmgPath,
    "-format",
    "UDZO",
    "-o",
    dmgPath,
  ];
}

function buildHdiutilDetachArgs({ volumePath }) {
  return [
    "detach",
    volumePath,
    "-force",
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

  await cleanupMountedVolume(paths.volumePath);
  await fs.rm(paths.stagingDir, { force: true, recursive: true });
  await fs.rm(paths.mountRootDir, { force: true, recursive: true });
  await fs.rm(paths.dmgPath, { force: true });
  await fs.rm(paths.tempDmgPath, { force: true });
  await fs.mkdir(paths.stagingDir, { recursive: true });
  await fs.mkdir(paths.mountRootDir, { recursive: true });
  await fs.cp(
    paths.appBundlePath,
    path.join(paths.stagingDir, `${appName}.app`),
    { recursive: true },
  );
  await fs.symlink("/Applications", path.join(paths.stagingDir, "Applications"));

  let mounted = false;
  try {
    runCommand("hdiutil", buildHdiutilCreateWritableArgs({
      appName,
      stagingDir: paths.stagingDir,
      tempDmgPath: paths.tempDmgPath,
    }));
    runCommand("hdiutil", buildHdiutilAttachArgs({
      mountRootDir: paths.mountRootDir,
      tempDmgPath: paths.tempDmgPath,
    }));
    mounted = true;

    await assertDirectoryExists(paths.volumePath, "Mounted DMG volume");
    runCommand("osascript", buildFinderLayoutScriptArgs({
      appName,
      volumePath: paths.volumePath,
    }));
    await waitForFileExists(
      path.join(paths.volumePath, ".DS_Store"),
      "Finder layout metadata",
    );

    runCommand("hdiutil", buildHdiutilDetachArgs({ volumePath: paths.volumePath }));
    mounted = false;
    runCommand("hdiutil", buildHdiutilConvertArgs({
      dmgPath: paths.dmgPath,
      tempDmgPath: paths.tempDmgPath,
    }));
  } finally {
    if (mounted) {
      await cleanupMountedVolume(paths.volumePath);
    }
    await fs.rm(paths.stagingDir, { force: true, recursive: true });
    await fs.rm(paths.mountRootDir, { force: true, recursive: true });
    await fs.rm(paths.tempDmgPath, { force: true });
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

async function waitForFileExists(targetPath, label, { attempts = 10, delayMs = 500 } = {}) {
  let lastError;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    try {
      await assertFileExists(targetPath, label);
      return;
    } catch (error) {
      lastError = error;
      await delay(delayMs);
    }
  }
  throw lastError;
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
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

async function cleanupMountedVolume(volumePath) {
  try {
    const stat = await fs.lstat(volumePath);
    if (!stat.isDirectory()) {
      return;
    }
  } catch (error) {
    if (error && error.code === "ENOENT") {
      return;
    }
    throw error;
  }

  if (!isVolumeMounted(volumePath)) {
    return;
  }

  const result = spawnSync("hdiutil", buildHdiutilDetachArgs({ volumePath }), {
    stdio: "ignore",
  });
  if (result.error || result.status !== 0) {
    throw new Error(`Failed to detach existing DMG volume at ${volumePath}`);
  }
}

function isVolumeMounted(volumePath) {
  const result = spawnSync("mount", [], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
  if (result.error) {
    throw new Error(`Failed to inspect mounted volumes: ${result.error.message}`, {
      cause: result.error,
    });
  }
  if (result.status !== 0) {
    throw new Error("Failed to inspect mounted volumes with mount");
  }
  return isMountedVolumeOutput(result.stdout, volumePath);
}

function isMountedVolumeOutput(mountOutput, volumePath) {
  const mountPointMarker = ` on ${volumePath} (`;
  return String(mountOutput)
    .split(/\r?\n/)
    .some((line) => line.includes(mountPointMarker));
}

function buildFinderLayoutScriptArgs({
  appName = APP_NAME,
  volumePath,
} = {}) {
  const appItemName = `${appName}.app`;
  if (!volumePath) {
    throw new Error("Finder layout requires a mounted volume path.");
  }
  return [
    "-e",
    `set dmgFolder to POSIX file ${toAppleScriptString(`${volumePath}/`)} as alias`,
    "-e",
    "tell application \"Finder\"",
    "-e",
    "open dmgFolder",
    "-e",
    "delay 1",
    "-e",
    "set current view of container window of dmgFolder to icon view",
    "-e",
    "set toolbar visible of container window of dmgFolder to false",
    "-e",
    "set statusbar visible of container window of dmgFolder to false",
    "-e",
    "set the bounds of container window of dmgFolder to {200, 120, 720, 430}",
    "-e",
    "set viewOptions to the icon view options of container window of dmgFolder",
    "-e",
    "set arrangement of viewOptions to not arranged",
    "-e",
    "set icon size of viewOptions to 96",
    "-e",
    `set position of item ${toAppleScriptString(appItemName)} of dmgFolder to {150, 165}`,
    "-e",
    "set position of item \"Applications\" of dmgFolder to {390, 165}",
    "-e",
    "update dmgFolder without registering applications",
    "-e",
    "delay 1",
    "-e",
    "close container window of dmgFolder",
    "-e",
    "end tell",
  ];
}

function toAppleScriptString(value) {
  return `"${String(value).replaceAll("\\", "\\\\").replaceAll("\"", "\\\"")}"`;
}

if (require.main === module) {
  createMacDmg().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}

module.exports = {
  assertMacPackagingPlatform,
  buildFinderLayoutScriptArgs,
  buildHdiutilAttachArgs,
  buildHdiutilConvertArgs,
  buildHdiutilCreateWritableArgs,
  buildHdiutilDetachArgs,
  buildMacDmgPaths,
  cleanupMountedVolume,
  createMacDmg,
  isMountedVolumeOutput,
  toAppleScriptString,
  waitForFileExists,
};
