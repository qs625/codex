# process exit 恢复失败后的等待状态清理

## 任务 brief

用户反馈重启客户端后，部分 `process_exit_subscribe` 恢复失败并提示原 exec session 不可用，但 thread 仍显示为等待 process exit。成功标准是：

- 当 `process_exit_subscribe` 的恢复失败事件进入 thread 历史后，对应旧 process monitor 不再被视为 active。
- active thread 若只剩这类失败恢复的 process monitor，不再显示 `waiting-eventtool`。
- 正常 `Process exited` 事件仍会结束 process monitor。
- 文件监听和 schedule 这类重复触发 monitor 在观察到事件后仍保持 active。

非目标是不修改后端 exec session 恢复机制、不改变 file/schedule 订阅语义、不重做 event tool UI。

## 现象与根因

后端在恢复 process exit 订阅时，如果原 exec session 已不可用，会写入 `Process exit restore failed` 事件，并把失败的订阅从持久化 metadata 中剔除。客户端 root-worker 仍需要从 thread 历史重建 monitor 状态：历史中旧的 `eventDrivenToolCall` 会先被识别为 active monitor，然后再尝试用同 tool 的事件来判断是否已触发。

当前判断只依赖事件文本是否包含 monitor 的 label 或 detail；在多 process monitor、label/detail 不稳定或 restore failed 文本与旧 call 信息不完全一致时，失败恢复事件不能可靠终结旧 monitor，导致 `hasActiveMonitors` 仍为 true，active thread 继续显示等待 event tool。

## 技术设计

最小修复点在 `apps/root-worker-prototype/src/lib/threadAnalysis.ts`：

- 保持现有 monitor 重建流程和 section 数据结构不变。
- 对 `process_exit_subscribe` 增加 restore failed 终止事件识别。
- 终止匹配优先使用 subscription id，其次使用 process session detail，最后沿用旧的 label/detail 文本匹配。
- 只对 `process` monitor 使用观察到事件即终结的语义；filesystem 和 schedule 仍按现有逻辑保留 active，并累计 eventCount/latestEvent。

这样改动只影响客户端从历史推断 active process monitor 的逻辑，不改变后端订阅持久化、RPC 协议或 UI 展示组件。

## 测试设计

- `threadAnalysis.test.ts`：旧 process subscribe call 后出现 matching restore failed event 时，process monitor section 为空，且 eventCount 仍计入历史事件。
- `thread.test.ts`：active thread 只包含旧 process subscribe call 和 restore failed event 时，tree 状态不再是 `waiting-eventtool`。

## 风险

如果后端未来改变 restore failed 文案或字段结构，客户端需要同步更新终止事件识别。当前实现保留 `Process exit restore failed` 标题/文本和 subscription/session 两种匹配方式，能覆盖现有历史事件格式。
