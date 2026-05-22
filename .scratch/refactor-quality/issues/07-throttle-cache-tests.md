# 07 — Tests for throttle + token cache TTL

Status: ready-for-agent
Phase: C
Depends on: 04

## Problem

Throttle (`MIN_FETCH_INTERVAL_SECS=30`) and token cache TTL (`TOKEN_CACHE_TTL_SECS=86400`) are critical to correct behavior and rate-limit safety, but untested.

## Change

### `state/payload_cache.rs` tests

```rust
#[test] fn cached_if_fresh_returns_none_when_empty() { ... }
#[test] fn cached_if_fresh_returns_payload_within_ttl() { ... }
#[test] fn cached_if_fresh_returns_none_after_ttl() { ... }
#[test] fn store_overwrites_previous() { ... }
```

Use a small TTL (e.g. 1s) to keep tests fast.

### `state/token_cache.rs` tests

Trickier — `get_or_read` shells out to keychain. Options:

**Option A:** test only `parse_keychain_json` (already exists). Skip TTL test for now.

**Option B:** refactor `TokenCache` to accept an injected `fn() -> Result<String, String>` reader. Then test TTL with a counter-fn.

Recommend **Option B** if it's a 10-line change. Otherwise Option A and move on.

```rust
pub struct TokenCache<R = KeychainReader>
where R: Fn() -> Result<String, String> {
    cached: Mutex<Option<(String, Instant)>>,
    reader: R,
}
```

Or simpler: free fn `get_or_read(cached: &Mutex<...>, reader: &dyn Fn() -> Result<String, String>) -> Result<String, String>` separated from the struct method.

### `commands.rs` (or wherever throttle lives after ticket 04) tests

Throttle behavior inside `trigger_refresh`:

- After first fetch, second call within MIN_FETCH_INTERVAL_SECS returns cached payload.
- After MIN_FETCH_INTERVAL_SECS elapses, throttle is released.

Requires either time-injection or a custom `MIN_FETCH_INTERVAL_SECS` const exposed as a struct field on `PayloadCache`. Cleaner: parameterize `cached_if_fresh(ttl_secs)` — already proposed in ticket 04. Test with `ttl_secs=0` to bypass, `ttl_secs=999` to force.

## Verify

- `cargo test` — new tests green.

## Out of scope

- Don't test the polling loop itself (timer behavior). Trust `tokio::time::sleep`.
- Don't add a `tokio::time::pause()` test — overkill for this scope.
