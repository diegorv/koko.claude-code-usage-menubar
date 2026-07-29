# 03 — Kimi fetch + parse + popup section

Status: ready-for-agent

## What to build

The Kimi provider end-to-end: fetch, parse, and display.

- Save the captured response (in `.scratch/kimi-provider/PRD.md` discussion / user-provided curl output) as `src-tauri/fixtures/kimi_usage_response.json` and pin parser tests against it.
- New `kimi_parser.rs`: maps `limits[0]` (300-min window) → session %, `usage` → weekly % (`used/limit*100`, percentages are computed, not provided), `parallel` → extra metric (`details.len()` of `limit`). Emits `shape_warning` when `limits` or `usage` are missing — silent reshapes have happened before with the Anthropic API.
- `fetch_kimi()` in the fetch layer: reqwest GET `https://api.kimi.com/coding/v1/usages`, Bearer from the keychain module (issue 01), no extra headers. 401 → provider status `auth_error` (keep the stored key — don't delete on failure); 429 → respect `Retry-After`.
- `do_refresh_cycle` fetches both providers in parallel (`tokio::join!`) and emits a single `usage_updated` with `providers[]`. No key saved → Kimi provider omitted or `disabled`, never an error.
- Popup renders the Kimi section: session (5h window), weekly with reset time, parallel sessions N/limit. `auth_error` shows "API key inválida" near the section with the settings field accessible.

## Acceptance criteria

- [ ] Parser unit tests pass against the fixture, including the shape_warning case
- [ ] With a valid key saved, the popup shows real Kimi session/weekly/parallel data
- [ ] With an invalid key, popup shows auth_error for Kimi while Claude keeps working
- [ ] Without any key, app behaves exactly as before (Claude only, no errors)
- [ ] `cargo test`, `cargo check`, `pnpm check` pass

## Blocked by

- Issue 01 (keychain + key UI)
- Issue 02 (providers[] payload)

## Out of scope

- Tray icon changes (issue 04).
- CSP changes — not needed, fetch is Rust-side.
