# Live Thread Cache Policy 设计 Brief

## 产品目标

修正 root-worker prototype 在 live thread 模式下的线程展示策略：当 thread 已经进入本地 live cache 后，用户切换 thread 不应触发 `thread/read` 或 snapshot/history rebuild 来覆盖已接收的 live `ThreadItem`，避免已显示的 `childCompletion`、subagent 通知、event-command 等 typed display item 被 destructive/non-destructive merge 破坏。

## 目标用户

- 主要用户：使用 root-worker prototype 调试多 agent 协作、子任务完成通知和 live thread 展示的开发者。
- 使用频率：高频切换 thread、观察 live item 增量状态。
- 设备与平台：桌面端 Electron/root-worker prototype。
- 专业程度：熟悉 Codex thread、live event、subagent 和 app-server v2 payload。

## 范围

本次范围是 UX/UE 行为策略确认，不涉及视觉布局重做。

包含：

- thread 首次打开、已加载后再次切换、subscribe/resume、turn lifecycle、item lifecycle 的展示策略。
- `ThreadItem` 与 snapshot/history 的权威边界。
- 开发 handoff 中的状态保护规则。

不包含：

- 新页面、新控件、新视觉样式。
- root-worker prototype 视觉重排。
- app-server v2 API 新增字段设计。

## 约束

- root-worker prototype 展示 app-server v2 thread/live 内容时，只能消费 typed `ThreadItem` / v2 payload。
- 不得从 `agentMessage.text`、`eventDrivenTool.text`、raw `ResponseItem`、marker 文本或 inter-agent JSON envelope 反解 display item。
- live 模式下，已进入本地 live cache 的 thread 在切换展示时只能使用持续接收的 live `ThreadItem`。
- `thread/read` 仅用于 cold start、缺失本地 thread 或显式恢复路径。

## Baseline 与原型说明

本次是非视觉改造的 UX 策略确认，不改变页面结构、组件外观或交互控件形态，因此未获取 root-worker 当前 UI baseline screenshot，也未生成 bitmap/mockup 原型。设计交付以文本状态机、事件权威关系和开发 handoff 为准。

如后续把该策略转化为可见控件、调试面板、状态标识或错误反馈样式，需要按 root-worker prototype 规则补充 baseline screenshot，并在 `assets/` 下维护截图和视觉稿。

## 验收标准

- 首次打开一个本地缺失的 thread 时，允许 `thread/read` 初始化 turns/items。
- 已 loaded 的 thread 再次切换回来时，不触发 `thread/read`，也不用 snapshot/history rebuild merge 当前可见 items。
- `subscribe` / `resume` 只建立 live 订阅与 metadata，不覆盖 turns/items。
- `turn/started` 和 `turn/completed` 只更新 lifecycle metadata，不用 snapshot items 覆盖 live item。
- `item/started` 和 `item/completed` 是可见内容的权威来源。
- 子任务完成通知和 `childCompletion` 在切换 thread 后仍稳定保留。

