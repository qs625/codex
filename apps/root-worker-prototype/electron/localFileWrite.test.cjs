const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");

const { writeLocalFileTarget } = require("./localFileWrite.cjs");

async function withTempDir(callback) {
  const tempDir = await fs.mkdtemp(path.join(os.tmpdir(), "morpheus-write-"));
  try {
    return await callback(tempDir);
  } finally {
    await fs.rm(tempDir, { recursive: true, force: true });
  }
}

test("writeLocalFileTarget writes utf8 content to local files", async () => {
  await withTempDir(async (tempDir) => {
    const filePath = path.join(tempDir, "notes.txt");
    await fs.writeFile(filePath, "before", "utf8");

    const result = await writeLocalFileTarget(
      "./notes.txt:12:3",
      "after\n",
      tempDir,
    );

    assert.deepEqual(result, {
      ok: true,
      path: filePath,
      byteSize: Buffer.byteLength("after\n", "utf8"),
    });
    assert.equal(await fs.readFile(filePath, "utf8"), "after\n");
  });
});

test("writeLocalFileTarget rejects non-local targets", async () => {
  await assert.rejects(
    () => writeLocalFileTarget("https://example.com/file.txt", "content", "/tmp"),
    /Only local file links can be written/,
  );
});

test("writeLocalFileTarget rejects directories", async () => {
  await withTempDir(async (tempDir) => {
    await assert.rejects(
      () => writeLocalFileTarget(tempDir, "content", tempDir),
      /Only files can be written/,
    );
  });
});

test("writeLocalFileTarget rejects non-string content", async () => {
  await withTempDir(async (tempDir) => {
    const filePath = path.join(tempDir, "notes.txt");
    await fs.writeFile(filePath, "before", "utf8");

    await assert.rejects(
      () => writeLocalFileTarget(filePath, Buffer.from("content"), tempDir),
      /File content must be a string/,
    );
  });
});
