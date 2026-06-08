# 设计 Review

## 独立 Review 记录

已委派独立 `@ui-ue-reviewer` 完成首轮 review。首轮结论为“需修正后复审”，整体方向符合用户要求，问题集中在文档一致性与 handoff 准确性。

## 首轮问题与处理

1. README 旧资产引用
   - 问题：README 仍引用已移除的 HTML 原型和旧 baseline。
   - 处理：已改为当前三张实际资产：`baseline-current-app-clean.png`、`run-config-current-app-modification.png`、`run-config-component-states.png`。

2. 文案真源不一致
   - 问题：README 保留英文文案，和 `04-components.md` 中文文案清单冲突。
   - 处理：README 已统一为中文，并以 `04-components.md` 文案清单为实现真源。

3. 关闭行为范围冲突
   - 问题：README 将 outside click / Escape 关闭列为后续风险，但 UE 与可访问性文档将其列为首版行为。
   - 处理：outside click、Escape、点击“取消”关闭和焦点返回 trigger 已统一为首版要求。

4. Handoff 未同步当前实现
   - 问题：文档仍按未暴露 `model/list` IPC、未存在 picker 的状态描述。
   - 处理：`01-research.md` 与 `04-components.md` 已同步当前 worktree 现状：已有 `RunConfigPicker`、`codex:listModels` 和 `codex:setThreadRunConfig` 初版实现，开发 handoff 改为基于现有组件补齐。

## 当前结论

复审已发起，等待独立 reviewer 最终结论。

设计交付当前满足以下要求：

- 原型图基于当前应用截图，只展示 header 入口与 popover 的功能修改。
- 另有只包含运行配置相关组件的状态图。
- 不再交付 HTML prototype 或完整想象应用图。
- 交互状态、文案、可访问性和开发 handoff 已落文档。

## 复审最终结论

状态：
复审通过

发现：
无阻塞问题。

复审确认：
1. README 旧资产引用已修正：当前只列出 `baseline-current-app-clean.png`、`run-config-current-app-modification.png`、`run-config-component-states.png` 三张实际资产，未继续引用旧 HTML prototype 或旧 baseline。
2. 文案真源不一致已修正：README、UE flow、信息架构与组件 handoff 均使用中文文案，`04-components.md` 保持为开发实现文案真源。
3. 关闭行为范围冲突已修正：outside click、Escape、点击“取消”关闭，以及关闭后焦点回到 trigger 已统一为首版可访问性要求。
4. Handoff 未同步当前实现已修正：`01-research.md` 与 `04-components.md` 均已说明当前 worktree 存在 `RunConfigPicker`、`codex:listModels`、`codex:setThreadRunConfig` 初版实现，后续开发应基于现有组件补齐状态、文案、可访问性与视觉密度。

非阻塞建议：
无

开放问题：
无

结论：
首次 review 的四项问题均已修正，可进入开发。
