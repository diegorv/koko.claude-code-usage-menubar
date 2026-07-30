# PRD — Kimi (api.kimi.com) usage provider

## Goal

Show Kimi Code usage alongside Claude usage in the menubar popup and tray, using the Kimi usages API:

```
GET https://api.kimi.com/coding/v1/usages
Authorization: Bearer <MOONSHOT_API_KEY>
```

## Decisions (confirmed with user)

- **Both providers simultaneously** — popup shows a section per provider.
- **API key stored in the macOS Keychain** via `/usr/bin/security` subprocess (same pattern as `token_cache.rs` — never in-process Keychain API, never plaintext).
- **Key entry happens in the popup** — no new settings window (avoids per-window capabilities work).
- Fetch stays Rust-side; CSP in `tauri.conf.json` does not change.

## Response mapping (captured payload, 2026-07-29)

| Kimi field | App concept |
|---|---|
| the `limits[]` entry whose window is 300 min | session % (`used/limit*100`) |
| `usage` (limit/used/remaining/resetTime) | weekly % |
| `parallel` (`details.len()` / `limit`) | extra metric: parallel sessions |
| — | no per-model breakdown exists |

Percentages are computed, not provided. A silent reshape is possible (happened twice with Anthropic), so the parser must emit `shape_warning` when `limits` or `usage` go missing, or when no 300-minute window is present.

### Open: is `usage.resetTime` really weekly?

**Still unconfirmed.** The code labels it "Weekly" and the popup says so, on one observation.

What the capture actually shows: taken 2026-07-29, `usage.resetTime` is 2026-08-04T11:59:17.868440Z — about 5 days 13 hours out — while the session window resets the same evening. Both timestamps carry identical fractional seconds (`.868440`), so they are derived from one account anchor rather than being independent counters.

That is consistent with a 7-day window seen roughly a day and a half in. It is equally consistent with any longer window seen near its end. One sample cannot distinguish them.

To settle it, capture `usage.resetTime` twice more than 24h apart: a fixed window keeps the same instant and jumps by exactly the period when it rolls. Until then the label is an assumption, not a finding — and note that a wrong guess here is cosmetic, since the percentage itself comes from `used`/`limit` and does not depend on the period.

## Risks

- 401 means invalid/revoked key → surface `auth_error` in the popup, don't drop the stored key silently.
- 429 must respect `Retry-After`.
- Two providers = 2 requests per cycle; keep interval ≥ 60s.
- Tray is 22px tall: with both providers active, show one metric per provider (weekly %), labels C/K.

## Issues

1. `issues/01-kimi-key-keychain-popup-ui.md`
2. `issues/02-generalize-payload-providers.md`
3. `issues/03-kimi-fetch-parse-popup-section.md`
4. `issues/04-tray-icon-multi-provider.md`
