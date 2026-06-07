# UE 交互流程

## 主路径：child agent 正常完成

1. 父线程 conversation 收到 `collabAgentStatusUpdate`。
2. 列表新增一条 Agent Status item。
3. item 标题行展示：状态标签、child agent label、可选 role/path 短标识。
4. 如果 `status.message` 非空，标题或详情首行展示短 completion preview。
5. 如果 message 超过摘要限制，末尾显示省略，完整内容只在 details / 展开区域显示。

推荐默认文案结构：

```text
• Agent completed Robie [worker]
  └ 39916800
```

长 completion：

```text
• Agent completed Robie [worker]
  └ Implemented parser changes, added regression tests, and verified...
```

## 分支：child agent 报错

1. item 标题行必须显式展示错误状态。
2. 错误摘要优先使用 `status.message`。
3. 错误摘要限制为 1 行或 160 graphemes，两者先到为准。
4. 完整错误内容在详情中保留。

推荐结构：

```text
• Agent errored Robie [worker]
  └ tool timeout while running cargo test...
```

## 分支：无 message 的终态

无 message 时不渲染空详情行。

```text
• Agent completed Robie [worker]
```

## 分支：非终态状态

`Running`、`PendingInit` 这类状态只展示状态和 agent label。若需要展示说明，限制为 1 行，避免运行中状态频繁刷新造成视觉跳动。

## 展开 / details 流程

1. 用户聚焦或选择该 item。
2. 用户使用现有 conversation item 的 details / expanded 入口打开详情；若当前 TUI 已有统一快捷键或查看详情动作，必须复用该动作，不为 Agent Status item 单独发明一套键盘交互。
3. 完整内容按段落或原始换行保留，不再做 grapheme 截断。
4. details 顶部固定展示结构化元数据：
   - status
   - agent path
   - sender thread
   - recipient thread
   - item id
5. 完整 completion 放在元数据之后，支持选择、复制和滚动。

## 键盘与可访问性流程

1. Agent Status item 必须可通过现有 conversation 列表焦点路径抵达。
2. 焦点停在 item 上时，screen reader 或等价辅助输出应按固定顺序朗读：状态、agent label、role、是否有截断摘要、打开 details 可查看完整内容。
3. 状态不能只依赖颜色表达；`Completed`、`Error`、`Interrupted`、`Not found` 等文本必须在未展开状态可见。
4. 进入 details 后，焦点应落在详情容器或完整 message 的起始位置，用户可以继续滚动、选择和复制完整 `status.message`。
5. 若 message 在列表中被截断，item 需要提供可感知提示，例如视觉省略号和辅助文本“full message available in details”。该提示不应占用额外列表行。

## 加载状态

如果 item 已到达但 agent metadata 尚未解析，先使用 `sender_path` 或 `status.path` 作为 label。metadata 到达后允许就地替换为 nickname / role，但不能改变 message 截断策略。

## 空状态

Agent Status item 本身不需要单独空状态。若 details 中 `status.message` 为空，显示结构化元数据即可，不显示“暂无内容”占位。

## 反馈

- 终态 `Completed` 使用成功语义，但不应过度强化，保持 history cell 的低噪音。
- `Errored` 和 `NotFound` 使用错误语义，必须在未展开状态可见。
- `Interrupted` 使用 warning 语义，和错误区分。
