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
│  │  └─ Goal Lifecycle Event Cell (typed ThreadItem history)
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
- 操作：Pause / Resume / Cancel goal，按 canonical goal status 和后端能力显示。

Thread Analysis Goal Detail：

- 完整 goal 内容。
- 状态、创建/更新时间、预算使用、remaining budget。
- 最近 lifecycle event：created、updated、continued、cancel requested、cancelled、failed。
- 操作区：Pause / Resume / Cancel goal、Copy goal text、Edit goal。

Slash Menu：

- Commands 分组显示：
  - `/goal <objective>`：创建或更新当前 thread goal；带参数，选择后应补全 token 并让用户继续输入。
  - `/goal pause`：暂停当前 active goal。
  - `/goal resume`：恢复当前 paused goal。
  - `/goal cancel`：Cancel the current thread goal。
  - `/clear`：Archive this root session and start a fresh root。
- Skills 分组显示：
  - `/init`：system skill。由 Skills discovery 提供，不属于 runtime goal command。
- 其他 Skills 分组内容保持现状。

Goal action feedback：

- Composer status 承载从 slash command 发起的 set/pause/resume/cancel 结果。
- GoalStrip 承载当前 goal 的 live 状态与最近 action error，不展示完整事件历史。
- Thread Analysis Goal Detail 承载完整 objective、预算、最近 lifecycle event 和 action 区。
- Conversation 只有在后端提供 typed goal lifecycle `ThreadItem` 时展示事件；不得从普通 agent message 或 raw marker 反解。

Conversation Goal Lifecycle：

- 位置：Conversation Virtual List 中，按 typed item 时间顺序出现。
- 主信息：`Goal created` / `Goal updated` / `Goal complete` 等 lifecycle 标题。
- 次信息：目标摘要、状态 badge、时间、可选结构化失败原因。
- 不承载：完整预算表、action button、完整 objective 长文；这些进入 Goal Detail。
- 关联：entry id 等于 typed item id，RightPanel Recent event 和搜索定位都使用该 id。
- 响应式：title 可截断，badge 不压缩；time 在空间不足时下移到 meta 行或隐藏到 tooltip/aria-label；objective preview 使用两行 clamp。
- 定位入口：RightPanel Recent event 渲染为 button/link，支持 Tab、Enter、Space、focus ring 和跳转后的 live status。

## 响应式策略

- 宽屏：Goal Strip 位于 header 下方，占中间 panel 宽度；Right Panel Goal Detail 正常显示。
- 中等宽度：Goal Strip 仍显示，但按优先级折叠：先隐藏 secondary budget detail，只保留 `Budget limited` 等状态语义；目标摘要一行截断；完整预算进入 Goal Detail。
- 窄宽度：Right Panel 可关闭；Goal Strip 保留状态 badge、单行摘要和固定 32px primary action。操作按钮命中区域不小于 32x32，strip 最小高度 44px，预算文本全部隐藏到 detail/tooltip，不能挤压 composer 或最右 rail。
- 长 goal 内容：summary 区域使用 `min-width: 0` 和 text overflow；不能让 button 或 badge 被压缩。
- GoalLifecycleEventCell 在窄宽度下保持 icon + pill，不扩成整页 card；第一行 title 截断、badge 完整、time 降级，第二行 preview clamp，避免标题、badge、time 相互覆盖。

## 不应改变的边界

- Agent Tree 不展示 goal 完整内容，只可在未来加一个小 goal indicator；本 feature 不建议加入，以免和 canonical `ThreadStatus` 混淆。
- Conversation cell 不应把 goal state 作为普通 agent bubble 插入，除非后端提供 typed lifecycle `ThreadItem`。
- Goal lifecycle cell 不应替代 GoalStrip 的当前状态，也不应让 RightPanel 通过 objective 文本搜索定位；三者以 typed item id / typed goal state 建立关系。
- Search/定位只能基于已投影的 `ConversationEntry` / `ConversationCell`，goal header state 不参与 ThreadItem 合并。
