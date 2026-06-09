const test = require("node:test");
const assert = require("node:assert/strict");

const {
  buildTurnStartParams,
  mergeRuntimeOverride,
  resolveRuntimeForResume,
} = require("./turnStart.cjs");

test("buildTurnStartParams sends model and effort overrides", () => {
  const params = buildTurnStartParams(
    {
      threadId: "thread-1",
      model: "gpt-5.5",
      modelProvider: "modelhub-gpt",
      effort: "high",
    },
    [{ type: "text", text: "hello" }],
  );

  assert.deepEqual(params, {
    threadId: "thread-1",
    input: [{ type: "text", text: "hello" }],
    model: "gpt-5.5",
    modelProvider: "modelhub-gpt",
    effort: "high",
  });
  assert.equal(Object.hasOwn(params, "reasoningEffort"), false);
});

test("mergeRuntimeOverride preserves existing values when payload omits overrides", () => {
  assert.deepEqual(
    mergeRuntimeOverride(
      {
        model: "gpt-5",
        modelProvider: "openai",
        reasoningEffort: "medium",
      },
      {
        threadId: "thread-1",
      },
    ),
    {
      model: "gpt-5",
      modelProvider: "openai",
      reasoningEffort: "medium",
    },
  );
});

test("resolveRuntimeForResume preserves local overrides before the next turn starts", () => {
  assert.deepEqual(
    resolveRuntimeForResume(
      {
        model: "gpt-5.5",
        modelProvider: "modelhub-gpt",
        reasoningEffort: "high",
        localOverride: true,
      },
      {
        model: "gpt-5",
        modelProvider: "openai",
        reasoningEffort: "medium",
      },
    ),
    {
      model: "gpt-5.5",
      modelProvider: "modelhub-gpt",
      reasoningEffort: "high",
      localOverride: true,
    },
  );
});
