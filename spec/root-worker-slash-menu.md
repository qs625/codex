# Root-worker Composer Slash 菜单

## Brief

用户：root-worker prototype 的桌面客户端用户。

能力：在 composer 输入 `/` 时，同一个候选菜单同时展示当前客户端可执行的内置 slash command 和可用 Skills。选择 Skill 继续沿用现有 chip 与 `turn/start` skill payload；选择内置命令执行客户端已有语义，不作为普通用户文本发送。

成功标准：
- 输入 `/` 时展示 `/clear` 和未选择的 Skills。
- 输入 `/query` 时按命令 token、说明、alias 以及 Skill name/kind/path 过滤。
- Skills 列表为空时仍展示 `/clear`。
- 选择 Skill 后仍生成 Skill chip，提交 payload 保持 `{ type: "skill", name, path }` 语义。
- 选择 `/clear` 时调用现有 root session clear 语义，不创建普通 turn 文本。

非目标：
- 不新增 `/compact` 等当前 root-worker 客户端尚无执行入口的命令。
- 不改变 app-server v2 `UserInput::Skill` wire 语义。
- 不修改 TUI slash command 行为。

## 技术设计

实现形态：
- 新增 `src/lib/slashMenu.ts` 承载纯候选逻辑，统一输出 `ComposerSlashSuggestion`。
- `Panels.tsx` 从 Skill 专用 `skillSuggestions` 改为通用 `slashSuggestions`，键盘和鼠标选择复用同一 handler。
- `App.tsx` 提供 `runComposerSlashCommand(commandId)`，当前只处理 `clear`，直接调用已有 `clearCurrentRootSession()`。
- `Panels.tsx` 只有在当前 draft 没有 image attachment 和 Skill chip 时启用内置命令候选，避免候选选择路径绕过手动 `/clear` 的保护条件。

数据流：
1. composer draft 通过 `getActiveComposerSlashQuery` 判断是否处于 slash token。
2. `buildComposerSlashSuggestions` 合并内置命令与未选择 Skills。
3. 选择 `type: "skill"` 时调用现有 `onAddDraftSkill`，保留 chip/payload。
4. 选择 `type: "command"` 时调用显式 command handler，不进入 `buildSendMessagePayload`；带附件或 Skill chip 的 draft 不展示内置命令候选。

风险：
- 当前只知道 `/clear` 有客户端执行语义；新增命令必须先补 App/Electron/app-server 能力，再加入候选。
- Skills 加载错误状态当前在 App 侧只表现为 `availableSkills=[]`，本次不新增错误行，内置命令仍可用。

## 验收

- `slashMenu.test.ts` 覆盖 `/` 检测、命令和 Skills 共存、Skills 为空、过滤、已选 Skill 去重、`/clear` 作为 command 而非 Skill。
- `sendMessagePayload.test.ts` 继续覆盖 Skill payload；手动 `/clear` 仍由 `isClearComposerCommand` 在 App 发送前拦截。
