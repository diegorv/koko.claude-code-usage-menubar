# Refactor: Quality, Tests, Architecture

Incremental refactor to raise code quality, add missing tests, and reduce architectural smell without breaking the app.

## Background

Audit (2026-05-22) on `commands.rs` (421 LOC), `lib.rs` (154 LOC), `tray_icon.rs` (169 LOC), `PopupView.svelte` (361 LOC), `usage.ts`, `store.svelte.ts`, and the single test file.

Findings: no grave issues. Medium smell. 11 Rust unit tests, 5 frontend tests — HTTP error paths, throttle, cache TTL, and store are all untested.

## Goals

- Eliminate 5 disconnected globals in `commands.rs` → coherent state model in `src-tauri/src/state/`.
- Make HTTP classification logic pure + testable. Cover 401/403/429/5xx.
- Eliminate frontend two-sources-of-truth in `appState.usage`.
- Add tests where missing, without rewriting working code for testability alone.

## Non-goals

- No UI redesign.
- No new features.
- No dependency upgrades beyond what tests require.
- No CI/release changes.

## Phases

| Phase | Risk | Tickets | Verify |
|-------|------|---------|--------|
| A | Low (mechanical) | 01, 02, 03 | `cargo check` + `pnpm check` + smoke |
| B | Medium (invasive) | 04, 05 | `cargo test` + smoke |
| C | Low (additive) | 06, 07 | `cargo test` |
| D | Low | 08 | `pnpm test` + smoke |

## Deferred / skipped

- `read_saved_interval` in `lib.rs:66` bypassing `tauri-plugin-store` — works, format-drift risk is theoretical.
- Tick-driven `formatTimeRemaining` re-render — UX cosmetic, not architecture.

## Decisions made during grilling

- **State shape**: 3 separate structs by concern (`TokenCache`, `UsagePoller`, `PayloadCache`), not single flat `AppState`. Each registered as Tauri `State<>` separately.
- **HTTP_CLIENT**: stays as `static LazyLock<reqwest::Client>`. Stateless connection pool; wrapping buys nothing.
- **PayloadCache** bundles `last_fetch` + `last_payload`; the throttle reads both atomically.
- **HTTP test strategy**: extract pure `classify(status, retry_after, body)` function. Test exhaustively without HTTP. One `wiremock` smoke test for end-to-end happy path.
- **Side effects in `fetch_usage_payload`**: keep at caller layer after split (e.g. `invalidate_cache()` on `status == "unauthorized"`), keep `classify` pure.
- **Module layout**:
  ```
  src-tauri/src/
  ├── lib.rs
  ├── commands.rs           (thin Tauri command handlers)
  ├── parser.rs             (parse_api_response + classify, pure)
  ├── tray_icon.rs
  └── state/
      ├── mod.rs
      ├── token_cache.rs    (TokenCache + parse_keychain_json + keychain read)
      ├── poller.rs         (UsagePoller, owns JoinHandle)
      └── payload_cache.rs  (PayloadCache, last_fetch + last_payload)
  ```
- **Delivery**: direct commits to `main`, one per ticket. Phase A's 3 mechanical commits batchable into a single restart cycle.
- **Verification bar per commit**: defined per phase above. No characterization tests for Phase A — mechanical extracts verified by `cargo check` + manual smoke.
- **Frontend cleanup**: delete dead `appState.usage` getter/setter from `store.svelte.ts`. PopupView keeps its local `$state`.

## Tickets

See `issues/`.
