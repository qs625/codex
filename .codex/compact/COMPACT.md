你正在为 my-codex 项目执行 CONTEXT CHECKPOINT COMPACTION。

目标不是生成一份泛化的 handoff summary，而是维护当前项目的 3 个 memory 文件，使后续模型在多轮 compact 之后仍然保留稳定的项目事实、用户偏好与当前工作现场。

你必须维护这些文件：

- `.codex/memory/user-preferences.md`
- `.codex/memory/project-understanding.md`
- `.codex/memory/current-work.md`

## 一、总原则

- 优先更新 memory 文件，不要把重要工作记忆只留在 compact 摘要正文里。
- 只保留对后续继续工作真正有用的信息，不要写流水账。
- 记“结论、状态、文件地图、下一步”，不要记大段代码、长日志或临时推理。
- 如果某些旧内容已过时，必须直接删除或改写，不要只追加新内容。
- 如果当前上下文不足以可靠更新某条 memory，就保持原状，不要编造。

## 二、3 个 memory 文件的职责

### 1. `.codex/memory/user-preferences.md`

只记录跨任务稳定成立的用户偏好与协作要求，例如：

- 语言偏好
- 命令执行偏好
- 分支/合并偏好
- checkout/PM/owner 协作偏好
- 明确禁止的做法

不要记录：

- 当前任务的临时状态
- 架构理解
- 一次性命令结果

### 2. `.codex/memory/project-understanding.md`

这是当前项目的唯一事实来源。

只记录跨任务长期有效、并且后续模型必须统一依赖的项目事实，例如：

- 长期稳定的工作规则
- 长期稳定的架构边界
- 关键模块地图
- 已确认的系统约束
- 默认验证策略
- 已明确否定的长期方案或禁止路径

后续模型对“这个项目应该怎么工作、哪些边界必须遵守、哪些模块负责什么”的判断，必须以这份文件为准，而不是依赖 `AGENTS.md`、旧 compact 摘要或分散在别处的零散规则。

不要把这些事实拆散到其他 memory 文件中。

不要记录：

- 当前任务现场
- 短期 debugging 假设
- 近期细碎操作历史

### 3. `.codex/memory/current-work.md`

记录当前工作现场，重点是让后续模型不要重复探索，至少维护：

- 当前目标
- 当前状态
- 最近关键进展
- 已读哪些文件
- 每个已读文件为什么相关
- 从这些文件中得出的关键结论
- 哪些文件大概率可以跳过
- 当前阻塞
- 下一步做什么

这份文件是 compact 时最常更新的文件。

## 三、严格的文件大小限制

必须严格控制 3 个 memory 文件的大小，避免膨胀。

硬限制：

- `user-preferences.md`：不超过 120 行
- `project-understanding.md`：不超过 220 行
- `current-work.md`：不超过 220 行

软限制：

- 每个一级 section 尽量不超过 30 行
- 单条 bullet 尽量不超过 3 行
- 不保留重复表达；新信息如果覆盖旧信息，直接改写旧项

如果即将超限，必须优先做这些事：

1. 删除过时内容
2. 合并重复结论
3. 把长段落压缩成短 bullet
4. 只保留对继续工作最关键的信息

不要为了“保留完整历史”而突破限制。

## 四、更新规则

### 更新 `user-preferences.md`

只有在当前上下文中出现新的、明确的、跨任务稳定成立的用户偏好时才更新。

### 更新 `project-understanding.md`

只有在当前上下文中确认了新的长期项目事实时才更新。包括：

- 新的稳定工作规则
- 新的架构边界
- 新的模块职责理解
- 已确认的验证默认值
- 明确否定的长期方案

如果某条旧事实已不再成立，必须直接改写或删除旧项。

### 更新 `current-work.md`

每次 compact 都应检查并更新，尤其是：

- 当前目标是否变化
- 最近完成了什么
- 已读文件列表是否需要补充或精简
- 哪些方向已经证伪
- 下一步动作是否已改变

## 五、代码任务的特殊要求

对 coding task，必须优先维护“探索索引”，避免后续模型重复读文件。

在 `current-work.md` 中，已读文件至少记录：

- 文件路径
- 为什么读它
- 关键结论
- 是否还需要再看

不要保存：

- 大段代码正文
- 大量行号细节
- 可以重新快速定位的一次性输出

## 六、compact 的输出内容

在完成 memory 文件更新后，再生成一份简洁 handoff summary。

handoff summary 只需要包含：

- 当前 progress
- 本次更新了哪些 memory 文件
- 仍然未解决的问题
- 下一步最应该做什么

不要把 memory 文件内容完整重复到 handoff summary 中。

## 七、当前项目的 memory 边界

这 3 个文件的边界固定如下：

- `user-preferences.md`
  只放用户偏好
- `project-understanding.md`
  放项目唯一事实来源，包括长期工作规则、架构边界、模块地图、默认验证策略、禁止路径
- `current-work.md`
  只放当前任务现场

如果某条信息同时像“项目事实”和“当前任务结论”，优先判断它是否跨任务长期成立：

- 跨任务长期成立 -> 放 `project-understanding.md`
- 只对当前任务现场有用 -> 放 `current-work.md`

## 八、多 worktree / 多 agent 维护规则

当前项目存在多个 worktree 和多个长期 owner，因此这 3 个 memory 文件不能都按“每个 agent 各写一份”处理。

### 全局共享、单一来源

以下文件是全项目共享的 canonical memory，只允许 PM 在主仓库维护：

- `.codex/memory/user-preferences.md`
- `.codex/memory/project-understanding.md`

规则：

- 所有 agent 都读取这两份文件。
- 只有 PM 才能把新的内容正式写入这两份 canonical 文件。
- owner 不应把自己的局部理解直接提升成全项目事实。

### worktree 本地维护

以下文件是当前工作现场，只由各自 worktree / owner 维护：

- `<worktree>/.codex/memory/current-work.md`

规则：

- 每个 worktree 只维护自己的 `current-work.md`。
- 不同 owner 的当前任务现场不能混写到同一份 `current-work.md`。
- `current-work.md` 记录已读文件、当前目标、阻塞和下一步，天然属于局部状态。

### owner 发现新的长期项目事实时

如果 owner 在自己的任务中发现了新的长期项目事实：

1. 先写入自己 worktree 的 `current-work.md`
2. 在交付中明确提出“建议更新 `project-understanding.md`”
3. 由 PM 在主仓库统一吸收并改写 canonical `project-understanding.md`
4. 再按现有同步规则传播到其他空闲 worktree

不要让多个 owner 在不同 worktree 中并行修改 `project-understanding.md` 并各自保留不同版本。
