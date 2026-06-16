# Information Architecture

## 页面结构

```text
Root Worker App
├─ Sidebar: Agent Tree
├─ Main Thread Panel
│  ├─ Thread Header
│  │  ├─ Path / role / presence / run config
│  │  └─ Goal Strip (仅有 goal state 时显示)
│  ├─ Conversation Virtual List
│  └─ Composer
│     ├─ Draft skills/images
│     ├─ Textarea
│     ├─ Slash Menu
│     └─ Toolbar / send / stop / voice
└─ Right Panel
   ├─ Todo Board
   ├─ Thread Analysis
   │  ├─ Context / monitors / skills
   │  └─ Goal Detail
   ├─ File Preview
   └─ Rail
```

## 既有 Command Session 信息层级

Command cell header：

- 主文本：command，单行截断，title 保留完整命令。
- 次级文本：cwd path、status、duration 或 running time。
- 状态 pill：`Running`、`Waiting`、`Completed`、`Exit N`、`Failed`。

Command cell details：

- Execution：command、cwd、status、duration、exit code。
- Session：initial wait、notify on、yield wait alias、tty、max output tokens、sandbox/approval 参数。
- Output summary：最后非空输出行、输出截断说明、是否仍有 live tail。

Notification event：

- 位置：conversation 中作为独立 event/tool-adjacent entry 出现，不塞进 command cell 的 details。
- 主文案：`Command output notification` 或 `Command exit notification`。
- 关联：通过 typed command item id 回到 command cell。

## 既有 Composer Slash 菜单信息层级

- 菜单锚定在底部 composer 上方，最大宽度跟随 composer，最大高度不超过 conversation 可视区的 40%。
- `Commands` 分组优先展示内置命令，候选行包含 command token、动作说明、执行提示。
- `Skills` 分组展示可用 skill，候选行至少包含 `$skill-name`；说明仅在现有 metadata 可用时展示。
- 状态行只出现在对应分组内：loading、error、empty，不占用 active 候选序列。

## Goal 信息层级

Header Goal Strip：

- 状态：`Active` / `Paused` / `Complete` / `Budget limited` / `Cancelling` / `Cancelled`
- 摘要：goal 内容前 120-160 字符，两行截断。
- 预算：剩余 token/time/turn budget，字段不存在时不显示。
- 操作：Cancel goal。

Thread Analysis Goal Detail：

- 完整 goal 内容。
- 状态、创建/更新时间、预算使用、remaining budget。
- 最近 lifecycle event：created、updated、continued、cancel requested、cancelled、failed。
- 操作区：Cancel goal、Copy goal text。

Slash Menu：

- Commands 分组显示：
  - `/init`：system skill。由 Skills discovery 提供，不属于 runtime goal command。
  - `/goal cancel`：Cancel the current thread goal。
  - `/clear`：Archive this root session and start a fresh root。
- Skills 分组保持现状。

## 响应式策略

- 宽屏：Goal Strip 位于 header 下方，占中间 panel 宽度；Right Panel Goal Detail 正常显示。
- 中等宽度：Goal Strip 仍显示，但按优先级折叠：先隐藏 secondary budget detail，只保留 `Budget limited` 等状态语义；目标摘要一行截断；完整预算进入 Goal Detail。
- 窄宽度：Right Panel 可关闭；Goal Strip 保留状态 badge、单行摘要和固定 32px icon cancel button。Cancel button 命中区域不小于 32x32，strip 最小高度 44px，预算文本全部隐藏到 detail/tooltip，不能挤压 composer 或最右 rail。
- 长 goal 内容：summary 区域使用 `min-width: 0` 和 text overflow；不能让 button 或 badge 被压缩。

## 不应改变的边界

- Agent Tree 不展示 goal 完整内容，只可在未来加一个小 goal indicator；本 feature 不建议加入，以免和 canonical `ThreadStatus` 混淆。
- Conversation cell 不应把 goal state 作为普通 agent bubble 插入，除非后端提供 typed lifecycle `ThreadItem`。
- Search/定位只能基于已投影的 `ConversationEntry` / `ConversationCell`，goal header state 不参与 ThreadItem 合并。
