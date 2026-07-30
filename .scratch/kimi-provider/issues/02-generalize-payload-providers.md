# 02 — Generalize UsagePayload to providers[]

Status: done

## What to build

Pure refactor: turn the single-provider `UsagePayload` (fixed `session_percent`/`weekly_percent`, `parser.rs:3-17`) into `providers: Vec<ProviderPayload>`, where each provider carries: `id`, `title`, `status` (ok / auth_error / rate_limited / disabled), `session_percent`, `weekly_percent`, reset timestamps, and an `extra` slot for provider-specific metrics (extra_usage for Claude, parallel sessions for Kimi).

- Claude must emit exactly the same numbers as today — no behavior change.
- Frontend types in `src/lib/usage.ts` mirror the new shape.
- `PopupView.svelte` renders sections via `{#each providers as provider}`; the hardcoded "Claude Usage" header (line ~107) becomes the provider title.
- Tray keeps current behavior, reading the first (Claude) provider — multi-provider tray is issue 04.

## Acceptance criteria

- [x] All existing fixture-based parser tests pass unchanged (same Claude numbers)
- [ ] Popup renders Claude section identically to before (visual check)
- [x] Tray icon unchanged
- [x] `cargo test`, `cargo check`, `pnpm check` pass

## Blocked by

None - can start immediately (parallel with issue 01)

## Out of scope

- No Kimi fetching or parsing (issue 03).
- No tray layout changes (issue 04).

## Comments

Shipped in 0b6ded9.

Left unchecked: the popup visual check against the pre-refactor Claude
section. The parser tests prove the numbers are identical; only a running app
proves the rendering is.
