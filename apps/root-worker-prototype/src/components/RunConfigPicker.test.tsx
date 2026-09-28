import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import {
  isRunConfigApplyEnabled,
  RunConfigPicker,
  RunConfigPopoverContent,
} from "./RunConfigPicker";
import type { RunModel, Thread } from "../types";

function makeModel(overrides: Partial<RunModel> = {}): RunModel {
  return {
    id: "gpt-5",
    model: "gpt-5",
    displayName: "GPT-5",
    description: "Balanced model",
    hidden: false,
    supportedReasoningEfforts: [
      { reasoningEffort: "low", description: "" },
      { reasoningEffort: "medium", description: "" },
    ],
    defaultReasoningEffort: "medium",
    isDefault: true,
    ...overrides,
  };
}

function makeThread(overrides: Partial<Thread> = {}): Thread {
  return {
    id: "thread-1",
    sessionId: "session-1",
    forkedFromId: null,
    preview: "",
    ephemeral: false,
    modelProvider: "openai",
    model: null,
    reasoningEffort: null,
    createdAt: 1,
    updatedAt: 1,
    lifecycleStatus: { type: "final", result: { type: "completed" } },
    path: null,
    cwd: "/tmp",
    cliVersion: "test",
    source: "appServer",
    threadSource: null,
    agentNickname: null,
    agentRole: null,
    gitInfo: null,
    name: null,
    skills: [],
    turns: [],
    ...overrides,
  };
}

function renderPopover(
  overrides: Partial<React.ComponentProps<typeof RunConfigPopoverContent>> = {},
) {
  return renderToStaticMarkup(
    <RunConfigPopoverContent
      canApply
      disabled={false}
      draftModel="gpt-5"
      draftModelProvider={null}
      draftReasoningEffort="medium"
      fallbackMessage={null}
      hasChanged
      isLoading={false}
      loadError={null}
      models={[makeModel()]}
      onApply={() => {}}
      onCancel={() => {}}
      onRetry={() => {}}
      onSelectModel={() => {}}
      onSelectReasoningEffort={() => {}}
      supportedEfforts={["low", "medium"]}
      {...overrides}
    />,
  );
}

test("run config popover renders model and reasoning radio groups", () => {
  const markup = renderPopover();

  assert.match(markup, /运行配置/);
  assert.match(markup, /更改后仅影响当前 thread 的后续消息/);
  assert.match(markup, /role="radiogroup"/);
  assert.match(markup, /GPT-5/);
  assert.match(markup, /Balanced model/);
  assert.match(markup, /aria-checked="true"[^>]*>medium/);
});

test("run config trigger renders inherited provider default without catalog guess", () => {
  const markup = renderToStaticMarkup(
    <RunConfigPicker
      disabled={false}
      initialModelsForTest={[
        makeModel({
          model: "gpt-5.6",
          displayName: "GPT-5.6",
          modelProvider: "openai",
          defaultReasoningEffort: "high",
          supportedReasoningEfforts: [
            { reasoningEffort: "medium", description: "" },
            { reasoningEffort: "high", description: "" },
          ],
          isDefault: true,
        }),
      ]}
      onApply={() => {}}
      selectedThread={makeThread()}
    />,
  );

  assert.match(markup, /openai · default/);
  assert.match(
    markup,
    /aria-label="运行配置，当前模型 openai，reasoning default"/,
  );
  assert.doesNotMatch(markup, /GPT-5\.6 · openai · high/);
});

test("run config trigger reports inherited provider default without catalog", () => {
  const markup = renderToStaticMarkup(
    <RunConfigPicker
      disabled={false}
      onApply={() => {}}
      selectedThread={makeThread()}
    />,
  );

  assert.match(markup, /openai · default/);
});

test("run config popover opens above the composer with bounded height", () => {
  const css = readFileSync(new URL("../styles.css", import.meta.url), "utf8");

  assert.match(
    css,
    /\.run-config-popover \{[\s\S]*bottom: calc\(100% \+ 8px\);[\s\S]*max-height: min\(520px, calc\(100vh - 160px\)\);[\s\S]*overflow: auto;/,
  );
  assert.doesNotMatch(
    css,
    /\.run-config-popover \{[\s\S]*top: calc\(100% \+ 8px\);/,
  );
});

test("run config popover marks configured provider models", () => {
  const markup = renderPopover({
    models: [
      makeModel({
        id: "configured:corp:corp-model",
        model: "corp-model",
        displayName: "corp-model",
        description: "当前配置中的模型 · Corp Gateway",
        configured: true,
      }),
    ],
    draftModel: "corp-model",
  });

  assert.match(markup, /corp-model/);
  assert.match(markup, /Configured/);
  assert.match(markup, /当前配置中的模型 · Corp Gateway/);
});

test("run config popover marks current models missing from model list", () => {
  const markup = renderPopover({
    models: [
      makeModel({
        id: "current:thread-model",
        model: "thread-model",
        displayName: "thread-model",
        description: "当前 thread 的模型，未出现在 model/list",
        current: true,
        isDefault: false,
      }),
    ],
    draftModel: "thread-model",
  });

  assert.match(markup, /thread-model/);
  assert.match(markup, /Current/);
  assert.match(markup, /当前 thread 的模型，未出现在 model\/list/);
});

test("run config popover keeps current-only models from being applied", () => {
  const markup = renderPopover({
    canApply: false,
    hasChanged: false,
    models: [
      makeModel({
        id: "current:thread-model",
        model: "thread-model",
        displayName: "thread-model",
        description: "当前 thread 的模型，未出现在 model/list",
        current: true,
        supportedReasoningEfforts: [],
        defaultReasoningEffort: "unknown",
        isDefault: false,
      }),
    ],
    draftModel: "thread-model",
    draftReasoningEffort: null,
    supportedEfforts: [],
  });

  assert.match(markup, /Current/);
  assert.match(markup, /disabled="">应用/);
});

test("run config popover renders recoverable model list errors", () => {
  const markup = renderPopover({
    canApply: false,
    hasChanged: false,
    loadError: "network down",
    models: [],
  });

  assert.match(markup, /模型列表加载失败，当前配置未受影响。network down/);
  assert.match(markup, />重试</);
  assert.match(markup, /disabled="">应用/);
});

test("run config popover renders empty model state", () => {
  const markup = renderPopover({
    canApply: false,
    hasChanged: false,
    models: [],
  });

  assert.match(markup, /暂无可用模型，当前配置未受影响。/);
  assert.match(markup, /disabled="">应用/);
});

test("run config popover allows applying while a turn is running", () => {
  const activeThread = makeThread({
    lifecycleStatus: { type: "active", activeFlags: ["running"] },
  });
  const markup = renderPopover({
    canApply: true,
    disabled: false,
    fallbackMessage: "已回退到该模型默认 reasoning",
  });
  const panelsSource = readFileSync(
    new URL("./Panels.tsx", import.meta.url),
    "utf8",
  );

  assert.match(markup, /已回退到该模型默认 reasoning/);
  assert.doesNotMatch(markup, /当前 turn 正在运行，结束后可应用切换/);
  assert.match(markup, /<button type="button" class="primary">应用<\/button>/);
  assert.equal(
    isRunConfigApplyEnabled({
      disabled: false,
      draftReasoningEffort: "medium",
      selectedModel: makeModel(),
      selectedThread: activeThread,
    }),
    true,
  );
  assert.match(panelsSource, /<RunConfigPicker[\s\S]*disabled=\{isSending\}/);
  assert.doesNotMatch(
    panelsSource,
    /<RunConfigPicker[\s\S]*disabled=\{isSending \|\| activeTurnId != null\}/,
  );
});

test("run config popover still disables apply while sending a message", () => {
  const markup = renderPopover({
    canApply: false,
    disabled: true,
  });

  assert.match(markup, /正在发送消息，稍后可应用切换/);
  assert.match(markup, /disabled="">应用/);
  assert.equal(
    isRunConfigApplyEnabled({
      disabled: true,
      draftReasoningEffort: "medium",
      selectedModel: makeModel(),
      selectedThread: makeThread(),
    }),
    false,
  );
});
