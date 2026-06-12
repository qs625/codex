# 信息架构

## 页面结构

```text
ConversationViewport
  ConversationVirtualList
    ConversationCell(message, role=user)
    ConversationCell(message, role=agent/assistant)
      MessageSegment[]
    ConversationCell(tool/event/compact/archive)
  Composer
```

## 信息层级

1. cell 对齐方向：第一优先级，用于区分 user 与系统/agent。
2. cell header：作者、时间范围、状态。
3. segment 内容：Markdown、代码、附件摘要。
4. segment meta：单条 message 的时间、模型/agent 名称、streaming/error。
5. 操作：复制、展开、查看详情、重试。

## 桌面布局

- Conversation 内容区保持全宽滚动，但 message cell 需要 max-width，建议：
  - user cell：`max-width: min(720px, 72%)`，靠右。
  - agent/assistant cell：`max-width: min(860px, 78%)`，靠左。
  - tool/event cell：沿用左侧轨迹宽度，必要时可略宽但不超过内容区 86%。
- 外层 row gap 维持 16-18px，cell 内 segment gap 8-10px。
- avatar 对系统侧保留；user 侧可隐藏头像或放在右侧，避免抢占文本宽度。

## 移动布局

- 断点建议在 `<= 720px`。
- conversation padding 收敛到 8-12px。
- user cell：`max-width: 88%`。
- agent/assistant cell：`max-width: 94%`。
- avatar 缩小到 28-32px，或在窄屏只保留首个系统 cell 的小型身份标识。
- header 允许换行：作者在第一行，时间/状态在第二行右侧或低对比文本，不能压缩正文。

## 可读性

- 正文理想行宽控制在约 64-78 个英文字符范围；长代码块是例外，使用现有代码块横向滚动。
- user cell 背景可略有色彩，但必须保证文本对比度；不要只靠颜色表达角色。
- assistant/agent 合并 cell 内部用分隔线、meta label 或局部 padding 表达段落边界，避免多段内容被误读为单条回复。
