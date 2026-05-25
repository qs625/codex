const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");

const { adapterForFile } = require("./adapters.cjs");
const { resolveWorkspaceRoot } = require("./workspaceRoots.cjs");

test("rust workspace roots prefer the enclosing cargo workspace", async () => {
  const tempDir = await fs.mkdtemp(path.join(os.tmpdir(), "rw-lsp-workspace-"));
  const workspaceRoot = path.join(tempDir, "repo");
  const crateRoot = path.join(workspaceRoot, "crates", "member");
  const filePath = path.join(crateRoot, "src", "lib.rs");

  await fs.mkdir(path.dirname(filePath), { recursive: true });
  await fs.writeFile(
    path.join(workspaceRoot, "Cargo.toml"),
    ['[workspace]', 'members = ["crates/member"]', ""].join("\n"),
  );
  await fs.writeFile(
    path.join(crateRoot, "Cargo.toml"),
    ['[package]', 'name = "member"', 'version = "0.1.0"', 'edition = "2021"', ""].join("\n"),
  );
  await fs.writeFile(filePath, "pub fn demo() {}\n");

  const adapter = adapterForFile(filePath);
  const result = await resolveWorkspaceRoot(adapter, filePath);

  assert.equal(result.reason, null);
  assert.equal(result.workspaceRoot, workspaceRoot);
});

test("rust workspace roots fall back to the crate root without a workspace", async () => {
  const tempDir = await fs.mkdtemp(path.join(os.tmpdir(), "rw-lsp-crate-"));
  const crateRoot = path.join(tempDir, "standalone");
  const filePath = path.join(crateRoot, "src", "lib.rs");

  await fs.mkdir(path.dirname(filePath), { recursive: true });
  await fs.writeFile(
    path.join(crateRoot, "Cargo.toml"),
    ['[package]', 'name = "standalone"', 'version = "0.1.0"', 'edition = "2021"', ""].join(
      "\n",
    ),
  );
  await fs.writeFile(filePath, "pub fn demo() {}\n");

  const adapter = adapterForFile(filePath);
  const result = await resolveWorkspaceRoot(adapter, filePath);

  assert.equal(result.reason, null);
  assert.equal(result.workspaceRoot, crateRoot);
});
