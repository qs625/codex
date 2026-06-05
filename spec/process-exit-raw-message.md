# process_exit raw message 修复设计

## 任务 brief

用户反馈又看到一条 process_exit 的 raw message，期望 process exit 相关事件不要以原始 message 形式出现在客户端对话中，而是通过明确的结构化 item 展示。

成功标准：

- process_exit 订阅动作展示为 `eventDrivenToolCall`。
- process_exit 完成通知展示为 `eventDrivenTool`。
- 历史恢复或重启恢复时，legacy raw marker 不会残留为普通 `agentMessage`。
- captured output 仍只进入详情，不挤进折叠文本。

非目标：

- 不新增 `processExit` 专用协议类型。
- 不处理 child completion/subagent notification。
- 不改 process subscribe restore failed 后的 waiting 状态。

## 现状结论

协议层已经有明确类型：

- `eventDrivenToolCall` 表示 `process_exit_subscribe` 调用及输出。
- `eventDrivenTool` 表示进程退出通知事件。

raw message 的真实风险不在缺少 `ThreadItem`，而在历史/恢复路径中可能存在 legacy `<event_driven_tool>...</event_driven_tool>` marker 以普通 `agentMessage` 留在客户端状态里。即使后续 `thread/read` 返回了结构化 `eventDrivenTool`，客户端原有合并逻辑只按相同类型做语义去重，无法消费同一内容的 raw `agentMessage`。

## 技术设计

最小连贯改动放在客户端合并边界：

1. 在 TypeScript `normalizeThreadSnapshot`、`updateThreadTurn`、`updateThreadItem` 和 pending update replay 边界，把符合 marker 格式的 `agentMessage` 归一化为 `eventDrivenTool`。
2. 在 Electron 启动恢复用的 CJS snapshot 合并器中做同构归一化，避免启动状态和运行态逻辑分叉。
3. `mergeThreadItem` 在合并前也先归一化，保证同 id 的 legacy raw item 能被结构化 item 覆盖。
4. 保留 Rust history replay 现有结构化路径，并补 process_exit `OutputText` 测试锁住恢复行为。

风险控制：

- marker 必须完整匹配 `<event_driven_tool>` 与 `</event_driven_tool>`，且 JSON 同时包含 string 类型的 `tool/title/text`，普通 assistant 文本不会被误判。
- 不改变 app-server v2 schema 或 wire payload。
- 不改变 `eventDrivenTool` 的展示文案、captured output 解析或 tool grouping。

## 验证计划

- 前端运行态测试覆盖：raw marker history 被归一化；恢复 raw + read structured 时只保留 structured item；`updateThreadTurn` 首次收到 raw marker turn 时归一化；pending agent delta raw marker 遇到 snapshot structured event 时不重复。
- Electron 恢复测试覆盖：启动 snapshot merge 同样归一化并去重。
- Rust 单元测试覆盖：`process_exit_subscribe` assistant `OutputText` marker 从 raw response history 重建为 `ThreadItem::EventDrivenTool`。
