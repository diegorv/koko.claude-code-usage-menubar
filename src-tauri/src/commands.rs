use std::sync::LazyLock;
use image::Rgba;
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

/// Picks what the tray shows, as `(label, percent 0.0..=1.0, color)` rows.
/// One provider → the legacy session/weekly rows; several → one weekly row
/// per provider ("C"/"K"), since there's no vertical room for per-provider
/// session data (the popup carries the detail).
///
/// Returns None when the icon should stay untouched: it's a single baked
/// image, so one provider's row can't be frozen while another refreshes —
/// any non-ok provider skips the whole update, the same freeze-on-error
/// semantics the Claude-only tray has always had.
fn tray_rows(payload: &UsagePayload) -> Option<Vec<(char, f64, Rgba<u8>)>> {
    let claude = payload.providers.first()?;
    if payload.providers.iter().any(|p| p.status != ProviderStatus::Ok) {
        return None;
    }
    if payload.providers.len() == 1 {
        return Some(vec![
            (
                'S',
                claude.session_percent as f64 / 100.0,
                crate::tray_icon::COLOR_SESSION,
            ),
            (
                'W',
                claude.weekly_percent as f64 / 100.0,
                crate::tray_icon::COLOR_WEEKLY,
            ),
        ]);
    }
    Some(
        payload
            .providers
            .iter()
            .map(|p| {
                let (label, color) = match p.id.as_str() {
                    "kimi" => ('K', crate::tray_icon::COLOR_KIMI),
                    _ => ('C', crate::tray_icon::COLOR_WEEKLY),
                };
                (label, p.weekly_percent as f64 / 100.0, color)
            })
            .collect(),
    )
}

fn update_tray_icon(app: &AppHandle, payload: &UsagePayload) {
    let Some(rows) = tray_rows(payload) else {
        return;
    };
    let icon = crate::tray_icon::generate_icon(rows);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::ProviderExtra;

    fn provider(id: &str, status: ProviderStatus, session: u32, weekly: u32) -> ProviderPayload {
        ProviderPayload {
            id: id.to_string(),
            title: format!("{} Usage", id),
            status,
            session_percent: session,
            session_resets_at: None,
            weekly_percent: weekly,
            weekly_resets_at: None,
            models: vec![],
            extra: ProviderExtra::ExtraUsage {
                enabled: false,
                percent: 0,
            },
            error_message: None,
            shape_warning: None,
        }
    }

    #[test]
    fn single_provider_keeps_session_weekly_rows() {
        let payload = UsagePayload::new(vec![provider("claude", ProviderStatus::Ok, 45, 67)]);
        let rows = tray_rows(&payload).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, 'S');
        assert_eq!(rows[0].1, 45.0 / 100.0);
        assert_eq!(rows[0].2, crate::tray_icon::COLOR_SESSION);
        assert_eq!(rows[1].0, 'W');
        assert_eq!(rows[1].1, 67.0 / 100.0);
        assert_eq!(rows[1].2, crate::tray_icon::COLOR_WEEKLY);
    }

    #[test]
    fn two_providers_show_weekly_rows_with_provider_labels() {
        let payload = UsagePayload::new(vec![
            provider("claude", ProviderStatus::Ok, 45, 67),
            provider("kimi", ProviderStatus::Ok, 96, 19),
        ]);
        let rows = tray_rows(&payload).unwrap();
        assert_eq!(rows.len(), 2);
        // Weekly, not session, per provider — session is popup-only.
        assert_eq!(rows[0].0, 'C');
        assert_eq!(rows[0].1, 67.0 / 100.0);
        assert_eq!(rows[1].0, 'K');
        assert_eq!(rows[1].1, 19.0 / 100.0);
        // Distinct per-provider colors; Claude keeps today's weekly color.
        assert_eq!(rows[0].2, crate::tray_icon::COLOR_WEEKLY);
        assert_eq!(rows[1].2, crate::tray_icon::COLOR_KIMI);
        assert_ne!(rows[0].2, rows[1].2);
    }

    #[test]
    fn non_ok_claude_skips_update() {
        let payload = UsagePayload::new(vec![provider("claude", ProviderStatus::AuthError, 0, 0)]);
        assert!(tray_rows(&payload).is_none());
    }

    #[test]
    fn non_ok_kimi_skips_update() {
        // The icon is one baked image — it can't hold a stale Kimi row while
        // refreshing Claude, so the whole update is skipped (today's semantics).
        let payload = UsagePayload::new(vec![
            provider("claude", ProviderStatus::Ok, 45, 67),
            provider("kimi", ProviderStatus::RateLimited, 0, 0),
        ]);
        assert!(tray_rows(&payload).is_none());
    }

    #[test]
    fn empty_providers_skips_update() {
        let payload = UsagePayload::new(vec![]);
        assert!(tray_rows(&payload).is_none());
    }
}
