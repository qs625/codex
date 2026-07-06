# Current Work

## Current Goal
- 维护和整理当前项目的 agent / compact / memory 规则，使多轮 compact 后仍能稳定恢复工作现场。

## Current Status
- 状态：in_progress
- 当前仓库：`/Users/bytedance/Projects/my-codex`
- 当前重点：规则整理与 compact memory 设计

## Recent Progress
- 已整理 `project-pm.agent.md`，按分类重写规则，并明确 dev checkout 提交后再 merge 回主分支。
- 已整理 `feature-owner.agent.md`、`refactor-owner.agent.md`、`performance-owner.agent.md`，去掉与 PM 规则冲突的内容。
- 已确认 compact prompt 的优先级：
  - 显式 override
  - `experimental_compact_prompt_file`
  - `cwd/.codex/compact/COMPACT.md`
  - `CODEX_HOME/compact/COMPACT.md`
  - 内置 prompt fallback
- 已在 `CODEX_HOME` 下写入默认 `COMPACT.md`。

## Files Already Read
- `.codex/agents/project-pm.agent.md`
  - 原因：整理 PM 协作规则
  - 结论：当前规则已改成固定 checkout / 固定 owner / dev 提交后 merge 主线
  - 是否还需再看：如果继续改 PM 协作规则，需要回看
- `.codex/agents/feature-owner.agent.md`
  - 原因：检查是否违背 PM 规则
  - 结论：已重写为统一分类结构，并去掉默认 `spec/` 要求
  - 是否还需再看：短期内通常不需要
- `.codex/agents/refactor-owner.agent.md`
  - 原因：检查是否违背 PM 规则
  - 结论：已重写为统一分类结构，与 PM 规则对齐
  - 是否还需再看：短期内通常不需要
- `.codex/agents/performance-owner.agent.md`
  - 原因：检查是否违背 PM 规则
  - 结论：已重写为统一分类结构，与 PM 规则对齐
  - 是否还需再看：短期内通常不需要
- `codex-rs/config/src/runtime/load_config.rs`
  - 原因：确认 compact prompt 来源与优先级
  - 结论：workspace `COMPACT.md` 优先于 `CODEX_HOME`，两者都没有时配置层 `compact_prompt` 为 `None`
  - 是否还需再看：如果继续改 compact prompt 加载逻辑，需要回看
- `codex-rs/thread-service/src/session/turn_context.rs`
  - 原因：确认没有自定义 prompt 时的最终 fallback
  - 结论：`compact_prompt()` 最终回退到内置 `SUMMARIZATION_PROMPT`
  - 是否还需再看：如果改 compact runtime，需要回看
- `codex-rs/thread-service/src/compact.rs`
  - 原因：确认 compact 运行入口和内置 prompt 常量
  - 结论：manual / auto compact 都走这里，内置 compact prompt 仍存在
  - 是否还需再看：如果改 compact runtime，需要回看
- `apps/root-worker-prototype/src/App.tsx`
  - 原因：确认 compact history 客户端行为
  - 结论：compact details 按需加载，折叠后丢弃，再展开重新加载
  - 是否还需再看：如果改 compact UI 状态机，需要回看
- `apps/root-worker-prototype/src/components/Conversation.tsx`
  - 原因：确认 compact UI 展示方式
  - 结论：展开后按 compact round 分组展示 archived conversation 和 compacted context
  - 是否还需再看：如果改 compact 交互文案或分组展示，需要回看

## Key Findings
- 当前 compact 更像单轮 handoff summary，不像长期工作记忆。
- 对当前项目，更适合把 memory 拆成：
  - `user-preferences.md`
  - `project-understanding.md`
  - `current-work.md`
- `current-work.md` 必须重点记录已读文件及关键结论，避免 compact 后重复读文件。
- memory 文件必须严格限长，避免再次膨胀成新的上下文负担。

## Blockers
- 暂无明确阻塞。

## Next Steps
- 如果继续推进 compact 设计，下一步应决定：
  - compact runtime 是否只维护这 3 个 memory 文件
  - 更新逻辑由模型直接生成整文件，还是生成 patch/增量更新
  - 后续 turn 注入时如何读取并拼接这 3 个 memory 文件
