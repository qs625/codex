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

## 风险

- `useLayoutEffect` 只在客户端运行，本应用是 Electron/Vite 客户端渲染，不涉及 SSR hydration 风险。
- 对同一 thread 内用户手动上滑的保护仍由滚动事件维护，切换 thread 才会重置贴底意图。
