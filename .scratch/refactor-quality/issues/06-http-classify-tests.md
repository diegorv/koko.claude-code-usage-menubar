# 06 — HTTP classify exhaustive tests + one wiremock smoke

Status: ready-for-agent
Phase: C
Depends on: 05

## Problem

Even after extracting `classify`, the new fn is only minimally covered by ticket 05's characterization tests. Want exhaustive coverage of the HTTP response → `UsagePayload` translation.

## Change

### In `parser.rs` tests

Add cases:

- `classify(200, None, valid_json)` → status "ok", values populated.
- `classify(200, None, invalid_json)` → status "error", message contains "Invalid JSON".
- `classify(200, None, "")` → status "error".
- `classify(401, None, _)` → status "unauthorized".
- `classify(403, None, _)` → status "unauthorized".
- `classify(429, None, _)` → status "error", message contains "later".
- `classify(429, Some(42), _)` → status "error", message contains "42s".
- `classify(500, None, "boom")` → status "error", message contains "HTTP 500" and "boom".
- `classify(503, None, "")` → status "error", contains "HTTP 503".

### New: `tests/integration_fetch.rs` (Rust integration test)

Add `wiremock = "0.6"` as a dev-dependency. One test:

```rust
#[tokio::test]
async fn fetch_happy_path_through_real_http() {
    let server = MockServer::start().await;
    // Mock /api/oauth/usage returning the canonical payload
    // Override the base URL via a test-only fn or env var
    ...
}
```

Note: current code hard-codes `https://api.anthropic.com/api/oauth/usage`. Need to thread a base URL into the fetch fn. Simplest: env var `CLAUDE_USAGE_API_BASE` (used only in tests), default to the hard-coded URL when unset. Add this as a tiny helper in `commands.rs`.

If threading a base URL adds too much surface, drop the wiremock test for now and rely on `classify` coverage. Acceptable.

## Verify

- `cargo test` — all `classify` cases green, wiremock test green (if included).

## Out of scope

- Don't mock the network layer with a trait (`HttpClient`). `classify` already gives pure-fn coverage; wiremock is just one E2E smoke.
- Don't test polling, throttle, or cache TTL here — see ticket 07.
