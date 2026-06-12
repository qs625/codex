# 相关产品和模式调研

## 调研结论

会话类界面通常用左右分栏建立“自己 vs 对方/系统”的归属感。对开发者 agent 线程来说，最重要的不是模拟即时聊天，而是让长线程可扫描：用户意图应该像锚点一样出现在右侧；系统连续输出、agent 进展和 assistant 回复应形成左侧可折叠认知块。

## 外部模式参考

- NN/g 的 chat UX 指南强调聊天有书面轨迹，用户会回看上下文并在等待时多任务；因此会话区要让历史记录易于复扫，而不是只优化实时发送。来源：https://www.nngroup.com/articles/chat-ux/
- MUI X Chat Message List 将 message list 定义为可滚动历史区域，并明确包含 date dividers、message groups、streaming indicators 等模式。来源：https://mui.com/x/react-chat/material/message-list/
- Material Design Lists 将 list 描述为垂直文本和元素组，目标是阅读理解与扫描效率。来源：https://m3.material.io/components/lists/overview
- Apple HIG layout 强调内容和控件关系在不同设备尺寸中保持一致；Apple writing 指南强调简单、清晰、可本地化的文案。来源：https://developer.apple.com/design/human-interface-guidelines/layout 和 https://developer.apple.com/design/human-interface-guidelines/writing

## 对本项目的设计推导

- 不应把每条 assistant 输出都做成同级大卡片，否则长线程会出现过多头像、header 和阴影噪音。
- 不应把 user 与 assistant 混入同一个 cell；用户消息是线程中最强的任务边界，右对齐能显著提升回看效率。
- tool / event / compact / archive 是执行轨迹，不是普通 assistant 文本；它们可以视觉上贴近左侧系统轨迹，但不应被合并进 message cell。
- streaming / pending 是状态，不是新布局类型；应在当前 message cell 内显示状态，避免流式输出造成 cell 抖动。

## 当前代码现状

- `ThreadItem` typed union 在 `apps/root-worker-prototype/src/types.ts`。
- `ConversationEntry` / `ConversationCell` 是展示中间层，`cell.kind` 包含 `message | event | tool | compact | archive`。
- `buildConversationState` 已从 selected thread 生成 entries 和 cells，并做对象复用。
- `MessageRow` 当前按 cell 渲染 `message-stack`，每个 entry 是一个 `message-bubble`。
- 现有聚合逻辑中 agent message 已可连续合并到 message cell；本次重点是补齐 user 右对齐、合并边界和视觉规范。
