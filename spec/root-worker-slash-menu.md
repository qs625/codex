# Root-worker Composer Slash 菜单

## Brief

用户：root-worker prototype 的桌面客户端用户。

能力：在 composer 输入 `/` 时，同一个候选菜单同时展示当前客户端可执行的内置 slash command 和可用 Skills。选择 Skill 继续沿用现有 chip 与 `turn/start` skill payload；选择内置命令执行客户端已有语义或补全命令 draft，不作为普通用户文本发送。

成功标准：
- 输入 `/` 时展示 `/clear`、`/goal <objective>`、`/goal pause`、`/goal resume`、`/goal cancel` 和未选择的 Skills。
- 输入 `/query` 时按命令 token、说明、alias 以及 Skill name/kind/path 过滤。
- Skills 列表为空时仍展示 `/clear` 和 `/goal <subcommand>` family。
- 选择 Skill 后仍生成 Skill chip，提交 payload 保持 `{ type: "skill", name, path }` 语义。
- 选择 `/clear` 时调用现有 root session clear 语义，不创建普通 turn 文本。
- 选择 `/goal <objective|pause|resume|cancel>` family 时只补全 composer draft；发送前拦截完整命令并调用 typed goal API，不创建普通 turn 文本。
- 手动提交 `/goal <objective>` 时调用 `thread/goal/set` 创建或更新 active goal；手动提交精确 `/goal pause` / `/goal resume` 时调用 `thread/goal/set` 更新 status；手动提交精确 `/goal cancel` / `/goal clear` / `/cancel-goal` 时调用 `thread/goal/clear`。
- 只有完全等于 `/goal pause|resume|cancel|clear` 才按动作处理；例如 `/goal pause this migration` 是 objective。
- 空 `/goal` / `/goal ` 显示 `Enter a goal objective.`，不进入普通 user message。
- `/init` 由 embedded system skill 提供，随 Skills discovery 自动出现在菜单里；root-worker 不为 `/init` 增加 builtin 特判。

非目标：
- 不新增 `/compact` 等当前 root-worker 客户端尚无执行入口的命令。
- 不把可以表达为 Skill 的命令硬编码成 root-worker builtin slash command。
- 不改变 app-server v2 `UserInput::Skill` wire 语义。
- 不修改 TUI slash command 行为。

## 技术设计

实现形态：
- 新增 `src/lib/slashMenu.ts` 承载纯候选逻辑，统一输出 `ComposerSlashSuggestion`。
- `Panels.tsx` 从 Skill 专用 `skillSuggestions` 改为通用 `slashSuggestions`，键盘和鼠标选择复用同一 handler。
- `App.tsx` 提供 `runComposerSlashCommand(commandId)`，`clear` 调用已有 `clearCurrentRootSession()`；goal family 由菜单补全 draft，发送前解析后调用 `thread/goal/set` 或 `thread/goal/clear`。
- `Panels.tsx` 只有在当前 draft 没有 image attachment 和 Skill chip 时启用内置命令候选，避免候选选择路径绕过手动 `/clear` 的保护条件。
- `codex-rs/skills/src/assets/samples/init/SKILL.md` 提供 embedded `init` system skill。`SkillsManager::new` 会安装 bundled system skills 到 `$CODEX_HOME/skills/.system`，`skills/list` 通过现有 roots 加载它，因此未初始化项目也能发现 `/init`。

数据流：
1. composer draft 通过 `getActiveComposerSlashQuery` 判断是否处于 slash token。
2. `buildComposerSlashSuggestions` 合并内置命令与未选择 Skills。
3. 选择 `type: "skill"` 时调用现有 `onAddDraftSkill`，保留 chip/payload。
4. 选择 `type: "command"` 时，`/clear` 调用显式 command handler，goal family 只写入 `draftText` 等用户确认；带附件或 Skill chip 的 draft 不展示内置命令候选。
5. 手动提交 `/goal ...` 由 `parseGoalComposerCommand` 转换为 set/status/clear action，反馈保存在按 `threadId` keyed 的本地 action state。

风险：
- `/goal ...` 依赖 app-server goals feature；如果 feature 关闭，错误来自 typed RPC error 并显示在 goal UI 附近。
- Skills 加载错误状态当前在 App 侧只表现为 `availableSkills=[]`，本次不新增错误行，内置命令仍可用。

## 验收

- `slashMenu.test.ts` 覆盖 `/` 检测、命令和 Skills 共存、Skills 为空、过滤、已选 Skill 去重、`/clear` 和 `/goal <subcommand>` family 作为 command 而非 Skill。
- `composerDraft.test.ts` 覆盖手动 `/clear`、`/goal <objective>`、空 `/goal`、精确 `/goal pause|resume|cancel|clear`、`/cancel-goal` 在无附件/Skill 时拦截，以及 `/goal pause this migration` 作为 objective。
- `sendMessagePayload.test.ts` 继续覆盖 Skill payload；手动 slash command 仍由 App 发送前拦截。
