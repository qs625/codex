# UE Flow

## Command Session 主路径

1. agent 发起 `exec_command`。
2. conversation 插入一个 command cell，显示命令摘要和 running/in progress 状态。
3. command cell 可展开查看 session details：命令、工作目录、状态、时长、退出码、output 摘要和 session 参数。
4. command 运行中或等待 notification 时，RightPanel 的 Live Commands 显示该条 session。
5. `output` 或 `exit` notification 到达时，conversation 追加独立 typed notification event，用文案说明这是来自同一 command session 的通知，而不是 command cell 本身的展开内容。
6. 点击 RightPanel 条目，conversation 滚动到对应 command cell 或最近一条关联 notification，并短暂高亮。
7. 成功完成后 Live Commands 自动移除；失败完成后作为近期失败保留。

## 点击定位

- 关联键：只使用 typed item id。command cell 使用 `ThreadItem.id`，关联 notification event 使用 `targetCommandItemId` 或等价 typed reference。
- 默认目标：点击 command row 定位到 command cell。
- 如果 row 展示的是 latest notification 摘要，辅助点击区域或二级动作可定位到 latest notification；默认仍回到 command cell。
- 定位行为：滚动到目标，使其位于 conversation 可视区域中上部；目标 cell 高亮 1600ms 到 2400ms。
- 输入保护：不 blur composer，不修改 selection，不自动展开 details；如果目标 command details 已折叠，只高亮外层 cell。
- 未找到目标：保留 RightPanel 行，显示轻量失败状态，例如 `Not in local view`，并提供不可点击/禁用语义。

## 状态覆盖

- 空态：`No live commands.`，保留在 Command Activity 下。
- 加载态：thread analysis 尚未构建时显示 `Loading command activity...`，不要闪烁成空态。
- running：显示 `Running`，可附 latest output tail。
- waiting：显示 `Waiting` 或 `Waiting: output` / `Waiting: exit`，说明正在等待下一次 command session notification。
- success completed：从 Live Commands 移除，完整记录留在 conversation command cell 和 notification events。
- failed completed：显示 `Exit N`，作为近期失败保留；行视觉使用 error status，但不占用 running 语义。
