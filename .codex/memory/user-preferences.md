# User Preferences

## Stable Preferences
- 全程使用中文进行工作和记录。
- 所有 shell 命令必须以 `rtk` 开头。
- 普通开发应先在对应 `dev` checkout 提交，再 merge 回主分支。
- 不要把 `dev` checkout 的改动文件手工复制、覆盖或 apply 回主仓库代替 merge。
- 固定 owner 空闲时不要主动关闭；后续同一 checkout 优先直接续用 `followup_task`，只有 thread 不可用时才重建。
- 固定 owner 的 reviewer 也应长期复用：每个 owner 使用同一个 `<owner>/reviewer` child，不要每个任务或每轮 review 新建 reviewer。
- 对 subagent 交互不要频繁查看状态或发送催促；大多数情况下派发后等待 subagent 完成通知即可，除非用户明确询问状态、存在超时/阻塞风险，或需要处理已到达的完成通知。
- 我们自己的 agent/runtime 产品名定为 Morpheus；外部官方 Codex provider 仍称 `codex_cli` / external Codex CLI provider。
- 我们自己的配置 home 入口应使用 Morpheus 命名；不要把新的配置目录环境变量命名为 `CODEX_HOME`。
- 代码 crate、模块、变量名默认使用语义名，除非明确表达产品本身语义，否则不要带 Morpheus/Codex 等产品名。
- ThreadProvider / agent provider 设计中 external agent 和内置 agent 都应作为一等公民平等对待；遇到能力不对等时，默认补齐 provider-neutral runtime 语义，而不是通过隐藏 external 工具或降低 external 能力来表面对齐。
- 讨论或推进多个设计方向时，应把每个设计都作为一等公民平等对待；不要默认把某个设计降级为临时、次等或只能被隐藏的路径。
- workflow JS 脚本等待 agent 完成时应使用语义化 `await agent.wait()`；`poll_event` 是 agent 内部等待事件的 tool，`wf.pollEvent()` 只作为低层/advanced API 保留，不作为普通脚本的推荐等待入口。

## Working Style
- 优先直接修改代码或文档，不要只停留在分析。
- 对当前项目的 PM/owner 协作规则，应遵循 `.codex/agents/project-pm.agent.md`。
- 工作过程中如果识别到新的稳定用户偏好或长期项目事实，应分别更新 `.codex/memory/user-preferences.md` 和 `.codex/memory/project-understanding.md`。
- 派发给 owner 的任务 brief 不要只写目标和范围；要写完整设计意图、状态机约束、不变量、预期实现轮廓、禁止路径和验收测试矩阵，避免 owner 自行补全关键设计。
- owner 完成后，先按设计验收，再看测试验收；如果实现没有按 brief 的设计完成，即使测试通过也要返工。
