# Compact Memory Service 设计

## 背景

当前 Local Compact 主流程长期把 prompt 拼装、memory 文件读写、replacement history 组装和 auto compact 判定都堆在 `thread-service` 内，导致：

- compact 逻辑和 session/runtime 强耦合，不利于继续把 compact 从重 runtime 中拆出去。
- compact 产物仍偏向 summary-only，无法稳定利用 worktree 本地 `current-work.md` 与 shared memory。
- auto compact 只能依赖硬阈值，缺少对 recent progress、tool 输出体量和 current-work 完整度的判断。

## 目标

- 新增 owner crate，把 compact 纯逻辑收敛到独立 service。
- 让 compact 输出转为结构化 JSON，并稳定写入 worktree 本地 `current-work.md`。
- replacement history 从 summary-only 调整为 memory-backed checkpoint。
- 在 hard threshold 之外补一个 soft compact MVP 决策层。

## 非目标

- 本次不把 canonical shared memory 自动发现做成全局完备方案。
- 本次不写 canonical `project-understanding.md` / `user-preferences.md`。
- 本次不移除 `thread-service` 中保留给旧测试和旧回放路径的兼容 helper。

## 设计

### crate 边界

- `compact-service-api`
  - 定义 `CompactMemoryLayout`、`CompactMemoryBundle`、`CompactModelOutput`、`CompactPromptSpec`、`ReplacementHistoryInput`、`SoftCompactInputs` 等 DTO。
- `compact-service`
  - 提供 `FsCompactService`，负责 memory layout 推导、memory 文件读取、prompt 组装、模型输出解析/落盘、replacement history 构造和 soft compact 决策。
- `thread-service`
  - 保留 compact turn 生命周期、history replace、event 发射、provider streaming 与 session 侧依赖注入。

### memory root 策略

- `worktree_memory_root` 固定为 `cwd/.codex/memory`。
- `shared_memory_root` 采用最小保守策略：
  - 如果 `cwd/.codex/compact/COMPACT.md` 内容与当前 compact prompt 一致，则 shared root 指向 `cwd/.codex/memory`。
  - 如果命中 `codex_home/compact/COMPACT.md`，视为只有 home 级 prompt，不写 shared memory。
  - 其他情况 shared root 为 `None`。
- 当 shared root 不可解析时，compact 仍继续执行，只维护本地 `current-work.md`。

### prompt / output contract

- compact prompt 在原 `COMPACT.md` 后追加一个结构化 `Runtime Memory Bundle`：
  - shared user preferences
  - shared project understanding
  - local current work
- memory 文件在读入 bundle 时先按 token budget 截断，避免 compact 自己重新引入无界上下文。
- 模型输出改为严格 JSON schema，字段包括：
  - `current_work`
  - `shared_fact_candidates`
  - `handoff_summary`

### replacement history

- compact 后 replacement history 不再只保留 summary 文本。
- 新 history 顺序为：
  - 最近少量真实用户消息
  - compact marker
  - `Memory checkpoint: user preferences`
  - `Memory checkpoint: project understanding`
  - `Memory checkpoint: current work`
- mid-turn compact 的 initial context 改为整体前置到 memory-backed checkpoint 块之前，避免把 checkpoint 块从中间拆开。

### soft compact MVP

- hard compact: `usage_ratio >= 0.85`
- soft compact window: `0.70..0.85`
- 决策参考：
  - `turns_since_last_compact`
  - `recent_file_read_search_count`
  - `recent_tool_output_bytes`
  - `current_work_completeness`
  - cooldown 是否满足
- memory checkpoint 消息不计入“真实用户进展”，避免 compact 后被 checkpoint 噪音误触发下一次 soft compact。

## 风险

- shared root 仍依赖 prompt 文本匹配，跨 worktree / canonical root 自动发现还不完备。
- replacement history 现在是 memory-backed user messages，后续如果 rollout/replay 引入更强约束，需要再统一 typed memory checkpoint 表达。
- `shared_fact_candidates` 当前只作为模型建议，不会写回 canonical shared memory。
