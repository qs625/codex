# 切换 thread 后 conversation 自动贴底

## 任务 brief

- 用户：Root Worker Prototype 前端用户。
- 缺陷：用户在当前 thread 中上滑查看历史后，再切换到另一个 thread，conversation 仍保留旧滚动意图，没有自动定位到新 thread 的最新消息底部。
- 成功标准：每次选择不同 thread 时，conversation 都会在渲染前恢复贴底意图并滚到该 thread 底部；同一 thread 内用户手动上滑后，普通内容更新不应强制拉回底部；已有新消息贴底行为不回归。
- 非目标：不调整 conversation 布局、不修改 EventCommand/Rust/协议、不改变虚拟列表窗口算法。

## 技术设计

现有实现用 `shouldStickConversationToBottomRef` 记录用户是否接近底部，`handleConversationScroll` 在用户滚动时更新该状态，后续 `useLayoutEffect` 根据该状态决定内容变化时是否 `scrollTop = scrollHeight`。

根因是 thread 切换时虽然会把 `shouldStickConversationToBottomRef` 重置为 `true`，但重置发生在 `useEffect` 中，晚于负责滚动的 `useLayoutEffect`。如果旧 thread 中用户曾上滑，该 layout effect 会先读到旧的 `false` 并跳过滚动，之后重置为 `true` 时没有新的滚动触发。

最小连贯改动：

1. 将 `selectedThreadId` 变化时的贴底重置改为 `useLayoutEffect`，保证它在滚动 layout effect 之前执行。
2. 给 `ConversationVirtualList` 使用 `key={selectedThreadId}`，让 thread 切换时重建内部 viewport、测量缓存和工具展开状态，避免沿用旧 thread 的虚拟窗口。
3. 抽出 `isConversationNearBottom`，统一表达 24px 贴底阈值并补单元测试，降低行为回归风险。

## 后续修正：live 内容高度变化时的贴底策略

### 任务 brief

- 用户：Root Worker Prototype 前端用户。
- 缺陷：当前 conversation viewport 在底部或接近底部时，live delta、item started/completed 或测量结果更新可能让尾部 cell 高度变化；虚拟列表先执行锚点补偿再下一帧滚底，导致新内容到来时出现异常向上滚。
- 成功标准：viewport 在底部或接近底部时，新内容和 cell 高度变化应跟随到底部；用户已滚离底部时，新内容和高度变化不应拉回底部，并且视口上方 cell 高度变化时保持当前阅读锚点稳定。
- 非目标：不重做 conversation UI 布局；不改变 `ThreadItem -> ConversationEntry -> ConversationCell` 展示语义；不改变 live item 合并规则。

### 技术设计

`ConversationVirtualList` 的 cell 测量回调根据测量高度更新虚拟布局。原逻辑在判断贴底后仍会先对位于 viewport 上方的 cell 执行 `scrollTop += heightDelta`，随后再通过 `requestAnimationFrame` 滚到底部。对于贴底场景，尤其是估算高度高于实际测量高度时，这个补偿会先把 viewport 向上移动，造成可见跳动。

最小连贯改动：

1. 在 `conversationScroll.ts` 中新增 `planConversationHeightChangeScroll`，复用统一的 24px 贴底阈值。
2. 贴底时不执行锚点补偿，只在高度版本更新后滚到底部。
3. 非贴底时保持原有阅读锚点策略：只有变化 cell 的 top 在当前 `scrollTop` 上方时，才按高度差调整 `scrollTop`。
4. 用纯函数测试覆盖贴底不补偿、离底且上方变化补偿、离底且下方变化不移动三类关键行为。

## 风险

- `useLayoutEffect` 只在客户端运行，本应用是 Electron/Vite 客户端渲染，不涉及 SSR hydration 风险。
- 对同一 thread 内用户手动上滑的保护仍由滚动事件维护，切换 thread 才会重置贴底意图。
- live 内容高度变化仍依赖下一帧读取更新后的 `scrollHeight` 完成贴底；如果后续引入异步媒体加载，需要继续通过同一测量回调进入该策略。
