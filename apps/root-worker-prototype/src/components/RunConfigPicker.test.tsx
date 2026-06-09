import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { RunConfigPopoverContent } from "./RunConfigPicker";
import type { RunModel } from "../types";

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

function renderPopover(
  overrides: Partial<React.ComponentProps<typeof RunConfigPopoverContent>> = {},
) {
  return renderToStaticMarkup(
    <RunConfigPopoverContent
      canApply
      disabled={false}
      draftModel="gpt-5"
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

test("run config popover shows fallback and disables apply while running", () => {
  const markup = renderPopover({
    canApply: false,
    disabled: true,
    fallbackMessage: "已回退到该模型默认 reasoning",
  });

  assert.match(markup, /已回退到该模型默认 reasoning/);
  assert.match(markup, /当前 turn 正在运行，结束后可应用切换/);
  assert.match(markup, /disabled="">应用/);
});
