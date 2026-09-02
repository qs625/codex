const test = require("node:test");
const assert = require("node:assert/strict");
const fsSync = require("node:fs");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");

const {
  ensureDefaultWorkspace,
  ensureMorpheusSourceInstructionSync,
  INSTALLED_SOURCE_ORIGIN_URL,
  morpheusSourceInstructionPath,
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
  const workspace = resolveDefaultWorkspace(
    {
      MORPHEUS_HOME: "/tmp/prototype-home",
    },
    {
      isPackagedApp: true,
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
    },
  );

  assert.equal(workspace, "/custom/workspace");
});

test("ensureDefaultWorkspace clones installed source workspace when missing", async () => {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "root-worker-source-workspace-"));
  const morpheusHome = path.join(tempRoot, "morpheus-home");
  const calls = [];

  const workspace = await ensureDefaultWorkspace(
    { MORPHEUS_HOME: morpheusHome },
    {
      isPackagedApp: true,
      cloneTempSuffix: "test",
      spawnSync: (command, args, options) => {
        calls.push({ command, args, cwd: options.cwd, encoding: options.encoding });
        fsSync.mkdirSync(args[3], { recursive: true });
        fsSync.mkdirSync(path.join(args[3], ".git"), { recursive: true });
        return { status: 0, stderr: "" };
      },
    },
  );

  assert.equal(workspace, path.join(morpheusHome, "source_workspace"));
  const tempWorkspace = path.join(morpheusHome, ".source_workspace.clone-test");
  assert.deepEqual(calls, [
    {
      command: "rtk",
      args: ["git", "clone", INSTALLED_SOURCE_ORIGIN_URL, tempWorkspace],
      cwd: morpheusHome,
      encoding: "utf8",
    },
  ]);
  assert.equal(fsSync.statSync(path.join(workspace, ".git")).isDirectory(), true);
  const instruction = fsSync.readFileSync(
    morpheusSourceInstructionPath({ MORPHEUS_HOME: morpheusHome }),
    "utf8",
  );
  assert.match(instruction, new RegExp(escapeRegExp(workspace)));
  assert.match(instruction, /request_runtime_restart/);
  assert.match(instruction, /complete the relevant tests first/);
  assert.match(instruction, /update the runnable app artifacts/);

  await fs.rm(tempRoot, { recursive: true, force: true });
});

test("ensureDefaultWorkspace does not overwrite existing installed source workspace", async () => {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "root-worker-source-existing-"));
  const morpheusHome = path.join(tempRoot, "morpheus-home");
  const workspace = path.join(morpheusHome, "source_workspace");
  fsSync.mkdirSync(workspace, { recursive: true });
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

test("ensureDefaultWorkspace surfaces clone failure and removes partial workspace", async () => {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "root-worker-clone-fail-"));
  const morpheusHome = path.join(tempRoot, "morpheus-home");
  const workspace = path.join(morpheusHome, "source_workspace");

  await assert.rejects(
    ensureDefaultWorkspace(
      { MORPHEUS_HOME: morpheusHome },
      {
        isPackagedApp: true,
        cloneTempSuffix: "failed",
        spawnSync: (_command, args) => {
          fsSync.mkdirSync(args[3], { recursive: true });
          fsSync.writeFileSync(path.join(args[3], "partial"), "partial\n");
          return { status: 128, stderr: "clone failed\n" };
        },
      },
    ),
    /rtk git clone git@github\.com:qs625\/codex\.git .* exited with 128: clone failed/,
  );
  assert.equal(fsSync.existsSync(workspace), false);

  await fs.rm(tempRoot, { recursive: true, force: true });
});

test("ensureDefaultWorkspace clone failure does not remove externally created workspace", async () => {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "root-worker-clone-race-"));
  const morpheusHome = path.join(tempRoot, "morpheus-home");
  const workspace = path.join(morpheusHome, "source_workspace");
  const tempWorkspace = path.join(morpheusHome, ".source_workspace.clone-race");

  await assert.rejects(
    ensureDefaultWorkspace(
      { MORPHEUS_HOME: morpheusHome },
      {
        isPackagedApp: true,
        cloneTempSuffix: "race",
        spawnSync: (_command, args) => {
          fsSync.mkdirSync(args[3], { recursive: true });
          fsSync.writeFileSync(path.join(args[3], "partial"), "partial\n");
          fsSync.mkdirSync(workspace, { recursive: true });
          fsSync.writeFileSync(path.join(workspace, "user-file"), "keep\n");
          return { status: 128, stderr: "clone failed\n" };
        },
      },
    ),
    /rtk git clone git@github\.com:qs625\/codex\.git .* exited with 128: clone failed/,
  );
  assert.equal(fsSync.existsSync(tempWorkspace), false);
  assert.equal(fsSync.readFileSync(path.join(workspace, "user-file"), "utf8"), "keep\n");

  await fs.rm(tempRoot, { recursive: true, force: true });
});

test("ensureDefaultWorkspace successful clone does not replace raced workspace", async () => {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "root-worker-clone-install-race-"));
  const morpheusHome = path.join(tempRoot, "morpheus-home");
  const workspace = path.join(morpheusHome, "source_workspace");
  const tempWorkspace = path.join(morpheusHome, ".source_workspace.clone-race-install");

  const resolved = await ensureDefaultWorkspace(
    { MORPHEUS_HOME: morpheusHome },
    {
      isPackagedApp: true,
      cloneTempSuffix: "race-install",
      spawnSync: (_command, args) => {
        fsSync.mkdirSync(args[3], { recursive: true });
        fsSync.mkdirSync(path.join(args[3], ".git"), { recursive: true });
        return { status: 0, stderr: "" };
      },
      mkdirSync: (target, options) => {
        if (target === workspace && !fsSync.existsSync(workspace)) {
          fsSync.mkdirSync(workspace, { recursive: true });
          fsSync.writeFileSync(path.join(workspace, "user-file"), "keep\n");
          const error = new Error("already exists");
          error.code = "EEXIST";
          throw error;
        }
        fsSync.mkdirSync(target, options);
      },
    },
  );

  assert.equal(resolved, workspace);
  assert.equal(fsSync.existsSync(tempWorkspace), false);
  assert.equal(fsSync.readFileSync(path.join(workspace, "user-file"), "utf8"), "keep\n");

  await fs.rm(tempRoot, { recursive: true, force: true });
});

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
