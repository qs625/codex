const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");

const { ensureDefaultWorkspace, resolveDefaultWorkspace } = require("./workspace.cjs");

test("resolveDefaultWorkspace defaults under CODEX_HOME root_workspace", () => {
  const workspace = resolveDefaultWorkspace({
    CODEX_HOME: "/tmp/prototype-home",
  });

  assert.equal(workspace, "/tmp/prototype-home/root_workspace");
});

test("ensureDefaultWorkspace creates missing default workspace directory", async () => {
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "root-worker-workspace-"));
  const codexHome = path.join(tempRoot, "codex-home");
  const workspace = path.join(codexHome, "root_workspace");

  await ensureDefaultWorkspace({
    CODEX_HOME: codexHome,
  });

  const stat = await fs.stat(workspace);
  assert.equal(stat.isDirectory(), true);

  await fs.rm(tempRoot, { recursive: true, force: true });
});
