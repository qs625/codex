const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  SELF_PROJECT_ID,
  ensureSelfProjectSync,
  selfProjectPath,
} = require("./selfProject.cjs");

test("ensureSelfProjectSync creates hidden self project for workspace", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "root-worker-self-project-"));
  const morpheusHome = path.join(dir, "home");
  const workspace = path.join(dir, "source");

  const project = ensureSelfProjectSync(
    { MORPHEUS_HOME: morpheusHome },
    workspace,
  );

  assert.deepEqual(project, {
    id: SELF_PROJECT_ID,
    path: SELF_PROJECT_ID,
    workspace,
    hidden: true,
    system: true,
    managedBy: "morpheus",
  });
  assert.deepEqual(
    JSON.parse(fs.readFileSync(selfProjectPath({ MORPHEUS_HOME: morpheusHome }), "utf8")),
    project,
  );

  fs.rmSync(dir, { recursive: true, force: true });
});

test("ensureSelfProjectSync updates workspace while preserving non-target fields", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "root-worker-self-project-existing-"));
  const morpheusHome = path.join(dir, "home");
  const projectPath = selfProjectPath({ MORPHEUS_HOME: morpheusHome });
  fs.mkdirSync(path.dirname(projectPath), { recursive: true });
  fs.writeFileSync(
    projectPath,
    JSON.stringify({
      id: SELF_PROJECT_ID,
      path: SELF_PROJECT_ID,
      workspace: "/old/source",
      hidden: true,
      system: true,
      label: "Morpheus",
    }),
  );

  const project = ensureSelfProjectSync(
    { MORPHEUS_HOME: morpheusHome },
    "/new/source",
  );

  assert.equal(project.workspace, "/new/source");
  assert.equal(project.hidden, true);
  assert.equal(project.system, true);
  assert.equal(project.label, "Morpheus");

  fs.rmSync(dir, { recursive: true, force: true });
});
