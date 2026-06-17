# Root-worker Composer Slash 菜单

## Brief

用户：root-worker prototype 的桌面客户端用户。

能力：在 composer 输入 `/` 时，同一个候选菜单同时展示当前客户端可执行的内置 slash command、可用 Dynamic Workflows 和可用 Skills。选择 Skill 继续沿用现有 chip 与 `turn/start` skill payload；选择 workflow 基于 discovery 结果补全一段用户可编辑的请求草稿，由模型按 init context 使用 workflow tool；选择内置命令执行客户端已有语义或补全命令 draft，不作为普通用户文本发送。

补充能力：模型启动 workflow 后，root-worker conversation 能消费 typed `ThreadItem::WorkflowRunProgress` 并显示 workflow run 进度；workflow-created thread/agent 的所属 badge 只能来自后端 typed thread metadata binding，当前协议缺少该 metadata 时不显示 badge，也不从 progress item、runner output 或文本反推。

成功标准：
- 输入 `/` 时展示 `/clear`、`/goal <objective>`、`/goal pause`、`/goal resume`、`/goal cancel`、discovered workflows 和未选择的 Skills。
- 输入 `/query` 时按命令 token、说明、alias、workflow id/name/description/source/path/when-to-use/input name 以及 Skill name/kind/path 过滤。
- Skills 列表为空时仍展示 `/clear` 和 `/goal <subcommand>` family。
- 选择 Skill 后仍生成 Skill chip，提交 payload 保持 `{ type: "skill", name, path }` 语义。
- 选择 workflow 后写入 `Use the <workflow-id> workflow...` 草稿，用户可补充 objective/cwd 等 inputs；客户端不直接调用 app-server `workflow/start`。
- 模型调用 `workflow_start` 后，conversation 显示 workflow progress cell，包含 workflow id、run id、runner status、message、terminal status 和 typed status details。
- 当前 `WorkflowRunProgressEvent` 不携带 `staticGraph` / `stages`，root-worker 显示 `No graph details in this update.`，不从 workflow output 或 assistant text 构造 graph。
- 当前 app-server v2 `Thread` metadata 不携带 workflow id/run id/stage binding，Agent Tree 不显示 workflow-owned thread badge；后端补 typed binding 后再显示。
- 选择 `/clear` 时调用现有 root session clear 语义，不创建普通 turn 文本。
- 选择 `/goal <objective|pause|resume|cancel>` family 时只补全 composer draft；发送前拦截完整命令并调用 typed goal API，不创建普通 turn 文本。
- 手动提交 `/goal <objective>` 时调用 `thread/goal/set` 创建或更新 active goal；手动提交精确 `/goal pause` / `/goal resume` 时调用 `thread/goal/set` 更新 status；手动提交精确 `/goal cancel` / `/goal clear` / `/cancel-goal` 时调用 `thread/goal/clear`。
- 只有完全等于 `/goal pause|resume|cancel|clear` 才按动作处理；例如 `/goal pause this migration` 是 objective。
- 空 `/goal` / `/goal ` 显示 `Enter a goal objective.`，不进入普通 user message。
- `/init` 由 embedded system skill 提供，随 Skills discovery 自动出现在菜单里；root-worker 不为 `/init` 增加 builtin 特判。

非目标：
- 不新增 `/compact` 等当前 root-worker 客户端尚无执行入口的命令。
- 不把可以表达为 Skill 的命令硬编码成 root-worker builtin slash command；workflow 候选必须来自 `workflow/list` discovery，不硬编码具体 workflow id。
- 不改变 app-server v2 `UserInput::Skill` wire 语义。
- 不修改 TUI slash command 行为。
- 不新增 raw marker、assistant JSON、legacy envelope、workflow tool output 或 runner output parser 来反解 progress graph 或 thread 所属关系。

## 技术设计

实现形态：
- 新增 `src/lib/slashMenu.ts` 承载纯候选逻辑，统一输出 `ComposerSlashSuggestion`。
- `Panels.tsx` 从 Skill 专用 `skillSuggestions` 改为通用 `slashSuggestions`，键盘和鼠标选择复用同一 handler。
- `App.tsx` 提供 `runComposerSlashCommand(commandId)`，`clear` 调用已有 `clearCurrentRootSession()`；goal family 由菜单补全 draft，发送前解析后调用 `thread/goal/set` 或 `thread/goal/clear`；同选中 thread cwd 一起加载 `workflow/list`，并把 discovered workflows 传给 slash menu。
- `electron/preload.cjs` / `electron/main.cjs` 暴露 `listWorkflows(cwd)`，仅调用 app-server v2 `workflow/list` 控制面，不直接启动 workflow。
- root-worker `ThreadItem` 类型补齐 `workflowRunProgress`，`buildConversationItemEntries` 将其映射为 `toolCategory: "workflow"` 的 conversation tool entry；`Conversation.tsx` 复用 tool card 展示 details。
- workflow-created thread/agent badge 设计见 `ui-design/root-worker-client/features/workflow-progress-display.md`，实现前提是后端提供 typed thread workflow binding metadata。
- `Panels.tsx` 只有在当前 draft 没有 image attachment 和 Skill chip 时启用内置命令候选，避免候选选择路径绕过手动 `/clear` 的保护条件。
- `codex-rs/skills/src/assets/samples/init/SKILL.md` 提供 embedded `init` system skill。`SkillsManager::new` 会安装 bundled system skills 到 `$CODEX_HOME/skills/.system`，`skills/list` 通过现有 roots 加载它，因此未初始化项目也能发现 `/init`。

数据流：
1. composer draft 通过 `getActiveComposerSlashQuery` 判断是否处于 slash token。
2. `buildComposerSlashSuggestions` 合并内置命令、discovered workflows 与未选择 Skills。
3. 选择 `type: "skill"` 时调用现有 `onAddDraftSkill`，保留 chip/payload。
4. 选择 `type: "command"` 时，`/clear` 调用显式 command handler，goal family 只写入 `draftText` 等用户确认；带附件或 Skill chip 的 draft 不展示内置命令候选。
5. 选择 `type: "workflow"` 时写入面向模型的 workflow 请求草稿；模型随后根据 init context 的 `workflow_start` / `workflow_describe` / `workflow_list` 工具完成执行，避免客户端 `workflow/start` 控制面缺少 runner-runtime bridge 的问题。
6. 模型 tool path 通过 `EventMsg::WorkflowRunProgressCompleted -> ThreadItem::WorkflowRunProgress` 产生进度 item，root-worker 按 typed item id 渲染为 workflow tool cell。
7. 手动提交 `/goal ...` 由 `parseGoalComposerCommand` 转换为 set/status/clear action，反馈保存在按 `threadId` keyed 的本地 action state。

风险：
- `/goal ...` 依赖 app-server goals feature；如果 feature 关闭，错误来自 typed RPC error 并显示在 goal UI 附近。
- Skills 加载错误状态当前在 App 侧只表现为 `availableSkills=[]`，本次不新增错误行，内置命令仍可用。
- Workflows 加载错误与 Skills 加载共用当前顶部错误状态；本次不新增菜单内错误行。没有 discovered workflows 时菜单只展示 commands/skills。
- Workflow progress 当前只有 run-level 进度；static graph/stage rail 和 thread/agent 所属 badge 需要后端后续把 graph/stage/binding 纳入 typed payload 或 thread metadata。

## 验收

- `slashMenu.test.ts` 覆盖 `/` 检测、命令、workflow 和 Skills 共存、workflows 为空、Skills 为空、过滤、已选 Skill 去重、`/clear` 和 `/goal <subcommand>` family 作为 command 而非 Skill。
- `conversation.test.ts` 覆盖 typed `workflowRunProgress` 进入 conversation tool entry，防止回退到 unsupported item。
- `composerDraft.test.ts` 覆盖手动 `/clear`、`/goal <objective>`、空 `/goal`、精确 `/goal pause|resume|cancel|clear`、`/cancel-goal` 在无附件/Skill 时拦截，以及 `/goal pause this migration` 作为 objective。
- `sendMessagePayload.test.ts` 继续覆盖 Skill payload；手动 slash command 仍由 App 发送前拦截。
