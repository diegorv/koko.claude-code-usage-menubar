# 04 — Split 5 globals into 3 state structs

Status: ready-for-agent
Phase: B

## Problem

`commands.rs` holds 5 disconnected `static LazyLock<Mutex<...>>`:

- `HTTP_CLIENT` (stays static — stateless pool)
- `TOKEN_CACHE` → goes into `TokenCache`
- `POLLING_HANDLE` → goes into `UsagePoller`
- `LAST_FETCH` + `LAST_PAYLOAD` → both go into `PayloadCache` (used atomically by throttle)

Untestable in isolation. No lifetime control.

## Change

Create new module tree:

```
src-tauri/src/state/
├── mod.rs
├── token_cache.rs
├── poller.rs
└── payload_cache.rs
```

### `state/token_cache.rs`

```rust
pub struct TokenCache {
    cached: Mutex<Option<(String, Instant)>>,
}

impl TokenCache {
    pub fn new() -> Self { ... }
    pub fn get_or_read(&self) -> Result<String, String> { ... } // current get_cached_token
    pub fn invalidate(&self) { ... }
}

fn read_token_from_keychain() -> Result<String, String> { ... }
pub(crate) fn parse_keychain_json(json_str: &str) -> Result<String, String> { ... }
```

Move existing `parse_keychain_json` tests into this file.

### `state/poller.rs`

```rust
pub struct UsagePoller {
    handle: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

impl UsagePoller {
    pub fn new() -> Self { ... }
    pub fn restart<F, Fut>(&self, interval_secs: u64, tick: F) // generic over async tick fn
        where F: Fn() -> Fut + Send + 'static + Clone,
              Fut: std::future::Future<Output = ()> + Send;
}
```

Or non-generic: take `Arc<AppHandle>` + the tick fn pointer. Pick whichever is cleaner during implementation.

### `state/payload_cache.rs`

```rust
pub struct PayloadCache {
    last_fetch: Mutex<Option<Instant>>,
    last_payload: Mutex<Option<UsagePayload>>,
}

impl PayloadCache {
    pub fn new() -> Self { ... }
    pub fn mark_fetch_start(&self);
    pub fn store(&self, payload: UsagePayload);
    pub fn get(&self) -> Option<UsagePayload>;
    pub fn cached_if_fresh(&self, ttl_secs: u64) -> Option<UsagePayload>; // throttle check
}
```

### `lib.rs` setup

```rust
.manage(state::TokenCache::new())
.manage(state::UsagePoller::new())
.manage(state::PayloadCache::new())
```

### `commands.rs`

Tauri commands take `State<TokenCache>` / `State<PayloadCache>` etc. via injection. All `LazyLock` statics removed.

## Verify

- `cargo test` — all existing tests still pass (parse tests move with their fns).
- `pnpm tauri dev` → app starts, tray icon redraws on poll, popup opens with data, refresh button works, throttle kicks in within 30s, interval change persists.

## Out of scope

- Don't add new behavior. Pure restructure.
- Don't split `fetch_usage_payload` yet — that's ticket 05.
- Don't migrate `HTTP_CLIENT` — stays static.
