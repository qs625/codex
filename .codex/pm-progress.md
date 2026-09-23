# PM Progress

## Index
- [Current Goal](#current-goal)
- [Active Work](#active-work)
- [Recent Completed](#recent-completed)
- [PM Progress Archive](pm-progress-archive/index.md)
- [Known Issues](#known-issues)

## Current Goal
Fix compact fresh Init Context so project-level `instruction_files` from the current workspace `.morpheus/config.toml` are included after compact, not only home-level `MORPHEUS_HOME/instructions` files. Current main HEAD is `9d8f24efd50e42bee94cea69819783e401a1b1c6` after merging the followup-tool-item display fix; installed Runtime Capsule is still `sha256:16f2fe9c704e42d438f2604afc904e60f05c4017a56f212df8a96915e2d02562`. Restart request `call_EvYNHCj9ESFHGrDc4zOyyTqd` has already recovered and must not be repeated for the same delivery.

## Active Work
- id: compact-project-instruction-files
  status: dispatched
  owner: /self/owner_dev
  reviewer: /self/owner_dev/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev
  branch: owner to choose
  task_type: runtime_context_bugfix
  depends_on: none
  files: likely `codex-rs/thread-service/src/session/*`, `codex-rs/config/src/*`, focused compact/init-context tests
  base_commit: `fb9d8813c1ce5a49820071ee55cea000266cb832`
  next_action: owner should reproduce why compact turn config lacks project `instruction_files`, implement minimal fix, review, and return commit + validation.
  validation: pending
  commit: pending
  notes: Main checkout currently has user/PM local dirt (`.morpheus/instructions/project-understanding.md`, user edits in `events_history.rs` and tests, `.codex/pm-progress.md`, generated `dist-seed-capsule/`). Do not overwrite these; merge back via git when owner completes.

- id: followup-tool-item-hide-stale-agent-state
  status: merged_pending_capsule_delivery
  owner: /self/owner_dev_2
  reviewer: /self/owner_dev_2/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev-2
  branch: fix/init-context-baseline-missing-root-cause
  task_type: ui_display_bugfix
  depends_on: none
  files: likely `apps/root-worker-prototype/src/lib/conversation*`, thread item/tool presentation tests, possibly app-server-protocol projection only if the stale state is server-provided
  base_commit: `fb9d8813c1ce5a49820071ee55cea000266cb832`
  next_action: deliver via next Runtime Capsule after compact instruction_files fix is also resolved, unless user asks for immediate UI-only delivery.
  validation: owner `pnpm --dir apps/root-worker-prototype exec tsx --test src/lib/conversation.test.ts` 84 passed; `pnpm --dir apps/root-worker-prototype exec tsx --test src/lib/conversationPresentation.test.ts` 12 passed; `git diff --check` passed; reviewer approved; PM diff/design验收 passed.
  commit: owner `a51d80a8fd9413c96f1e8338fe5b412866162563`; merged to main via `9d8f24efd50e42bee94cea69819783e401a1b1c6`
  notes: Tool item now hides stale `Agent States` details for `sendInput`/`resumeAgent` while retaining spawn/list agent state display and current call audit facts. Pending capsule delivery because this changes root-worker frontend runtime code.

## Recent Completed
- id: browser-cdp-direct-target-creation
  status: installed_effective
  owner: /self/owner_dev_2
  reviewer: /self/owner_dev_2/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev-2
  branch: fix/browser-cdp-direct-target-creation
  commit: merged through `fb9d8813c1ce5a49820071ee55cea000266cb832`
  summary: Browser CDP compatibility proxy now supports direct target creation for `/json/new` and browser-level websocket `Target.createTarget` without hanging, and the follow-up native BrowserView lifecycle fix prevents direct-created tabs from forcing Browser panel attach/raise or full-window overlay while another right panel is active.
  validation: focused owner/reviewer tests passed before merge; installed Runtime Capsule `sha256:16f2fe9c704e42d438f2604afc904e60f05c4017a56f212df8a96915e2d02562` is running from main `fb9d8813c1ce5a49820071ee55cea000266cb832` after restart request `call_EvYNHCj9ESFHGrDc4zOyyTqd`. Installed CDP validation: `/json/new` returned 200 target info for `https://example.org/?morpheus_json_new_installed=1` with target id `A7913E23359466A9AAA9338478FED399`; browser websocket `Target.createTarget` returned target id `738288D2225E78DBD367E65521350C36` for `https://example.net/?morpheus_create_target_installed=1`; visible Browser content was bounded to the right panel (Browser content/control x `1049..1474`, rail x `1474..1520`, app width `1520`); with Terminal active, direct `Target.createTarget` returned target id `34CAEC8CCB703CD0889E7FF2BD9D2464` for `https://example.com/?morpheus_hidden_panel_installed=1` while Terminal stayed active, Browser stayed inactive, and main renderer DOM did not show Example Domain or the hidden URL. Cleanup closed matching test targets; remaining `morpheus_(json_new|create_target|hidden_panel)_installed=1` targets are `[]`. OS screenshot proof unavailable because `computer_use.observe` failed with macOS `screencapture ... could not create image from display`; Playwright/CDP/DOM evidence was used instead.
  workspace: tracked working tree clean after PM stash isolation; unrelated tracked dirt remains preserved in `stash@{Tue Sep 22 18:35:13 2026}: On main: pm isolate dirty tracked files before runtime capsule 2026-09-22`; generated `apps/root-worker-prototype/dist-seed-capsule/` remains untracked and must not be committed.

- id: readme-codex-build-benchmark
  status: merged_no_capsule_required
  owner: /self/owner_dev_3
  reviewer: /self/owner_dev_3/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev-3
  branch: docs/codex-build-benchmark
  commit: owner `cdf5ad20b`; merge `8573d309e`
  summary: README includes a reproducible single-run Rust app-server debug build benchmark comparing Morpheus `app-server` at `9f0f257a...` and OpenAI Codex reference `codex-app-server` at `78245b47...`, with environment, cold/warm definitions, commands, results, and caveats. PM later adjusted the hot-compile section per user feedback: Morpheus hot sample is the real plan-tool feature merge build `418d78e25` at 61.57s; OpenAI Codex hot sample is now `codex-rs/core/src/lib.rs` timestamp invalidation followed by `codex-app-server` build at 229.62s, so downstream server rebuild/link time is included. The earlier `codex-core`-only 9.12s measurement is documented only as a non-comparable旁注. Pure documentation; no Runtime Capsule needed.
  validation: owner benchmark and reviewer passed; PM inspected README diff and raw log `/tmp/morpheus-codex-build-benchmark.v7STH6/benchmark.log`; PM reran hot samples: Morpheus app-server after feature merge 61.57s, Codex core invalidation -> app-server 229.62s; `git diff --check` pass.

- id: split-init-context-into-items
  status: installed_effective
  owner: /self/owner_dev_3
  reviewer: /self/owner_dev_3/reviewer
  checkout: /Users/bytedance/.morpheus/source_workspace-dev-3
  branch: fix/init-context-multiple-items
  commit: owner `decbd58e4`; merge `6abcc5a6`
  summary: Typed `injectedContext.sections` now project as multiple adjacent Init Context context/tool items (`ctx-id:section:N`) instead of one item with `toolDetailSections`; compact replacement history uses the same section-item projection and context entries are prevented from being re-merged by compact cell grouping.
  validation: owner/reviewer approved; PM design验收 passed against the brief (typed projection, not CSS-only; no regex details splitting; persisted/model-visible history unchanged); main `pnpm --dir apps/root-worker-prototype exec tsx --test src/components/Conversation.test.tsx src/lib/conversation.test.ts src/lib/conversationVirtualization.test.ts src/lib/thread.test.ts` 358/358. Delivered via capsule-only release `sha256:1cc50bd...` from sourceCommit `a4987fa84`, restart `call_rXgzD1RpJ3Z9aGlJCdo5zBG4`; self-debug confirmed installed renderer URL points to `1cc50bd...`, `.tool-detail-sections` count 0, compact summary details/body count 0, and console 0 errors/warnings.

- id: compact-marker-non-expandable-ui
  status: installed_effective
  owner: /self/owner_dev_3
  reviewer: /self/owner_dev_3/reviewer
  commit: owner `c64e67639`; merge `33aaff9cc`
  summary: Compact rows now render as non-expandable `Context compacted` markers; no compact summary `<details>`, summary preview/body, or “Summary available/View summary” appears in the active chat UI.
  validation: owner/reviewer approved; PM design验收 passed; main focused UI/lib tests 114/114 and then combined Init Context tests 121/121; installed via capsule-only release `sha256:3e6269...`; self-debug DOM confirmed compact summary details/body count 0 and console 0 errors/warnings.

- id: split-init-context-display
  status: installed_effective
  owner: /self/owner_dev_3
  reviewer: /self/owner_dev_3/reviewer
  commit: owner `ac04bd218`; merge `924c1a598`
  summary: Init Context remains one row/event but carries typed `toolDetailSections`; ToolRow details render section labels/text separately while preserving `toolDetails` as compatibility/search fallback.
  validation: owner/reviewer approved after virtualization height double-count fix; PM design验收 passed; main `pnpm --dir apps/root-worker-prototype exec tsx --test src/components/Conversation.test.tsx src/lib/conversation.test.ts src/lib/conversationVirtualization.test.ts` 121/121; installed via capsule-only release `sha256:3e6269...`; self-debug confirmed new renderer bundle and zero console errors/warnings.

- id: capsule-only-runtime-delivery
  status: installed_effective
  owner: /self/owner_dev
  reviewer: /self/owner_dev/reviewer
  commit: owner `572d158d305b5a1d6a2f39aa7db90733e0168b7f`; merge `71e628861`
  summary: Added explicit `package:root-worker-prototype:mac:capsule` path that builds a complete Runtime Capsule into `runtime-launcher/incoming/<activationId>` without rebuilding the outer Launcher or DMG; full mac packaging still reuses the capsule stage.
  validation: owner/reviewer approved; PM design验收 passed; main script tests 9/9; actual delivery used `pnpm package:root-worker-prototype:mac:capsule`, produced `sha256:3e6269...`, and recovered restart `call_74YbmRKaOr3SkSr4SJYNhgXn` selected it as externalCurrent/activeLaunch. No DMG/full outer app packaging was used for this delivery.

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
- 2026-09-22 PM isolated unrelated tracked dirt into `stash@{Tue Sep 22 18:35:13 2026}: On main: pm isolate dirty tracked files before runtime capsule 2026-09-22`; do not blindly pop it because it contains mixed non-Browser changes. `apps/root-worker-prototype/dist-seed-capsule/` is generated output and remains untracked; do not commit it.
- Older completed entries and stale known issues are archived in [PM Progress Archive](pm-progress-archive/index.md).
