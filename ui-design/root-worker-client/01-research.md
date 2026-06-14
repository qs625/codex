# Research

## 本次是否调研

未做外部产品调研。

原因：本次是现有 root-worker prototype 的 Command Session 展示修复，范围集中在 typed command cell、RightPanel live index、虚拟列表定位和 notification event 信息层级。设计依据来自当前完整 Electron baseline、现有前端组件结构和 typed `ThreadItem` 约束。

## Baseline 依据

![Command Session baseline](assets/baseline-command-session-2026-06-14.png)

## 相关内部模式

- Conversation 继续作为 canonical timeline。
- RightPanel 继续作为 live/recent activity index。
- typed item id 是跨 conversation 与 RightPanel 的唯一可靠关联键。
- 不从 raw marker、message text 或 JSON envelope 反解 UI 状态。
