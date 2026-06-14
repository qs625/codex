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

## Composer Slash 菜单主路径

1. 用户在 composer 当前已支持的 slash 触发位置输入 `/`：trimStart 后第一行以 `/` 开头，且 slash query 不包含空格。本次不扩大到普通文本边界、路径中间或多行任意位置触发。
2. composer 上方打开 `SlashCommandMenu`，焦点仍在 composer，第一条可选候选进入 active descendant。
3. 菜单按 `Commands`、`Skills` 分组展示。Commands 行显示 token、动作说明和类型/快捷提示；Skills 行至少显示 `$skill-name`，如 metadata 可用再显示短说明。
4. 用户继续输入查询，例如 `/comp`；菜单即时过滤名称、别名和简短说明。
5. `Down` / `Up` 在可见候选中循环移动，跳过分组标题和禁用行。
6. `Enter` 选择 active 候选；鼠标点击选择对应候选。
7. 选择内置命令时，使用候选的稳定 `commandId` 执行语义动作。本次验收只覆盖现有无参数内置命令 `/clear`。
8. 选择 Skill 时，沿用当前 skill slash 行为：添加对应 Skill chip/attachment，payload 继续走现有结构化 skill 输入链路；本次不改变清空 slash query、chip 展示和提交规则。
9. `Tab` 对 active 候选做补全/选择：内置命令补全为 `/command ` 或直接执行；Skill 走现有选择行为；不触发普通消息发送。
10. `Escape` 关闭菜单，保留 composer 中的 `/query` 文本和 selection。

## Composer Slash 菜单分支与反馈

- 触发条件：沿用当前 `trimStart()` 后首行 `/query` 规则；URL、文件路径或普通单词中间的 `/` 不打开。
- 关闭条件：`Escape`、点击菜单外、光标移动到 slash token 之外、删除触发 slash、提交消息后。
- 空态：查询无匹配时显示 `No commands or skills match “/query”`，保留关闭和继续输入能力。
- 加载态：Skills 尚未加载时保留 Commands 分组，Skills 区显示 `Loading skills...`，不阻塞内置命令。
- 失败态：Skills 加载失败时显示 `Skills unavailable` 和短原因；Commands 仍可选。
- 鼠标：hover 更新 active 候选；点击 row 选择；滚轮只滚动菜单，不滚动 conversation。
- Skill chip 保留：本次新增内置命令不能破坏现有 skill chip 和 payload 行为；重复选择、删除和提交沿用当前实现。
- 内置命令执行语义：`/clear` 不得作为普通用户消息发送；选择后按当前实现归档当前 session threads 并创建新的 root thread，菜单文案必须提示真实后果。
