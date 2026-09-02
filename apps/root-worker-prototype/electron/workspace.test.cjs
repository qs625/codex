const test = require("node:test");
const assert = require("node:assert/strict");
const fsSync = require("node:fs");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");

const {
  ensureDefaultWorkspace,
  findPackagedSourceSnapshotPath,
  resolveDefaultWorkspace,
} = require("./workspace.cjs");

test("resolveDefaultWorkspace defaults under MORPHEUS_HOME root_workspace", () => {
  const workspace = resolveDefaultWorkspace({
    MORPHEUS_HOME: "/tmp/prototype-home",
  });

  assert.equal(workspace, "/tmp/prototype-home/root_workspace");
});

test("ensureDefaultWorkspace creates missing default workspace directory", async () => {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "root-worker-workspace-"));
  const morpheusHome = path.join(tempRoot, "morpheus-home");
  const workspace = path.join(morpheusHome, "root_workspace");

  await ensureDefaultWorkspace({
    MORPHEUS_HOME: morpheusHome,
  });

  const stat = await fs.stat(workspace);
  assert.equal(stat.isDirectory(), true);

  await fs.rm(tempRoot, { recursive: true, force: true });
});

test("resolveDefaultWorkspace ignores CODEX_HOME", () => {
  const workspace = resolveDefaultWorkspace({
    CODEX_HOME: "/tmp/legacy-home",
  });

  assert.notEqual(workspace, "/tmp/legacy-home/root_workspace");
  assert.ok(workspace.endsWith("/.morpheus/root_workspace"));
});

test("resolveDefaultWorkspace prefers packaged source workspace when available", () => {
  const workspace = resolveDefaultWorkspace(
    {
      MORPHEUS_HOME: "/tmp/prototype-home",
    },
    {
      resourcesPath: "/Applications/Root Worker.app/Contents/Resources",
      existsSync: (candidate) =>
        candidate === "/Applications/Root Worker.app/Contents/Resources/source",
      statSync: () => ({ isDirectory: () => true }),
    },
  );

  assert.equal(workspace, "/tmp/prototype-home/source_workspace");
});

test("resolveDefaultWorkspace keeps explicit ROOT_WORKER_WORKSPACE", () => {
  const workspace = resolveDefaultWorkspace(
    {
      MORPHEUS_HOME: "/tmp/prototype-home",
      ROOT_WORKER_WORKSPACE: "/custom/workspace",
    },
    {
      resourcesPath: "/Applications/Root Worker.app/Contents/Resources",
      existsSync: () => true,
      statSync: () => ({ isDirectory: () => true }),
    },
  );

  assert.equal(workspace, "/custom/workspace");
});

test("ensureDefaultWorkspace seeds packaged source when workspace is missing", async () => {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "root-worker-source-workspace-"));
  const resourcesPath = path.join(tempRoot, "resources");
  const sourceSeed = path.join(resourcesPath, "source");
  const morpheusHome = path.join(tempRoot, "morpheus-home");
  fsSync.mkdirSync(path.join(sourceSeed, "apps/root-worker-prototype"), {
    recursive: true,
  });
  fsSync.writeFileSync(path.join(sourceSeed, "README.md"), "seed readme\n");
  fsSync.writeFileSync(
    path.join(sourceSeed, "apps/root-worker-prototype/package.json"),
    "{}\n",
  );

  const workspace = await ensureDefaultWorkspace(
    { MORPHEUS_HOME: morpheusHome },
    { resourcesPath },
  );

  assert.equal(workspace, path.join(morpheusHome, "source_workspace"));
  assert.equal(
    fsSync.readFileSync(path.join(workspace, "README.md"), "utf8"),
    "seed readme\n",
  );
  assert.equal(
    fsSync.readFileSync(
      path.join(workspace, "apps/root-worker-prototype/package.json"),
      "utf8",
    ),
    "{}\n",
  );

  await fs.rm(tempRoot, { recursive: true, force: true });
});

test("ensureDefaultWorkspace does not overwrite existing packaged source workspace", async () => {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "root-worker-source-existing-"));
  const resourcesPath = path.join(tempRoot, "resources");
  const sourceSeed = path.join(resourcesPath, "source");
  const morpheusHome = path.join(tempRoot, "morpheus-home");
  const workspace = path.join(morpheusHome, "source_workspace");
  fsSync.mkdirSync(sourceSeed, { recursive: true });
  fsSync.mkdirSync(workspace, { recursive: true });
  fsSync.writeFileSync(path.join(sourceSeed, "README.md"), "seed readme\n");
  fsSync.writeFileSync(path.join(workspace, "README.md"), "custom readme\n");

  const resolved = await ensureDefaultWorkspace(
    { MORPHEUS_HOME: morpheusHome },
    { resourcesPath },
  );

  assert.equal(resolved, workspace);
  assert.equal(
    fsSync.readFileSync(path.join(workspace, "README.md"), "utf8"),
    "custom readme\n",
  );

  await fs.rm(tempRoot, { recursive: true, force: true });
});

test("findPackagedSourceSnapshotPath ignores non-directory resources", () => {
  assert.equal(
    findPackagedSourceSnapshotPath({
      resourcesPath: "/resources",
      existsSync: () => true,
      statSync: () => ({ isDirectory: () => false }),
    }),
    null,
  );
});
