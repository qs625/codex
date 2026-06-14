# Root-worker 客户端设计基线

## 产品目标

root-worker prototype 是面向 Codex 多线程/多 agent 工作流的桌面客户端。核心任务是让用户稳定理解当前线程、工具执行、子 agent、上下文和等待状态，并能从实时状态快速回到 conversation 中的 canonical 记录。

## 目标用户

- 角色：高频使用 Codex 的工程师、维护者、agent 调度者。
- 频率：日常连续使用，常伴随长命令、后台 command session、subagent 和 event notification。
- 设备：桌面 Electron，主要是键盘和鼠标操作。
- 专业程度：熟悉 shell、构建日志、exit code 和 command session 参数。

## 设计原则

- Conversation 是事实记录：所有可回溯内容以 typed `ThreadItem` / typed conversation event 为来源，不从 raw marker 或 message text 反解。
- RightPanel 是实时索引：只显示需要用户关注或可快速跳转的 live/recent 状态，不替代 conversation 详情。
- 命令信息分层：command cell 解释一次 command session 的完整生命周期；notification event 解释后续 output/exit 通知；RightPanel 只给定位和摘要。
- 不打断输入：右侧点击定位只滚动和高亮 conversation cell，不改变 composer draft，不抢输入焦点。

## 当前 feature

Feature：Command Session UI details and live index behavior。

目标：
- 补全 command cell 详情层级，展示 command、cwd、status、duration、exit code、output 摘要、initial wait、notify on 等 session 参数。
- 明确 Live Commands 状态规则：running/waiting 显示，successful completed 自动消失，failed 保留为近期失败。
- 明确点击 Live Commands 定位：通过 `ThreadItem.id` 关联 conversation cell，滚动到目标并短暂高亮。
- 将 output/exit notification 作为独立 typed conversation event 显示，与 command cell live tail 区分。

## Baseline

当前 baseline 使用完整 Electron smoke 获取，不使用 Vite renderer 直开：

![Command Session baseline](assets/baseline-command-session-2026-06-14.png)
