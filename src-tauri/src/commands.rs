use std::sync::LazyLock;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::parser::{self, UsagePayload};
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

// --- Usage fetching (internal) ---

async fn fetch_usage_payload(token_cache: &TokenCache, payload_cache: &PayloadCache) -> UsagePayload {
    payload_cache.mark_fetch_start();

    let token = match token_cache.get_or_read() {
        Ok(t) => t,
        Err(e) => {
            let status = if e.contains("Failed to read keychain") || e.contains("No accessToken") {
                "missing_credentials"
            } else {
                "error"
            };
            return UsagePayload::error(status, &e);
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
        Err(e) => return UsagePayload::error("error", &format!("Request failed: {}", e)),
    };

    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let body = match response.text().await {
        Ok(b) => b,
        Err(e) => return UsagePayload::error("error", &format!("Failed to read response: {}", e)),
    };

    let payload = parser::classify(status, retry_after, &body);

    if payload.status == "unauthorized" {
        token_cache.invalidate();
    }
    if payload.status == "ok" {
        payload_cache.store(payload.clone());
    }

    payload
}

// --- Refresh cycle ---

fn update_tray_icon(app: &AppHandle, payload: &UsagePayload) {
    if payload.status != "ok" {
        return;
    }
    let session = payload.session_percent as f64 / 100.0;
    let weekly = payload.weekly_percent as f64 / 100.0;
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
