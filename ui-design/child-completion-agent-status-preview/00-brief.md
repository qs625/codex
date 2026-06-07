# Child Completion / Agent Status 列表摘要优化 Brief

## 产品目标

root-worker prototype 的 conversation 列表需要展示 child completion / Agent Status item，但不能因为完整 completion 内容过长而把单个 item 撑得过高。用户在主线程里应能快速判断哪个 child agent 完成、完成状态是什么、是否有错误，以及是否需要进入详情查看完整内容。

## 目标用户

- 角色：使用 my-codex multi-agent 工作流的开发者和调试者。
- 使用频率：高频浏览 conversation，偶尔追踪某个 subagent 的完整结果。
- 设备：以桌面端为主，包含窄宽度终端或 prototype 面板。
- 专业程度：熟悉 agent path、thread、completion、status 等概念。

## 范围

- 涉及：conversation 列表中的 `collabAgentStatusUpdate` / child completion 展示规则。
- 涉及：Agent Status item 的摘要字段、截断策略、详情区域保留策略、响应式和可访问性要求。
- 不涉及：新增协议 item 类型、重做 multi-agent 信息架构、修改后端 completion 发送语义、改动普通 tool call 的产品分类。
- 不涉及：代码实现和视觉稿产出。本交付只给开发 handoff。

## 约束

- 保持中文设计文档；代码层 display text 可继续使用现有英文 UI 文案。
- 遵循现有 TUI 轻量 history cell 模式：标题行 + 缩进详情行。
- 沿用现有 truncation 思路：prompt/error/response 预览按 grapheme 限制，不按 byte 截断。
- 列表摘要必须稳定占高，不能随 completion 原文长度线性增长。
- 完整 completion 必须可在 details / expanded 区域读取、复制或继续调试。
- 终态 status item 仍应保留为独立历史项，不因语义内容相同而合并掉。

## 验收标准

- conversation 列表中单个 Agent Status item 默认不超过 3 行，推荐目标高度为 1 到 2 行。
- `Completed` 带长 message 时，列表只展示压缩后的短摘要，不展示完整 completion。
- `Errored` 带长 message 时，列表优先展示错误摘要，且不会占用超过 `Completed` 更多的默认高度。
- 详情或展开区域能查看完整 `status.message`、agent path、sender/recipient thread 信息和原始状态。
- 窄宽度下不会出现横向撑开、文本重叠或状态标签被长文本吞掉。
- snapshot 覆盖短 completion、长 completion、错误、无 message、窄宽度截断。
