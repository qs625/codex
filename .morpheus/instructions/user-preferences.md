# User Preferences

## Stable Preferences
- Morpheus 自身源码的主 checkout 固定为 `~/.morpheus/source_workspace`；普通开发 worktree 固定使用相邻的 `source_workspace-dev`、`source_workspace-dev-2`、`source_workspace-dev-3`，后续构建也从这套 checkout 执行，不再使用 `~/Projects/my-codex*`。
- 产品改动要选择合适的构建重启时机：重大 bugfix、重大 feature、Launcher/runtime/安装恢复链路改动，或需要真实安装态验收的修改，应从 `~/.morpheus/source_workspace` 立即构建 Launcher 期望的完整 Runtime Capsule并 full restart；低风险小修复不必每次单独重启，可以记录后与后续改动批量交付。必须区分“已 merge”和“已安装生效”，并持续记录待交付 commit 与当前 installed release。纯文档/协作规则修改无需构建重启。
- 普通 dev checkout 的 owner 阶段不要求 release 构建；涉及 Rust/app-server/backend 启动路径时，owner 只需跑相关 focused tests 和 debug 后端编译（例如 `cargo build --manifest-path codex-rs/Cargo.toml -p app-server --bin app-server`）。release build、完整 Runtime Capsule 构建、full restart 和安装态 self-debug 验证由 PM 在合并 canonical main 后负责。
- 任何涉及 Morpheus 客户端、前端 UI、Browser/Terminal 面板、Playwright/CDP/frontend-debug 调试链路，或用户可见安装态行为的修复，在 Runtime Capsule 重启安装生效后，都应使用项目 `self-debug` skill（配合 `frontend-debug` 和 `playwright-cli` 连接当前客户端 CDP）调试当前运行客户端自己做安装态验收；如果 self-debug 发现问题，应继续修改，不要把“已 merge / 已构建 / 已重启”当成完成。
- 预期 Runtime Capsule 重启恢复提示只应防止为了同一个已完成 restart 请求连续重复调用 `request_runtime_restart`；如果后续又完成新的代码修改或需要交付新的 Runtime Capsule，应按正常构建交付规则继续调用 restart。
- 全程使用中文进行工作和记录。
- 普通开发应先在对应 `dev` checkout 提交，再 merge 回主分支。
- 不要把 `dev` checkout 的改动文件手工复制、覆盖或 apply 回主仓库代替 merge。
- 固定 owner 空闲时不要主动关闭；后续同一 checkout 优先直接续用 `followup_task`，只有 thread 不可用时才重建。
- 固定 owner 的 reviewer 也应长期复用：每个 owner 使用同一个 `<owner>/reviewer` child，不要每个任务或每轮 review 新建 reviewer。
- 对 subagent 交互不要频繁查看状态或发送催促；大多数情况下派发后等待 subagent 完成通知即可，除非用户明确询问状态、存在超时/阻塞风险，或需要处理已到达的完成通知。
- `.codex/pm-progress.md` 只保留最近半个月左右的活跃/近期进度和当前约束；更早的完成记录和过期 known issues 归档到 `.codex/pm-progress-archive/`，通过目录 index 管理。
- 我们自己的 agent/runtime 产品名定为 Morpheus；外部官方 Codex provider 仍称 `codex_cli` / external Codex CLI provider。
- 我们自己的配置 home 入口应使用 Morpheus 命名；不要把新的配置目录环境变量命名为 `CODEX_HOME`。
- 代码 crate、模块、变量名默认使用语义名，除非明确表达产品本身语义，否则不要带 Morpheus/Codex 等产品名。
- ThreadProvider / agent provider 设计中 external agent 和内置 agent 都应作为一等公民平等对待；遇到能力不对等时，默认补齐 provider-neutral runtime 语义，而不是通过隐藏 external 工具或降低 external 能力来表面对齐。
- 讨论或推进多个设计方向时，应把每个设计都作为一等公民平等对待；不要默认把某个设计降级为临时、次等或只能被隐藏的路径。
- workflow JS 脚本等待 agent 完成时应使用语义化 `await agent.wait()`；`poll_event` 是 agent 内部等待事件的 tool，`wf.pollEvent()` 只作为低层/advanced API 保留，不作为普通脚本的推荐等待入口。

## Working Style
- 优先直接修改代码或文档，不要只停留在分析。
- 对当前项目的 PM/owner 协作规则，应遵循 `.morpheus/agents/project-pm.agent.md`。
- 工作过程中如果识别到新的稳定用户偏好或长期项目事实，应分别更新 `.morpheus/instructions/user-preferences.md` 和 `.morpheus/instructions/project-understanding.md`。
- 以后开发新功能前要先说清楚实现方式；如果需求、语义边界或实现细节有不清楚的地方，先问清楚再推进。
- 派发给 owner 的任务 brief 不要只写目标和范围；要写完整设计意图、状态机约束、不变量、预期实现轮廓、禁止路径和验收测试矩阵，避免 owner 自行补全关键设计。
- owner 完成后，先按设计验收，再看测试验收；如果实现没有按 brief 的设计完成，即使测试通过也要返工。
