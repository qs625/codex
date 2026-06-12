# 组件拆分和开发 handoff

## ConversationCell

职责：

- 接收 typed `ConversationCell`，不解析 raw message 文本。
- 根据 `cell.kind` 和 message entry role 决定布局方向。
- 为 virtual list 提供稳定 DOM 根节点和测量边界。

状态：

- `message:user`
- `message:agent`
- `message:agentGrouped`，仅作为 UI presentation state 表示连续 assistant/agent 内部多 segment，不包含 user。不得新增 `ConversationCell.kind`、protocol 类型或 raw 展示分支；实现上可用 `data-grouped="true"` / `data-side="system"` 这类派生属性。
- `tool`
- `event`
- `compact`
- `archive`

## MessageRow

职责：

- 渲染左右对齐的 message cell。
- user 右对齐，agent/assistant 左对齐。
- 根据 segment 数量决定 single / grouped 样式。

建议 DOM 语义：

```text
article.message-row[data-side="user|system"]
  div.message-avatar
  div.message-main
    header.message-cell-header
    div.message-cell-surface
      section.message-segment
      section.message-segment
```

开发注意：

- `data-side="user"` 时 row 使用 `justify-content: flex-end`。
- `message-main` 不应 `flex: 1` 占满全行；应使用 `width: fit-content` + `max-width`，并允许正文内部换行。
- user cell 的 header 可右对齐，但屏幕阅读顺序仍应是作者、时间、内容。
- 分组 cell 只显示一个外层 surface，内部 segment 使用轻量 divider；不要给每个 segment 单独大阴影。

## MessageSegment

职责：

- 保留单条 `ConversationEntry` 的文本、附件、状态和操作。
- 支持 streaming 原地更新。

状态：

- `idle`
- `streaming`
- `pending`
- `error`
- `withAttachments`
- `longMarkdown`
- `codeBlock`

行为：

- hover/focus 显示 copy / more 操作。
- error segment 显示错误摘要和可恢复操作。
- 代码块不撑破 cell，遵循现有 Markdown/code 样式。

## 样式 handoff

建议 CSS 方向：

```css
.message-row[data-side="user"] {
  justify-content: flex-end;
}

.message-row[data-side="system"] {
  justify-content: flex-start;
}

.message-main {
  width: fit-content;
  max-width: min(860px, 78%);
}

.message-row[data-side="user"] .message-main {
  max-width: min(720px, 72%);
}

.message-cell-surface {
  border-radius: 8px;
}
```

移动端：

```css
@media (max-width: 720px) {
  .message-main {
    max-width: 94%;
  }

  .message-row[data-side="user"] .message-main {
    max-width: 88%;
  }
}
```

## virtual list handoff

- 如果 cell 内合并减少 row 数，`conversationVirtualization.ts` 的默认高度估算需要用真实长线程检查。
- 保持每个 cell 的外层 `article` 高度可测量，不在内部使用 absolute layout 承载主内容。
- streaming 更新时优先复用同一 cell DOM，避免滚动 anchor 跳动。

## 无障碍

- `article` 需要可被屏幕阅读器理解为一条会话记录或一组连续输出。
- grouped assistant cell 内的 segment 需要有可访问名称或隐藏文本说明边界，例如“assistant message 2 of 3”。
- 操作按钮必须可键盘访问；hover-only 操作在 focus 时同样出现。
- 颜色不作为唯一角色区分，左右对齐、header label 和语义属性共同表达角色。
