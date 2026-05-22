# 05 — Split `fetch_usage_payload` and extract pure `classify`

Status: ready-for-agent
Phase: B
Depends on: 04

## Problem

`fetch_usage_payload` is 100 lines doing 5 jobs: token retrieval, HTTP send, status classification, body parsing, side-effect cache invalidation. Not testable without HTTP mocking, and even with mocking the test surface is too coarse to cover all branches (401, 403, 429 with/without retry-after, 5xx, JSON parse error, network error).

## Change

New module `src-tauri/src/parser.rs`:

```rust
pub fn classify(status: u16, retry_after: Option<u64>, body: &str) -> UsagePayload {
    match status {
        200..=299 => parse_api_response_or_error(body),
        401 | 403 => UsagePayload::error("unauthorized", "Token expired. Run \"claude login\" to re-authenticate."),
        429 => {
            let msg = match retry_after {
                Some(secs) => format!("Rate limited. Try again in {}s.", secs),
                None => "Rate limited. Please try again later.".to_string(),
            };
            UsagePayload::error("error", &msg)
        }
        s => UsagePayload::error("error", &format!("HTTP {}: {}", s, body)),
    }
}

pub fn parse_api_response(json: &serde_json::Value) -> UsagePayload { ... }  // moved from commands.rs
```

Where `parse_api_response_or_error` wraps `serde_json::from_str` failure in an `UsagePayload::error("error", ...)`.

### Caller (commands.rs)

```rust
async fn fetch_usage_payload(token_cache: &TokenCache, payload_cache: &PayloadCache) -> UsagePayload {
    payload_cache.mark_fetch_start();

    let token = match token_cache.get_or_read() { ... };

    let response = match HTTP_CLIENT.get(...).send().await { ... };

    let status = response.status().as_u16();
    let retry_after = response.headers().get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());
    let body = response.text().await.unwrap_or_default();

    let payload = parser::classify(status, retry_after, &body);

    if payload.status == "unauthorized" {
        token_cache.invalidate();
    }
    if payload.status == "ok" {
        payload_cache.store(payload.clone());
    }

    payload
}
```

Tests in this ticket: characterization for `classify` (one per status branch) + `parse_api_response` (existing 2 tests move from `commands.rs` to `parser.rs`).

## Verify

- `cargo test` — new `classify` tests green, existing parse tests still green.
- `pnpm tauri dev` → manually trigger error states if possible (revoke token, observe `unauthorized`; throttle by spamming refresh).

## Out of scope

- Don't add wiremock yet (that's ticket 06 for one happy-path smoke).
- Don't change the `status` string values — frontend depends on them.
