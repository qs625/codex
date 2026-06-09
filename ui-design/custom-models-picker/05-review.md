# 设计 Review

## Review 状态

已完成独立 `ui-ue-reviewer` review，复审通过。

## Review 结论

- 初审结论：通过，无阻塞问题。
- 复审发现：provider 身份与 apply payload 可能不一致；可访问性 handoff 不足。
- 复审结论：通过，可进入开发 handoff。
- 开发决策：本次不扩展 provider-aware apply selection；只保证当前 configured model 可见且不会被打开 picker 覆盖。

## 设计自检

- 已覆盖 catalog model、configured custom model、当前 model 不在 catalog。
- 已明确 provider 名、`Configured` 标记和 `Current` 标记。
- 已明确打开 picker 不自动回退。
- 已覆盖 loading、error、empty、running、fallback。
- 已提供低保真状态原型图。

## Review 建议处理

- 已补充：`current` synthetic item 不能单独让 `应用` 进入可用状态；只有选择 catalog/configured item 后才允许应用。
- 已补充：同名不同 provider 的精确切换不是本次实现范围，当前 apply payload 只携带 `model` 与 `reasoningEffort`。
- 已补充：model option accessible label、focus 返回、radio 键盘行为、`aria-live` 状态朗读和禁用原因。
- 已补充：错误详情单独展示，不直接拼在中文句号后；长文本截断优先级。

## 剩余建议

- 后续如果要支持同名不同 provider 的精确选择，需要扩展 thread/run config 保存 provider identity。
