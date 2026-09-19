# PM Progress

## Index
- [Current Goal](#current-goal)
- [Active Work](#active-work)
- [Recent Completed](#recent-completed)
- [PM Progress Archive](pm-progress-archive/index.md)
- [Known Issues](#known-issues)

## Current Goal
Current installed Runtime Capsule is `sha256:bb7f0bab0d56b639995f6ead7d38441c26d3f212dcb590e9fdfca40ed6a033f3`, sourceCommit `53717f8991d08022f6db1d14c4f1a0b3e21c1bc3`, installed by recovered restart request `call_TJBnSBkBADXsvums2TQlSK3F` (do not repeat the same restart request). Runtime control selected/externalCurrent point at `bb7f0bab...`; previous is `sha256:c1ea6473408c0a7497287f50292f55738cddfd4c517f978a95bf6759e9f62741`. Running payload/app-server processes execute from `bb7f0bab...`. Self-debug attached to CDP `127.0.0.1:9223`: renderer URL points to `bb7f0bab.../app.asar/dist/index.html`, readyState complete, Electron UA confirmed, console 0 errors/warnings. Stable Computer Use helper was not rewritten by the `bb7f0bab...` UI delivery: mtime remains `2026-09-19 18:13:05`, inode `193655211`, CDHash `92a58a08293a2db3bb4cffc0f22297b7ca09d70c`.

Current user request queued next: optimize Runtime Capsule delivery so ordinary Runtime/frontend/backend changes can build/install only the Capsule and skip recompiling Launcher and building DMG when Launcher/outer app did not change.

## Active Work
- id: compact-marker-non-expandable-ui
  owner: /self/owner_dev_3
  reviewer: /self/owner_dev_3/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev-3
  branch: fix/compact-marker-non-expandable-ui
  task_type: ui-bugfix/conversation
  depends_on: user correction on 2026-09-19 CST: “compact summary 还是没改啊 / 现在还是折叠起来的 ui”. Previous `compact_marker_hide_summary` only removed duplicate summary inside replacement history and did not remove the expandable compact row UI.
  files: likely `apps/root-worker-prototype/src/components/Conversation.tsx`, `apps/root-worker-prototype/src/components/Conversation.test.tsx`, possibly compact/search/detail helpers if needed.
  base_commit: `b612853af`
  status: merged_pending_capsule_delivery
  objective: Make context compaction render as a small non-expandable boundary marker in the active conversation UI, not an expandable/collapsible summary card.
  design_intent: Compact is an internal context-management boundary, not content the user should be invited to read inline. The chat timeline should remain clean after compaction while preserving durable audit facts off the default surface.
  invariants: Do not delete persisted compact facts, replacement history, archived history, search/detail extraction, or reload semantics; do not hide later user/assistant messages after the compact boundary; do not regress active command/orphan item handling around compaction.
  forbidden_paths: no CSS-only hiding that leaves keyboard/screen-reader expandable controls; no removing backend/thread history facts; no reintroducing summary body/preview under another label; no broad redesign of conversation rows.
  expected_implementation_outline: Remove or gate the compact row `<details>`/summary expansion affordance from active conversation rendering, update component tests that currently expect expandable summary markup, and keep compact row as “Context compacted” marker with bounded metadata only if non-interactive.
  minimum_regression_matrix: compact summary present + replacement history missing; replacement history available; large summary payload; archived artifacts/history; nested compact archived history; same-turn pre/post compact user messages; search/detail helpers still work if they are intentionally retained; focused component/lib tests and diff check; reviewer approval.
  owner_result: commit `c64e67639` removes compact summary/details rendering from active Conversation UI, keeps compact facts in lib/detail paths, focused tests `pnpm --dir apps/root-worker-prototype exec tsx --test src/components/Conversation.test.tsx src/lib/conversation.test.ts` 114/114, `git diff --check` pass, reviewer approved.
  merge_commit: `33aaff9cc`
  pm_validation: PM inspected implementation against UI contract; main `pnpm --dir apps/root-worker-prototype exec tsx --test src/components/Conversation.test.tsx src/lib/conversation.test.ts` 114/114; diff check passed before merge.
  next_action: include in next Runtime Capsule delivery, preferably batched with `split-init-context-display`.
  blockers: none.

- id: split-init-context-display
  owner: /self/owner_dev_3
  reviewer: /self/owner_dev_3/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev-3
  branch: fix/split-init-context-display
  task_type: ui-data-model/conversation-init-context
  depends_on: user correction on 2026-09-19 CST: “然后拆分 init context 也没做”. Related to compact/replacement history display but broader than removing compact summary folding.
  files: likely `apps/root-worker-prototype/src/lib/conversationReplacementHistory.ts`, `apps/root-worker-prototype/src/components/Conversation.tsx`, `apps/root-worker-prototype/src/components/Conversation.test.tsx`, `apps/root-worker-prototype/src/lib/conversation.test.ts`, possibly protocol/types only if section facts are currently insufficient.
  base_commit: `33aaff9cc`
  status: dispatched
  objective: Render/present Init Context as distinct source sections instead of one monolithic “Init Context” text/details block, while preserving the single init-context event and provider-visible audit facts.
  design_intent: Init Context is a bundle of separately meaningful provider-visible inputs (AGENTS/instructions, environment, tools, runtime activity, skills, etc.). Users need to inspect these by source/section, not read a single concatenated blob.
  problem_model: Typed `injectedContext.sections` already exists, but `conversationReplacementHistory.ts` currently joins every section into one `toolDetails` string for a single `Init Context` tool entry, so the UI cannot expose section boundaries.
  invariants: Do not duplicate Init Context turns; do not split the persisted event into fake independent user/assistant messages; do not lose section labels/text, ordering, truncation metadata, reload behavior, search/detail discoverability, or provider-visible audit fidelity.
  forbidden_paths: no regex splitting rendered text after labels; no CSS-only visual splitting over a monolithic string; no backend history rewrite just for UI; no hard-coded section names as the only supported model.
  expected_implementation_outline: Preserve typed section structure through conversation projection or tool detail modeling, render Init Context details as per-section blocks/rows with labels and bounded text, and update tests so sections remain individually visible/searchable without duplicating the top-level init event.
  minimum_regression_matrix: typed injectedContext with multiple sections; empty/missing preview; long section text; reload/read snapshot path; replacement-history/compact detail path; search across labels and text; no duplicate Init Context rows; focused tests and reviewer approval.
  next_action: await owner_dev_3/reviewer completion, then PM design验收/merge and include in next Runtime Capsule delivery.
  blockers: none.

- id: capsule-only-runtime-delivery
  owner: /self/owner_dev
  reviewer: /self/owner_dev/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev
  branch: feature/capsule-only-runtime-delivery
  task_type: build/runtime-delivery-optimization
  depends_on: user request on 2026-09-19 CST: “每次重启如果 launcher 没修改不用编译，也不用打包 dmg，只要构建 capsule”.
  files: likely `apps/root-worker-prototype/scripts/package-mac-app.cjs`, Runtime Capsule producer/update scripts, package scripts in `apps/root-worker-prototype/package.json`, tests around installed artifact update/capsule producer.
  base_commit: `0ed98d7c8`
  status: merged_pending_installed_validation
  objective: Add or expose a fast delivery path that builds the Runtime Capsule payload/artifact needed by `request_runtime_restart` without rebuilding the outer Launcher or creating a DMG when Launcher/outer app inputs are unchanged.
  design_intent: Runtime Capsule is the update unit for ordinary renderer/main/preload/app-server/default-config changes. Full app packaging/DMG is only needed for installer/outer Launcher/seed distribution. PM delivery should avoid unnecessary launcher release compile and DMG packaging for every runtime restart.
  invariants: Do not weaken capsule manifest/integrity/signing; do not skip app-server/renderer/native helper builds that are part of the Runtime payload; do not break full mac app/DMG packaging; request_runtime_restart must still install a complete verified Runtime Capsule.
  forbidden_paths: no copying loose files into installed artifacts; no in-place mutation of current artifact; no unsigned/partial Capsule; no assuming Launcher unchanged by file name only without a reliable dependency/input check or explicit script target; no removing full package flow.
  expected_implementation_outline: Identify current package script boundaries, add a dedicated capsule-only build command or split existing mac packaging into payload/capsule vs outer-app/dmg phases, update PM/developer docs/scripts, and add focused tests or smoke checks proving the capsule-only path emits the same valid capsule metadata/release id structure needed by Runtime Launcher.
  minimum_regression_matrix: capsule-only command builds renderer/app-server/default-config/native/helper payload and capsule metadata; full `package:root-worker-prototype:mac` still works; request_runtime_restart can install capsule-only artifact; Launcher unchanged path skips launcher compile/DMG; Launcher-changed/full package path remains available; focused tests and diff check; reviewer approval.
  owner_result: commit `572d158d305b5a1d6a2f39aa7db90733e0168b7f` adds `package:root-worker-prototype:mac:capsule`, splits `packageMacRuntimeCapsule`, copies complete sealed Capsule into `runtime-launcher/incoming/<activationId>`, full mac package reuses capsule stage before Launcher packaging, focused script tests 9/9 and `git diff --check` pass, reviewer approved after activation id boundary fix.
  merge_commit: `71e628861`
  pm_validation: PM inspected against build/runtime delivery brief; main `node --test apps/root-worker-prototype/scripts/package-mac-app.test.cjs apps/root-worker-prototype/scripts/package-mac-capsule.test.cjs` 9/9; diff check passed before merge.
  next_action: after pending UI fixes are merged, use `pnpm package:root-worker-prototype:mac:capsule` from canonical main for installed delivery and verify Launcher selection/control state/self-debug.
  blockers: none.

## Recent Completed
- id: file-tree-open-normal-preview
  status: installed_effective
  owner: /self/owner_dev
  reviewer: /self/owner_dev/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev
  branch: fix/file-tree-opens-file-preview
  commit: owner `d3a5ab0c8b8a9e14b1e8f9226f3fdec57fcd2cbf`; merge `53717f8991d08022f6db1d14c4f1a0b3e21c1bc3`
  summary: File tree file clicks now open normal file preview; Git panel staged/unstaged/commit file rows remain the diff-entry points. Removed file-tree-to-Git-diff resolver/status lookup.
  validation: owner/reviewer approved; PM design验收 passed; `pnpm --dir apps/root-worker-prototype exec tsx --test src/components/RightPanel.test.tsx` 59/59; `git diff --check HEAD~1..HEAD` pass. Delivered in Runtime Capsule `sha256:bb7f0bab0d56b639995f6ead7d38441c26d3f212dcb590e9fdfca40ed6a033f3`; self-debug installed renderer URL points to `bb7f0bab...`, console 0 errors/warnings.

- id: computer-use-helper-signed-equivalence
  status: installed_effective
  owner: /self/owner_dev_3
  reviewer: /self/owner_dev_3/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev-3
  branch: fix/computer-use-helper-signed-equivalence
  commit: owner `c108505c732a66328b4af4fb4974e204dcb3f0cd`; merge `e79fb2ecc26e2fa39d96a2bcc11b2356c5b6237b`
  summary: Stable Computer Use helper equivalence now normalizes Mach-O code-signature-only differences instead of raw hashing signed executable bytes, while still detecting real code/resource changes.
  validation: owner/reviewer approved; PM design验收 passed; main `node --test apps/root-worker-prototype/electron/installedArtifactUpdate.test.cjs apps/root-worker-prototype/electron/appServerClient.test.cjs` 85/85; package/restart installed `sha256:c1ea6473...`; explicit current-release materialization dry-check returned `status:"unchanged"` without changing inode/mtime. After user authorization, MCP `computer.permissions_status(includeObservation:true, includePerception:false)` reported Screen Recording granted, Accessibility true, stablePermissionSubject true. Subsequent UI Capsule delivery did not rewrite the helper.

- id: readme-project-comparison
  status: merged_no_capsule_required
  commit: `17bf5645c Document Morpheus project comparison`
  summary: README rewritten in Chinese with Morpheus overview and comparison against OpenAI Codex, Claude Code Best, and DeepSeek Harness/dsh; stable reference source locations documented. Pure documentation change; no Runtime Capsule needed.

## Known Issues
- 2026-09-19 Computer Use stable helper no-rewrite keeps `Contents/Resources/payload-electron-path` unchanged by design to avoid touching the stable helper bundle. After the `bb7f0bab...` delivery, MCP server diagnostics still show `execPath` from previous artifact `c1ea6473...` while the main Runtime is `bb7f0bab...`. This is currently safe because `c1ea6473...` is externalPrevious, but future artifact GC could break the stable helper MCP server if the referenced previous payload is removed. Need a follow-up design for a stable current-runtime indirection or non-TCC-breaking payload pointer update.
- 2026-09-19 current package/delivery workflow is inefficient for Runtime-only changes: PM used full mac app packaging and DMG to produce Runtime Capsule releases `c1ea6473...` and `bb7f0bab...`. User requested a capsule-only build/install path when Launcher/outer app is unchanged; tracked as active queued work `capsule-only-runtime-delivery`.
- 2026-09-19 `.morpheus/config.toml` remains dirty from user/runtime config changes and must not be committed accidentally.
- Older completed entries and stale known issues are archived in [PM Progress Archive](pm-progress-archive/index.md).
