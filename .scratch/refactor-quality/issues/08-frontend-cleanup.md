# 08 — Frontend cleanup: drop dead `appState.usage`

Status: ready-for-agent
Phase: D

## Problem

`src/lib/store.svelte.ts` exposes:

```ts
get usage() { return _usage; }
set usage(v) { _usage = v; }
```

Neither getter nor setter is read by any caller. `PopupView.svelte` keeps its own `let usage = $state<UsageData | null>(null)`. The store field is dead code that suggests a source-of-truth that doesn't exist.

## Change

### `src/lib/store.svelte.ts`

- Delete `_usage` declaration.
- Delete `get usage()` and `set usage()` from the exported object.
- Keep `_intervalSeconds`, `loadSettings`, `saveInterval`.

### Optional new test: `tests/store.test.ts`

Light coverage of `appState.loadSettings` / `appState.saveInterval` using a mocked `@tauri-apps/plugin-store`. Mock `load()` to return an in-memory map.

If mocking the plugin is more code than the store itself, skip the optional test.

## Verify

- `pnpm check` — TypeScript clean.
- `pnpm test` — existing tests still pass.
- `pnpm tauri dev` → open popup, confirm usage data renders, change interval, restart app, confirm interval persists.

## Out of scope

- Don't refactor `PopupView.svelte` further. The local `$state` pattern is fine for this component.
- Don't add ProgressBar tests — visual component, low ROI for the test infra cost.
