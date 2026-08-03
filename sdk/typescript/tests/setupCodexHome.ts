import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";

import { afterEach, beforeEach } from "@jest/globals";

const originalMorpheusHome = process.env.MORPHEUS_HOME;
let currentMorpheusHome: string | undefined;

beforeEach(async () => {
  currentMorpheusHome = await fs.mkdtemp(
    path.join(os.tmpdir(), "codex-sdk-test-"),
  );
  process.env.MORPHEUS_HOME = currentMorpheusHome;
});

afterEach(async () => {
  const morpheusHomeToDelete = currentMorpheusHome;
  currentMorpheusHome = undefined;

  if (originalMorpheusHome === undefined) {
    delete process.env.MORPHEUS_HOME;
  } else {
    process.env.MORPHEUS_HOME = originalMorpheusHome;
  }

  if (morpheusHomeToDelete) {
    await fs.rm(morpheusHomeToDelete, { recursive: true, force: true });
  }
});
