const test = require("node:test");
const assert = require("node:assert/strict");
const fsSync = require("node:fs");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");

const {
  ensureDefaultWorkspace,
  ensureMorpheusSourceInstructionSync,
  morpheusSourceInstructionPath,
  removeMorpheusSourceInstructionIfManagedSync,
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

test("resolveDefaultWorkspace prefers installed source workspace for packaged app", () => {
  const sourceWorkspace = "/tmp/prototype-home/source_workspace";
  const workspace = resolveDefaultWorkspace(
    {
      MORPHEUS_HOME: "/tmp/prototype-home",
    },
    {
      isPackagedApp: true,
      existsSync: (target) =>
        target ===
          path.join(
            sourceWorkspace,
            "apps",
            "root-worker-prototype",
            "package.json",
          ) || target === path.join(sourceWorkspace, "codex-rs", "Cargo.toml"),
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
      isPackagedApp: true,
      existsSync: (target) =>
        target ===
          path.join(
            "/custom/workspace",
            "apps",
            "root-worker-prototype",
            "package.json",
          ) || target === path.join("/custom/workspace", "codex-rs", "Cargo.toml"),
    },
  );

  assert.equal(workspace, "/custom/workspace");
});

test("packaged startup ignores an invalid explicit source workspace consistently", () => {
  const workspace = resolveDefaultWorkspace(
    {
      MORPHEUS_HOME: "/tmp/prototype-home",
      ROOT_WORKER_WORKSPACE: "/deleted/source-workspace",
    },
    {
      isPackagedApp: true,
      existsSync: () => false,
    },
  );

  assert.equal(workspace, "/tmp/prototype-home/root_workspace");
});

test("packaged startup without source workspace uses an operational workspace without cloning", async () => {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "root-worker-source-workspace-"));
  const morpheusHome = path.join(tempRoot, "morpheus-home");
  const calls = [];

  const workspace = await ensureDefaultWorkspace(
    { MORPHEUS_HOME: morpheusHome },
    {
      isPackagedApp: true,
      spawnSync: (...args) => calls.push(args),
    },
  );

  assert.equal(workspace, path.join(morpheusHome, "root_workspace"));
  assert.deepEqual(calls, []);
  assert.equal(fsSync.statSync(workspace).isDirectory(), true);
  assert.equal(
    fsSync.existsSync(
      morpheusSourceInstructionPath({ MORPHEUS_HOME: morpheusHome }),
    ),
    false,
  );

  await fs.rm(tempRoot, { recursive: true, force: true });
});

test("ensureDefaultWorkspace does not overwrite existing installed source workspace", async () => {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "root-worker-source-existing-"));
  const morpheusHome = path.join(tempRoot, "morpheus-home");
  const workspace = path.join(morpheusHome, "source_workspace");
  createSourceWorkspace(workspace);
  fsSync.writeFileSync(path.join(workspace, "README.md"), "custom readme\n");
  const calls = [];

  const resolved = await ensureDefaultWorkspace(
    { MORPHEUS_HOME: morpheusHome },
    {
      isPackagedApp: true,
      spawnSync: (...args) => {
        calls.push(args);
        return { status: 0, stderr: "" };
      },
    },
  );

  assert.equal(resolved, workspace);
  assert.deepEqual(calls, []);
  assert.equal(
    fsSync.readFileSync(path.join(workspace, "README.md"), "utf8"),
    "custom readme\n",
  );
  assert.match(
    fsSync.readFileSync(
      morpheusSourceInstructionPath({ MORPHEUS_HOME: morpheusHome }),
      "utf8",
    ),
    new RegExp(escapeRegExp(workspace)),
  );

  await fs.rm(tempRoot, { recursive: true, force: true });
});

test("ensureMorpheusSourceInstructionSync updates only managed instruction file", async () => {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "root-worker-instruction-"));
  const morpheusHome = path.join(tempRoot, "morpheus-home");
  const instructionPath = morpheusSourceInstructionPath({
    MORPHEUS_HOME: morpheusHome,
  });

  assert.equal(
    ensureMorpheusSourceInstructionSync(
      { MORPHEUS_HOME: morpheusHome },
      "/workspace/one",
    ),
    true,
  );
  assert.match(fsSync.readFileSync(instructionPath, "utf8"), /\/workspace\/one/);

  assert.equal(
    ensureMorpheusSourceInstructionSync(
      { MORPHEUS_HOME: morpheusHome },
      "/workspace/two",
    ),
    true,
  );
  assert.match(fsSync.readFileSync(instructionPath, "utf8"), /\/workspace\/two/);

  fsSync.writeFileSync(instructionPath, "user managed instructions\n");
  assert.equal(
    ensureMorpheusSourceInstructionSync(
      { MORPHEUS_HOME: morpheusHome },
      "/workspace/three",
    ),
    false,
  );
  assert.equal(
    fsSync.readFileSync(instructionPath, "utf8"),
    "user managed instructions\n",
  );

  await fs.rm(tempRoot, { recursive: true, force: true });
});

test("removes stale managed source instruction without deleting user content", async () => {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "root-worker-instruction-"));
  const env = { MORPHEUS_HOME: path.join(tempRoot, "morpheus-home") };
  const instructionPath = morpheusSourceInstructionPath(env);

  ensureMorpheusSourceInstructionSync(env, "/workspace/one");
  assert.equal(removeMorpheusSourceInstructionIfManagedSync(env), true);
  assert.equal(fsSync.existsSync(instructionPath), false);

  fsSync.mkdirSync(path.dirname(instructionPath), { recursive: true });
  fsSync.writeFileSync(instructionPath, "user managed instructions\n");
  assert.equal(removeMorpheusSourceInstructionIfManagedSync(env), false);
  assert.equal(
    fsSync.readFileSync(instructionPath, "utf8"),
    "user managed instructions\n",
  );

  await fs.rm(tempRoot, { recursive: true, force: true });
});

test("ensureDefaultWorkspace keeps dev default workspace local without clone", async () => {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "root-worker-dev-workspace-"));
  const morpheusHome = path.join(tempRoot, "morpheus-home");
  const calls = [];

  const workspace = await ensureDefaultWorkspace(
    { MORPHEUS_HOME: morpheusHome },
    {
      isPackagedApp: false,
      spawnSync: (...args) => {
        calls.push(args);
        return { status: 0, stderr: "" };
      },
    },
  );

  assert.equal(workspace, path.join(morpheusHome, "root_workspace"));
  assert.deepEqual(calls, []);
  assert.equal(fsSync.statSync(workspace).isDirectory(), true);

  await fs.rm(tempRoot, { recursive: true, force: true });
});

function createSourceWorkspace(workspace) {
  fsSync.mkdirSync(
    path.join(workspace, "apps", "root-worker-prototype"),
    { recursive: true },
  );
  fsSync.writeFileSync(
    path.join(workspace, "apps", "root-worker-prototype", "package.json"),
    "{}\n",
  );
  fsSync.mkdirSync(path.join(workspace, "codex-rs"), { recursive: true });
  fsSync.writeFileSync(path.join(workspace, "codex-rs", "Cargo.toml"), "\n");
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
