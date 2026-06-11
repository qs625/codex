# 设计 Review

## Review 状态

第一轮 review：未通过。已根据 findings 修正。

第二轮复审：通过，无 blocking findings。

## 第一轮 Findings 与处理

1. 跳转语义不一致：已统一为 runtime agent/thread 节点主点击、Enter、`Open thread` 都同时定位 Agent Tree 并打开 conversation；`Show in Tree` 只定位和高亮，不切换 conversation。
2. 响应式规则不具体：已补充 >=1440px、1024-1439px、<1024px 三档布局、最小宽度、overflow、折叠和长 path 截断策略。
3. 可访问性 handoff 不足：已补充 runtime node button 语义、accessible name、焦点顺序、details 焦点恢复和 live region 策略。
4. 主图过度重画：已新增 [workflow-panel-added-on-baseline.png](/Users/bytedance/Projects/my-codex/ui-design/dynamic-workflow/assets/workflow-panel-added-on-baseline.png)，真实 baseline 原样保留，只在右侧追加 Workflow Graph panel。

## 复审结论

通过。

- 主设计图符合“真实 baseline 原样保留，只在右侧新增 Workflow Graph panel”的要求。
- 左侧 Agent Tree、中间 conversation、Thread Analysis 和原右侧 rail 均未被重画或替换。
- runtime agent/thread 节点主点击、Enter、`Open thread` 都定位 Agent Tree 并打开 conversation；`Show in Tree` 只定位和高亮，不切换 conversation。
- 响应式和可访问性 handoff 已补到可进入开发的程度。

## 自检清单

- UX：主列表和右侧 Graph 的职责已拆分，避免信息过载。
- UI：工程工具风格，低装饰、高密度、可扫描。
- Accessibility：状态不只依赖颜色，节点可键盘访问，动态更新使用 polite 策略。
- Engineering：不要求客户端探测 runner，不要求 raw message 解析，不实现流程编辑器。
- Content：runnerStatus、branch、loop、parallel、join、resume、missing binding 均有明确文案方向。

## 待 review 问题

- 是否需要把 `Graph` rail 从 disabled 改为 workflow run 存在时 enabled。
- WorkflowRunCard 默认高度 3 到 5 行是否适合当前 conversation density。
- loop iteration 在无 metadata 时按创建顺序分组是否足够可靠，还是必须要求 runtimeGraph 提供 iteration id。
