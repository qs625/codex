# 组件拆分与开发 Handoff

## AgentStatusHistoryCell

职责：把 `ThreadItem::CollabAgentStatusUpdate` 渲染为稳定高度的 conversation history cell。

输入数据：

- `id`
- `senderThreadId`
- `senderPath`
- `recipientThreadId`
- `recipientPath`
- `status.path`
- `status.status`
- `status.message`
- agent metadata：nickname、role

默认结构：

- title line：status label + agent label + optional role。
- preview line：message preview，仅在非空时显示。

状态：

- `PendingInit`：显示 `Pending init`。
- `Running`：显示 `Running`。
- `Interrupted`：显示 `Interrupted`，warning 语义。
- `Completed`：显示 `Completed`，成功语义，可带 message preview。
- `Errored`：显示 `Error`，错误语义，必须带错误摘要 fallback。
- `Shutdown`：显示 `Shutdown`。
- `NotFound`：显示 `Not found`，错误语义。

## MessagePreview

职责：把长 completion / error message 压缩为列表摘要。

规则：

- 先 trim。
- 列表摘要中 collapse 连续 whitespace，避免原始换行撑高 item。
- `Completed` preview 建议限制为 160 到 240 graphemes；优先沿用现有 `COLLAB_AGENT_RESPONSE_PREVIEW_GRAPHEMES = 240`。
- `Errored` preview 建议限制为 160 graphemes；沿用现有 `COLLAB_AGENT_ERROR_PREVIEW_GRAPHEMES = 160`。
- 截断顺序必须固定：数据预处理先做 trim 和 whitespace collapse，再按状态类型做 grapheme preview 截断；渲染阶段再按当前视口宽度做单行截断。
- grapheme preview 是宽屏上限，视口单行截断是窄屏保护，两者都需要保留。
- 截断符使用现有 `truncate_text` 规则，保持项目一致。

## DetailsPanel / ExpandedItem

职责：保留完整 completion 和 debug metadata。

内容：

- status
- agent label
- `senderPath`
- `recipientPath`
- `senderThreadId`
- `recipientThreadId`
- `id`
- full `status.message`

行为：

- 完整 message 不做 grapheme 截断。
- 保留原始换行，方便阅读 agent 最终回答。
- 长内容在 details 内滚动，不影响 conversation 列表高度。
- 支持复制完整 message。
- 复用现有 conversation item 的详情/展开键盘入口；若现有 TUI 没有统一入口，开发前需要先确定焦点、打开、返回列表的键盘路径。
- details 中完整 message 必须可被键盘焦点和辅助技术访问，不能只作为 hover tooltip。

## AgentLabel

职责：生成稳定、短、可识别的 agent 名称。

优先级：

1. nickname
2. `status.path` 的最后一段或 sender path 的最后一段
3. role
4. thread id 短格式

长 path 只在 details 完整展示。列表中 path 应 middle truncate，避免右侧状态或 message 被挤掉。

## Snapshot 覆盖建议

需要新增或更新 snapshot：

- completed with short message：默认 2 行。
- completed with very long markdown message：列表仍为 2 行，details 保留全文。
- errored with long message：错误状态未展开可见。
- completed without message：只有标题行。
- narrow width：message 单行截断，无横向撑开。
- very narrow width：状态和 agent label 仍可读，message 可被进一步截断。
- long path without nickname：列表使用短 label 或 middle truncate，details 保留完整 path。
- long error with line breaks：列表 collapse 为单行，details 保留原始换行。
- full message with markdown / bullets / code-like text：列表不被格式撑高，details 可完整阅读。
- duplicate terminal status items：两个终态 item 都保留。

## 实现注意点

- 不要把 `status.message` 直接放进 title line 的无限宽 segment。
- 不要在列表中保留原始换行；原始换行只属于 details。
- 不要为解决高度问题丢弃完整 message。
- 不要让 completed message 的存在改变 status label 的位置。
- 如果现有 `status_summary_spans` 仍承担 wait result 展示，可以新增小的 status item 渲染分支，避免影响 `Wait` 汇总行。
