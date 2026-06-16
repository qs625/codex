# Command Wait Replacement History

## 设计结论

本次增量设计覆盖 compact / replacement history 中 `command_wait`、`command_write_stdin` 和 `wait_agent` 的用户可见展示规则。设计方向符合用户预期：replacement history 应帮助用户回看「当时系统为什么等待、等待结果是什么」，不应暴露普通 `function_call_output` JSON。

推荐策略：

- 正式展示以 typed `ThreadItem` 为准：`CommandWait`、`CommandWriteStdin`、`CollabWaitingBegin/End` 或等价 typed lifecycle item。
- 已知等待类工具的普通 raw start / raw JSON output 只作为内部审计/调试数据，不进入默认可见 conversation/replacement history。
- 未知 function output 不全局隐藏，避免误删真正需要用户理解的工具结果。
- 如果 typed item 缺失，不要回退展示 JSON；显示短 fallback event，并标记信息不完整。

## 范围

涉及：

- replacement history 中的 `function_call(name=command_wait)` 与普通 `function_call_output`。
- replacement history 中的 `function_call(name=command_write_stdin)` 与普通 `function_call_output`。
- replacement history 中的 `function_call(name=wait_agent)` 与普通 `function_call_output`。
- typed `CommandWait` / `CommandWriteStdin` / `CollabWaitingBegin` / `CollabWaitingEnd` entry 的信息层级。

不涉及：

- CommandExecutionCell 主体布局重做。
- Live command tail 的输出更新逻辑。
- 未知 MCP/function tool output 的展示策略。
- 从 assistant message、raw marker、legacy JSON envelope 解析 display item。

## Baseline 与原型资产

本次不新增 baseline 截图或 bitmap 原型资产。

原因：这是文字级 UI/UE handoff，关注 replacement history 的内容过滤、typed 展示优先级和文案层级，不改变现有 root-worker 客户端的页面结构、布局、视觉密度或组件形态。既有 command session baseline 已覆盖当前视觉基线：

- [baseline-command-session-2026-06-14.png](../assets/baseline-command-session-2026-06-14.png)

若后续需要调整 command wait entry 的具体视觉样式，例如新增折叠详情、状态 icon 或 grouped timeline，再使用 `$root-worker-playwright-debug` 获取完整 Electron baseline 和状态截图。

## 用户任务

用户在 compact 后回看历史时，需要快速判断：

- 是否曾主动等待 command/subagent。
- 等待的是 output、exit、stdin 写入还是 subagent 更新。
- 本次等待是否命中通知、超时、完成或仍在等待。
- 如果等待命中 command exit，是否成功、退出码是多少。
- 等待窗口是多少，避免误以为 UI 展示的是 hard cap。

用户不需要在默认历史里看到：

- `{"command_id":58732,"status":"completed",...}` 这类 raw JSON。
- `Function call wait-call-1`、`command_wait`、arguments JSON 这类 protocol start 行。
- tool call protocol 的内部字段名。
- 可以由 typed event 直接表达的普通 function call/output 成对记录。

## 信息层级

### CommandWait Replacement Entry

标题：

- `Waited for command`

主状态：

- `Completed`：命中 command completed / exit。
- `Output received`：命中 output notification。
- `Still running`：本次 wait window 结束但 command 仍运行。
- `Timed out`：本次 wait window 无相关通知，返回给模型继续决策。
- `Command unavailable`：target command 不在本地 thread/replacement history 中。

第一行摘要：

- output 命中：`Output notification received`
- exit 命中且成功：`Command completed`
- exit 命中且失败：`Command exited with code N`
- timeout：`No command notification during this wait window`

推荐字段顺序：

1. 关联 command：短 command label 或 `Command #<short id>`，可通过 typed command id 定位时提供跳转。
2. status：用户可读状态，不展示 raw enum 或 JSON。
3. notification：`output` / `exit` / `completed` / `none`，只显示本次 wait 的命中结果。
4. exit code：仅当 command exit/completed 且字段存在时显示；0 显示 `Exit 0`，非 0 显示 `Exit N`。
5. wall time / duration：优先显示本次 wait 实际等待耗时；若 typed payload 只有 command 总时长，标签必须写成 `Command duration`。
6. wait timeout：显示本次 current window，例如 `Wait window 1s`；不得显示 hard cap。

不建议默认展示：

- 完整 stdout/stderr；输出摘要留给 command cell 或 notification event。
- `command_id` 原始数字，除非作为 details/debug 折叠字段。
- `max_wait_timeout_ms` / hard cap。

### CommandWriteStdin Replacement Entry

标题：

- `Sent input to command`

摘要：

- `Wrote stdin to running command`

字段：

- 关联 command。
- 输入长度或行数，例如 `1 line` / `24 chars`。
- 是否包含 trailing newline，不默认展示完整输入内容。
- 结果状态：`Sent`、`Command unavailable`、`Rejected`。

如果 typed payload 暂时没有异常结果状态，默认只能表达 `Sent`；`Command unavailable`、`Rejected` 等异常状态需要后端提供 typed 字段，UI 不从 raw output 推断。

如果输入内容来自用户或可能包含 token/secrets，默认只展示安全摘要；完整内容只能在显式 details/debug 模式中展示，并遵守既有 redaction 策略。

### WaitAgent Replacement Entry

`wait_agent` 默认不展示 raw tool output。用户可见展示应来自 typed collab lifecycle：

- `Waiting for subagent`
- `Subagent update received`
- `Subagent completed`
- `No subagent update during this wait window`

字段：

- target agent label/path。
- wait status。
- update 类型：message、child completion、status changed、timeout。
- wait timeout：本次 current window。

如果 replacement history 中只有 `wait_agent` raw output 而没有 typed lifecycle item，显示 fallback：

- 标题：`Waited for subagent`
- 摘要：`No typed subagent wait event was recorded for this history entry.`
- 字段：target agent、status、wait window，若 typed/canonical 字段不可得则省略。

不得把 `wait_agent` JSON 原样放进 conversation。

## 交互与可访问性

- 这些 replacement entries 默认作为 compact/replacement history 中的轻量 event 行，不需要升级为重型 card。
- 可定位时使用 typed item id 或 target command item id；禁止用 command 文本、JSON 内容或 agent message text 匹配。
- 状态必须用文本表达，不能只靠颜色或 icon。
- fallback event 应保持低视觉权重，但不能完全静默；用户需要知道这里发生过等待动作。
- 如果 function call start 仍可见，行文应是 `Waiting for command notification...` 这类语义动作，而不是 `function_call command_wait`。
- 更推荐的默认策略是隐藏已知等待类 raw function call start，并由 typed replacement entry 承担完整语义；仅在缺少 typed result 且必须保留审计线索时，才显示语义化 fallback start 行。

## 剩余 UX 风险

- 实现时若遗漏 raw function call start，用户仍会看到 protocol 细节；需要把 start 也 canonicalize 为语义化 typed entry，或在 replacement history 中与 typed result 合并展示。
- 如果 `CommandWait` typed payload 没有 `wait_timeout_ms`，UI 只能显示模糊的 `Waited`，用户无法区分 current window 与 hard cap。
- 如果 typed `CommandWait` 只记录 status，不记录 notification 类型，用户无法判断本次 wait 是被 output 唤醒还是 exit 唤醒。
- 如果 `wait_agent` 没有 CollabWaitingBegin/End replacement item，隐藏 JSON 后可能出现历史空洞；需要 fallback event。
- 如果普通未知 function output 被全局隐藏，会伤害 MCP / custom tools 的可审计性；本策略只针对已知等待类工具。

## 开发 Handoff

实现优先级：

1. projector/replacement normalization 识别等待类 tool output，只隐藏已知 `command_wait`、`command_write_stdin`、`wait_agent` 的普通 JSON output。
2. 同一 normalization 也处理等待类 raw function call start：默认隐藏并由 typed entry 承担完整语义；如果 typed entry 缺失，则改写为低权重语义 fallback，不显示 call id、tool name 或 arguments JSON。
3. 为 `command_wait` typed display payload 提供 `wait_timeout_ms`，并在 UI 文案中明确它是 current wait window。
4. 为 `CommandWait` replacement entry 展示 status、notification、exit code、wall time 或 command duration、wait timeout。
5. 为 `command_write_stdin` replacement entry 展示输入摘要和结果状态，不默认展示完整 stdin。
6. 为 `wait_agent` 缺 typed lifecycle 的场景提供轻量 fallback，避免空白历史或 raw JSON 回退。

验收：

- compact/replacement history 不再显示等待类工具的 raw JSON output。
- compact/replacement history 不再显示等待类工具的 protocol start 行，例如 `Function call <call_id>`、`command_wait` 或 arguments JSON。
- command wait 历史可读出「等了什么、等到什么、等多久、是否退出、退出码」。
- wait timeout 显示本次 current window，不显示 hard cap。
- 未知 function call/output 仍按现有规则可见。
- 不从 raw marker、assistant text 或 JSON envelope 反解 UI 展示。
