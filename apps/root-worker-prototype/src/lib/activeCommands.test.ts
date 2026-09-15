import test from "node:test";
import assert from "node:assert/strict";

import type { Thread } from "../types";
import {
  countActiveCommandItemsWithProcess,
  findActiveCommandItem,
  isRunningCommandExecutionStatus,
  selectActiveCommandItems,
  selectRunningActiveCommandItems,
  type CommandExecutionItem,
} from "./activeCommands";

function command(
  id: string,
  status: string,
  processId?: string | null,
): CommandExecutionItem {
  return {
    type: "commandExecution",
    id,
    command: `run ${id}`,
    cwd: "/repo",
    processId,
    source: "agent",
    status,
    initialWaitMs: null,
    notifyOn: null,
    aggregatedOutput: null,
    exitCode: null,
    durationMs: null,
  };
}

test("active command selectors keep latest command item by id", () => {
  const first = command("cmd-1", "running", "pid-1");
  const replacement = command("cmd-1", "inProgress", "pid-2");
  const second = command("cmd-2", "completed", "pid-3");
  const thread = {
    activeCommandItems: [
      first,
      {
        type: "agentMessage",
        id: "message-1",
        text: "ignored",
        phase: null,
        memoryCitation: null,
      },
      replacement,
      second,
    ],
  } satisfies Pick<Thread, "activeCommandItems">;

  assert.deepEqual(selectActiveCommandItems(thread), [replacement, second]);
  assert.equal(findActiveCommandItem(thread, "cmd-1"), replacement);
  assert.equal(findActiveCommandItem(thread, "missing"), null);
});

test("running active command selector normalizes legacy status spellings", () => {
  assert.equal(isRunningCommandExecutionStatus("running"), true);
  assert.equal(isRunningCommandExecutionStatus("inProgress"), true);
  assert.equal(isRunningCommandExecutionStatus("in_progress"), true);
  assert.equal(isRunningCommandExecutionStatus("in-progress"), true);
  assert.equal(isRunningCommandExecutionStatus("completed"), false);

  const thread = {
    activeCommandItems: [
      command("running", "running", "pid-1"),
      command("in-progress", "in-progress", "pid-2"),
      command("done", "completed", "pid-3"),
    ],
  } satisfies Pick<Thread, "activeCommandItems">;

  assert.deepEqual(
    selectRunningActiveCommandItems(thread).map((item) => item.id),
    ["running", "in-progress"],
  );
});

test("terminal badge selector counts command current-state items with process ids", () => {
  const thread = {
    activeCommandItems: [
      command("live", "running", "pid-1"),
      command("completed", "completed", "pid-2"),
      command("pending", "running", null),
    ],
  } satisfies Pick<Thread, "activeCommandItems">;

  assert.equal(countActiveCommandItemsWithProcess(thread), 2);
  assert.equal(countActiveCommandItemsWithProcess(null), 0);
});
