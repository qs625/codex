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
When launched from Finder or Dock, the app prepares `MORPHEUS_HOME`, preserves
the desktop process environment with an enhanced `PATH`, and creates
`~/.morpheus/compact/COMPACT.md` only if that file is missing.

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
