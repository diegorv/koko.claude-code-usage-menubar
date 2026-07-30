use std::sync::LazyLock;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::kimi_parser;
use crate::parser::{self, ProviderPayload, ProviderStatus, UsagePayload};
use crate::state::{PayloadCache, TokenCache, UsagePoller};

static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("ClaudeUsageMenubar")
        .build()
        .expect("failed to build HTTP client")
});

const MIN_FETCH_INTERVAL_SECS: u64 = 30;
const USAGE_API_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const KIMI_API_URL: &str = "https://api.kimi.com/coding/v1/usages";

// --- Usage fetching (internal) ---

/// Fetches both providers concurrently and assembles one payload. Claude is
/// always providers[0] (the tray reads it); Kimi is appended when a key is
/// configured and omitted entirely otherwise, so a keyless install behaves
/// exactly as a Claude-only build.
async fn fetch_usage_payload(token_cache: &TokenCache, payload_cache: &PayloadCache) -> UsagePayload {
    payload_cache.mark_fetch_start();

    let (claude, kimi) = tokio::join!(fetch_claude_provider(token_cache), fetch_kimi_provider());

    let mut providers = vec![claude];
    providers.extend(kimi);
    let payload = UsagePayload::new(providers);
    if payload.providers[0].status == ProviderStatus::Ok {
        payload_cache.store(payload.clone());
    }

    payload
}

async fn fetch_claude_provider(token_cache: &TokenCache) -> ProviderPayload {
    let token = match token_cache.get_or_read() {
        Ok(t) => t,
        Err(e) => {
            let status = if e.contains("Failed to read keychain") || e.contains("No accessToken") {
                ProviderStatus::AuthError
            } else {
                ProviderStatus::Error
            };
            return ProviderPayload::claude_error(status, &e);
        }
    };

    let response = match HTTP_CLIENT
        .get(USAGE_API_URL)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("anthropic-beta", "oauth-2025-04-20")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return ProviderPayload::claude_error(
                ProviderStatus::Error,
                &format!("Request failed: {}", e),
            )
        }
    };

    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let body = match response.text().await {
        Ok(b) => b,
        Err(e) => {
            return ProviderPayload::claude_error(
                ProviderStatus::Error,
                &format!("Failed to read response: {}", e),
            )
        }
    };

    let provider = parser::classify(status, retry_after, &body);

    if provider.status == ProviderStatus::AuthError {
        token_cache.invalidate();
    }

    provider
}

/// `None` when no key is usable — the provider is omitted from the payload,
/// never rendered as an error. A keychain infra failure reads as "no key":
/// the alternative is an error row every poll for a condition that usually
/// heals on the next cycle. On 401 the key is kept (kimi_parser classifies;
/// nobody deletes).
async fn fetch_kimi_provider() -> Option<ProviderPayload> {
    let key = match crate::state::kimi_key::read() {
        Ok(Some(key)) => key,
        _ => return None,
    };

    let response = match HTTP_CLIENT
        .get(KIMI_API_URL)
        .header("Authorization", format!("Bearer {}", key))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return Some(kimi_parser::error_payload(
                ProviderStatus::Error,
                &format!("Request failed: {}", e),
            ))
        }
    };

    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let body = match response.text().await {
        Ok(b) => b,
        Err(e) => {
            return Some(kimi_parser::error_payload(
                ProviderStatus::Error,
                &format!("Failed to read response: {}", e),
            ))
        }
    };

    Some(kimi_parser::classify(status, retry_after, &body))
}

// --- Refresh cycle ---

fn update_tray_icon(app: &AppHandle, payload: &UsagePayload) {
    // The tray shows the first (Claude) provider only — multi-provider tray
    // layout is a separate change.
    let Some(claude) = payload.providers.first() else {
        return;
    };
    if claude.status != ProviderStatus::Ok {
        return;
    }
    let session = claude.session_percent as f64 / 100.0;
    let weekly = claude.weekly_percent as f64 / 100.0;
    let icon = crate::tray_icon::generate_icon(session, weekly);
    if let Some(tray) = app.tray_by_id("main-tray") {
        let _ = tray.set_icon(Some(icon));
        let _ = tray.set_title(None::<&str>);
    }
}

pub async fn do_refresh_cycle(app: &AppHandle) {
    let token_cache = app.state::<TokenCache>();
    let payload_cache = app.state::<PayloadCache>();
    let payload = fetch_usage_payload(&token_cache, &payload_cache).await;
    update_tray_icon(app, &payload);
    let _ = app.emit("usage_updated", &payload);
}

/// Emits the last cached payload to the frontend (used when popup is shown).
pub fn emit_cached_payload(app: &AppHandle) {
    let payload_cache = app.state::<PayloadCache>();
    if let Some(payload) = payload_cache.get() {
        let _ = app.emit("usage_updated", &payload);
    }
}

// --- Tauri commands ---

/// Starts or restarts auto-refresh with the given interval
#[tauri::command]
pub fn start_auto_refresh(
    app: AppHandle,
    poller: State<'_, UsagePoller>,
    interval_secs: u64,
) -> Result<(), String> {
    poller.restart(app, interval_secs);
    Ok(())
}

/// Triggers a single immediate refresh and returns the data to the caller.
/// Skips the API call if data was fetched less than MIN_FETCH_INTERVAL_SECS ago.
#[tauri::command]
pub async fn trigger_refresh(
    app: AppHandle,
    token_cache: State<'_, TokenCache>,
    payload_cache: State<'_, PayloadCache>,
) -> Result<UsagePayload, String> {
    if let Some(cached) = payload_cache.cached_if_fresh(MIN_FETCH_INTERVAL_SECS) {
        return Ok(cached);
    }

    let payload = fetch_usage_payload(&token_cache, &payload_cache).await;
    update_tray_icon(&app, &payload);
    Ok(payload)
}

/// Hides the popup window
#[tauri::command]
pub async fn hide_popup(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("popup") {
        window.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Quits the application
#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

// --- Kimi API key management ---

/// Stores the Kimi API key in the macOS Keychain (updates in place).
#[tauri::command]
pub fn save_kimi_key(key: String) -> Result<(), String> {
    crate::state::kimi_key::save(&key)
}

/// Removes the Kimi API key from the macOS Keychain.
#[tauri::command]
pub fn delete_kimi_key() -> Result<(), String> {
    crate::state::kimi_key::remove()
}

/// Whether a Kimi API key is stored. The key itself never crosses the IPC
/// boundary — booleans only. Keychain infrastructure errors (e.g. a timed-out
/// `security` call) read as "no key" so the indicator never throws.
#[tauri::command]
pub fn has_kimi_key() -> bool {
    crate::state::kimi_key::exists().unwrap_or(false)
}
