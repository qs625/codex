# Compact Memory Service 设计

## 背景

当前 Local Compact 主流程长期把 compact prompt 解释、memory 文件读写、replacement history 组装和 auto compact 判定都堆在 `thread-service` 内，导致：

- compact 逻辑和 session/runtime 强耦合，不利于继续把 compact 从重 runtime 中拆出去。
- compact runtime 既决定模型该如何 compact，又解释模型输出，边界过重。
- auto compact 只能依赖硬阈值，缺少对 recent progress、tool 输出体量和 current-work 完整度的判断。

## 目标

- 新增 owner crate，把 compact 纯逻辑收敛到独立 service。
- 让 `COMPACT.md` 独立负责 compact 行为规则；runtime 只负责触发 compact turn、回读 memory 文件和替换上下文。
- replacement history 从 summary-only 调整为 memory-backed checkpoint。
- 在 hard threshold 之外补一个 soft compact MVP 决策层。

## 非目标

- 本次不把 canonical shared memory 自动发现做成全局完备方案。
- 本次不让 runtime 解析 compact 结构化输出，也不由 runtime 渲染/写入 `current-work.md`。
- 本次不移除 `thread-service` 中保留给旧测试和旧回放路径的兼容 helper。

## 设计

### crate 边界

- `compact-service-api`
  - 定义 compact replacement file、memory snapshot、replacement history 和 soft compact 所需 DTO。
- `compact-service`
  - 提供 `FsCompactService`，负责 replacement files 回读、token capped 文件读取、replacement history 构造和 soft compact 决策。
- `thread-service`
  - 保留 compact turn 生命周期、原始 `COMPACT.md` prompt 注入、history replace、event 发射、provider streaming 与 session 侧依赖注入。

### replacement file 配置

- replacement files 放进现有 `[memories]` `config.toml` 体系，不新增单独 compact 配置文件。
- 配置项支持：
  - 文件列表
  - 每个文件的语义 role
  - 全局 token cap
  - 每个文件单独 token cap 覆盖
- 当用户未显式配置时，runtime config 会生成默认 replacement files：
  - `cwd/.codex/memory/current-work.md`
  - `cwd/.codex/memory/project-understanding.md`
  - `cwd/.codex/memory/user-preferences.md`

### compact turn 边界

- compact turn 只把 `COMPACT.md` 作为 prompt 注入，不再由 runtime 拼装 memory bundle。
- compact turn 不要求结构化输出，不设置 output schema，也不解析模型输出 JSON。
- memory 文件由模型在 compact turn 内自行读取/修改；runtime 不再写入 `current-work.md` 或 shared memory 文件。

### replacement history

- compact 后 replacement history 不再只保留 summary 文本。
- 新 history 顺序为：
  - 最近少量真实用户消息
  - compact marker
  - 配置指定 replacement files 的 memory checkpoint
- mid-turn compact 的 initial context 改为整体前置到 memory-backed checkpoint 块之前，避免把 checkpoint 块从中间拆开。
- compact 结束后，runtime 只回读配置指定的 files，并据此重建 replacement history。

### soft compact MVP

- hard compact: `usage_ratio >= 0.85`
- soft compact window: `0.70..0.85`
- 决策参考：
  - `turns_since_last_compact`
  - `recent_file_read_search_count`
  - `recent_tool_output_bytes`
  - `current_work_completeness`
  - cooldown 是否满足
- `current_work_completeness` 只在实际回读到了 `current-work` snapshot 时生效；缺文件或空文件按 neutral 处理，不单独触发 soft compact。
- memory checkpoint 消息不计入“真实用户进展”，避免 compact 后被 checkpoint 噪音误触发下一次 soft compact。

## 风险

- replacement history 仍然使用 user-message 形式的 memory checkpoint，后续如果 rollout/replay 需要更强约束，仍可能需要 typed memory checkpoint 表达。
- 默认 replacement files 仍是 runtime config 给出的 convention；如果项目需要别的 memory 文件集，必须通过 `[memories]` 显式覆盖。
