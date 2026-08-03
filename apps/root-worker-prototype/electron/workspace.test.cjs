const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");

const { ensureDefaultWorkspace, resolveDefaultWorkspace } = require("./workspace.cjs");

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
