# 相关模式调研

## 代码现状

- `apps/root-worker-prototype` 是 React 19 + Vite + Electron 原型。
- 当前 header 在 `apps/root-worker-prototype/src/components/Panels.tsx` 中把 `getThreadModelLabel(selectedThread)` 与 `getThreadReasoningLabel(selectedThread)` 渲染为静态 chip。
- 样式集中在 `apps/root-worker-prototype/src/styles.css` 的 thread header / `.thread-chip` 区域。
- `Thread` 类型已有 `modelProvider`、`model`、`reasoningEffort` 字段。
- Electron 侧已有 app-server 通用 request 客户端，当前 worktree 也已出现 `codex:listModels` IPC 与 `RunConfigPicker` 初版实现。本设计 handoff 以“基于现有实现补齐状态、文案、可访问性与视觉密度”为准，不要求从零搭建入口。

## 协议现状

- app-server v2 已有 `model/list`。
- `Model` 包含 `displayName`、`description`、`supportedReasoningEfforts`、`defaultReasoningEffort`。
- `ModelListResponse` 返回 `data` 与 `nextCursor`。
- `TurnStartParams.model` 与 `TurnStartParams.effort` 的作用域是当前 turn 及后续 turn；本需求为了避免运行中竞态，UI 表达为“当前 thread 后续消息生效”，运行中禁止应用。

## 产品模式

1. AI chat / coding 工具通常把当前模型显示为紧凑入口，点击后在 popup 或 dropdown 内切换。GitHub Copilot Chat 文档中，用户从 chat panel 的当前 model 入口打开 popup menu 选择模型。

2. 模型选择与 reasoning effort 应形成父子关系。OpenAI reasoning 文档把 effort 作为 reasoning model 的配置项；本项目协议也把 supported/default efforts 放在 Model 上。

3. 运行中切换必须避免误解。用户可以查看可用配置，但不能让正在执行的 turn 被“半途切换”的假象影响；因此禁用应用按钮并给文本提示。

4. 紧凑工具 UI 比设置页更合适。此操作是当前 thread 内的运行参数调整，不应把用户带离对话上下文。

## 设计原则

- 一眼读当前值：入口先给 `model · effort` 摘要。
- 先解释作用域：popover 顶部固定显示“更改后仅影响当前 thread 的后续消息”。
- 先选 model，再选 reasoning：reasoning 区域随当前候选 model 更新。
- 错误不破坏当前 thread：列表错误只影响候选项加载，不清空当前配置。
- 运行中只读：允许打开查看，但不允许应用。

## 来源

- GitHub Copilot Chat 模型切换文档：https://docs.github.com/en/copilot/using-github-copilot/ai-models/changing-the-ai-model-for-copilot-chat
- OpenAI reasoning guide：https://developers.openai.com/api/docs/guides/reasoning
- Claude Code model 配置文档：https://code.claude.com/docs/en/model-config
