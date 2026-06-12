# UE 交互流程

## 主路径

1. 用户在 composer 提交任务。
2. 会话列表新增一个 `userMessage` cell，右对齐，最大宽度受限，显示用户文本和必要附件摘要。
3. app-server live 推送 assistant / agent 输出。
4. 如果连续输出都属于 assistant / agent message，则进入同一个左侧 message cell。
5. cell 内部按时间顺序堆叠 message segment：第一段显示作者、状态和时间；后续段用细分隔线或紧凑 meta 标记边界。
6. 下一条 `userMessage` 到来时结束左侧合并，新增右侧 user cell。

## 合并边界

允许合并：

- 连续 `agentMessage` / assistant message。
- 同一侧系统输出的 streaming 增量更新，保持在当前 segment 中更新。
- 同一连续块里的短状态提示，可作为 cell header 或 segment meta。

必须断开：

- 任意 `userMessage`。
- `tool` / `event` / `compact` / `archive` cell。
- agent/assistant 作者身份发生变化且需要保留身份识别时。
- typed source 已经不是 `agentMessage` 的 error、cancelled、permission required 等状态。断开方式取决于 typed item：如果源 item 仍是 `agentMessage`，留在当前 message cell 内作为 error/cancelled segment；如果源 item 是 tool、event、collab、hook 或 permission 轨迹，则保持对应 cell kind，不合并进 message cell。
- 附件、图片、代码块特别长且需要单独操作时，可保留在同 cell 但必须有 segment 边界。

## 分支状态

- 空状态：保持现有空线程入口，不引入左右气泡；空状态不是对话内容。
- 加载中：message cell 尾部显示紧凑 loading indicator，不新增空白大 cell。
- 流式输出：segment 内容原地增长，cell 宽度和外层对齐不跳变。
- 错误：错误 segment 使用左侧 cell 内的 inline error bar；若错误会阻塞下一步操作，单独断成 event/tool 类状态更清晰。
- 权限请求：通常属于 tool/hook 执行轨迹，保持 tool/event 类 cell；不要伪装为 assistant 普通文本。
- compact/archive：compact 后的历史保持独立 archive section；展开后内部仍按左右对齐和合并规则渲染。

## 反馈规则

- 用户发送成功：右侧 cell 立即出现，避免等待 assistant 后才显示。
- agent 正在回复：左侧 cell 尾部显示 `running` / `streaming` 状态。
- 子任务完成：如果是 typed child completion / subagent notification，按现有 tool/event 轨迹展示，不伪装成普通 assistant message。
- 复制、重试、查看原始项等操作只在 hover/focus 时出现，移动端通过更多菜单进入。
