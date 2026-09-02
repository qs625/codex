# Root Worker Prototype

Electron product shell for a root-agent plus worker-agent workspace.

## Run

```bash
pnpm install
pnpm --filter @my-codex/root-worker-prototype dev
```

This starts:

- Vite on `http://localhost:5173` for the Electron renderer
- Electron, which starts a bundled `app-server` when running from a packaged app; source-tree runs fall back to `../../codex-rs/target/debug/app-server`, `../../codex-rs/target/release/app-server`, then `app-server` from `PATH`
- the prototype defaults `MORPHEUS_HOME` to `~/.morpheus`

Use this only when you specifically want the Vite dev server flow.

You can override the app-server command, Morpheus home, or workspace with:

```bash
MORPHEUS_HOME=/tmp/my-codex-root-worker-ui-env/morpheus-home \
ROOT_WORKER_WORKSPACE=/tmp/my-codex-root-worker-ui-env/workspace \
APP_SERVER_CMD="app-server --listen stdio://" \
pnpm --filter @my-codex/root-worker-prototype dev
```

## Build

```bash
pnpm --filter @my-codex/root-worker-prototype build
```

## Package macOS

```bash
pnpm --filter @my-codex/root-worker-prototype package:mac
```

The macOS app packaging builds release `app-server`, bundles it into the app
resources at `Contents/Resources/bin/app-server`, and bundles the default
compact prompt seed at `Contents/Resources/default-config/compact/COMPACT.md`.
It also bundles a tracked source snapshot at `Contents/Resources/source`.
The snapshot excludes heavy build outputs, dependency caches, package staging,
local Android config, `.env*`, and common key/certificate files.
When launched from Finder or Dock, the app prepares `MORPHEUS_HOME`, preserves
the desktop process environment with an enhanced `PATH`, and creates
`~/.morpheus/compact/COMPACT.md` only if that file is missing.
When a packaged source snapshot is present and `ROOT_WORKER_WORKSPACE` is not
set, the app seeds `~/.morpheus/source_workspace` from the bundled snapshot only
if that writable workspace is missing, then uses it as the default workspace.
Existing user edits in `source_workspace` are never overwritten by launch.

The packaged app uses the stable bundle id
`com.openai.root-worker-prototype.dev` for macOS privacy prompts. `Info.plist`
declares microphone, screen capture, and app automation usage descriptions.
Screen Recording and Accessibility approval are still granted by macOS to the
installed, signed app identity in System Settings; they are not replaced by
external computer-use permissions.

## Electron

```bash
pnpm --filter @my-codex/root-worker-prototype start
```

`start` now loads the built renderer from `dist/index.html` even when running from the source tree. If the build output is missing, Electron exits with an error telling you to run the build first.

For manual prototype iteration without Vite hot reload, use:

```bash
pnpm --filter @my-codex/root-worker-prototype build
pnpm --filter @my-codex/root-worker-prototype start
```
