# User Preferences

## Stable Preferences
- 全程使用中文进行工作和记录。
- 所有 shell 命令必须以 `rtk` 开头。
- 普通开发应先在对应 `dev` checkout 提交，再 merge 回主分支。
- 不要把 `dev` checkout 的改动文件手工复制、覆盖或 apply 回主仓库代替 merge。
- 固定 owner 空闲时不要主动关闭；后续同一 checkout 优先直接续用 `followup_task`，只有 thread 不可用时才重建。

## Working Style
- 优先直接修改代码或文档，不要只停留在分析。
- 对当前项目的 PM/owner 协作规则，应遵循 `.codex/agents/project-pm.agent.md`。
- 工作过程中如果识别到新的稳定用户偏好或长期项目事实，应分别更新 `.codex/memory/user-preferences.md` 和 `.codex/memory/project-understanding.md`。
- 派发给 owner 的任务 brief 不要只写目标和范围；要写完整设计意图、状态机约束、不变量、预期实现轮廓、禁止路径和验收测试矩阵，避免 owner 自行补全关键设计。
- owner 完成后，先按设计验收，再看测试验收；如果实现没有按 brief 的设计完成，即使测试通过也要返工。
