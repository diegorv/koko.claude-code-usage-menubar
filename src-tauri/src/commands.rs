use std::sync::{LazyLock, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager};

static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("ClaudeUsageMenubar")
        .build()
        .expect("failed to build HTTP client")
});

static TOKEN_CACHE: LazyLock<Mutex<Option<(String, Instant)>>> =
    LazyLock::new(|| Mutex::new(None));

const TOKEN_CACHE_TTL_SECS: u64 = 86400; // 24 hours — only re-reads keychain on auth errors

static POLLING_HANDLE: LazyLock<Mutex<Option<tauri::async_runtime::JoinHandle<()>>>> =
    LazyLock::new(|| Mutex::new(None));

static LAST_FETCH: LazyLock<Mutex<Option<Instant>>> = LazyLock::new(|| Mutex::new(None));
static LAST_PAYLOAD: LazyLock<Mutex<Option<UsagePayload>>> = LazyLock::new(|| Mutex::new(None));

const MIN_FETCH_INTERVAL_SECS: u64 = 30;

// --- Event payloads ---

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsagePayload {
    pub status: String,
    pub session_percent: u32,
    pub session_resets_at: Option<String>,
    pub weekly_percent: u32,
    pub weekly_resets_at: Option<String>,
    pub models: Vec<ModelPayload>,
    pub last_updated_at: u64,
    pub error_message: Option<String>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPayload {
    pub name: String,
    pub percent: u32,
    pub resets_at: Option<String>,
}

impl UsagePayload {
    fn error(status: &str, message: &str) -> Self {
        Self {
            status: status.to_string(),
            session_percent: 0,
            session_resets_at: None,
            weekly_percent: 0,
            weekly_resets_at: None,
            models: vec![],
            last_updated_at: now_millis(),
            error_message: Some(message.to_string()),
        }
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// --- Token management ---

fn get_cached_token() -> Result<String, String> {
    {
        let cache = TOKEN_CACHE.lock().map_err(|e| format!("Lock error: {}", e))?;
        if let Some((ref token, ref cached_at)) = *cache {
            if cached_at.elapsed().as_secs() < TOKEN_CACHE_TTL_SECS {
                return Ok(token.clone());
            }
        }
    }

    let token = read_token_from_keychain()?;

    {
        let mut cache = TOKEN_CACHE.lock().map_err(|e| format!("Lock error: {}", e))?;
        *cache = Some((token.clone(), Instant::now()));
    }

    Ok(token)
}

fn invalidate_cache() {
    if let Ok(mut cache) = TOKEN_CACHE.lock() {
        *cache = None;
    }
}

fn read_token_from_keychain() -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        use security_framework::passwords::get_generic_password;

        let username = std::env::var("USER")
            .map_err(|_| "Could not get username".to_string())?;

        let password_data = get_generic_password("Claude Code-credentials", &username)
            .map_err(|e| format!("Failed to read keychain: {}", e))?;

        let json_str = String::from_utf8(password_data.to_vec())
            .map_err(|e| format!("Invalid UTF-8 in keychain: {}", e))?;

        parse_keychain_json(&json_str)
    }

    #[cfg(not(target_os = "macos"))]
    {
        Err("Keychain access only available on macOS".to_string())
    }
}

// --- Usage fetching (internal) ---

async fn fetch_usage_payload() -> UsagePayload {
    if let Ok(mut last) = LAST_FETCH.lock() {
        *last = Some(Instant::now());
    }

    let token = match get_cached_token() {
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
        .get("https://api.anthropic.com/api/oauth/usage")
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

    let status_code = response.status();
    if !status_code.is_success() {
        if status_code.as_u16() == 401 || status_code.as_u16() == 403 {
            invalidate_cache();
            return UsagePayload::error(
                "unauthorized",
                "Token expired. Run \"claude login\" to re-authenticate.",
            );
        }
        if status_code.as_u16() == 429 {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok());
            let msg = match retry_after {
                Some(secs) => format!("Rate limited. Try again in {}s.", secs),
                None => "Rate limited. Please try again later.".to_string(),
            };
            return UsagePayload::error("error", &msg);
        }
        let body = response.text().await.unwrap_or_default();
        return UsagePayload::error("error", &format!("HTTP {}: {}", status_code.as_u16(), body));
    }

    let body = match response.text().await {
        Ok(b) => b,
        Err(e) => {
            return UsagePayload::error("error", &format!("Failed to read response: {}", e))
        }
    };

    let json: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => return UsagePayload::error("error", &format!("Invalid JSON: {}", e)),
    };

    let payload = parse_api_response(&json);

    // Cache successful payload for use when throttled
    if let Ok(mut cached) = LAST_PAYLOAD.lock() {
        *cached = Some(payload.clone());
    }

    payload
}

fn parse_api_response(json: &serde_json::Value) -> UsagePayload {
    let clamp = |v: f64| v.max(0.0).min(100.0).round() as u32;

    let mut models = Vec::new();
    if let Some(util) = json["seven_day_sonnet"]["utilization"].as_f64() {
        models.push(ModelPayload {
            name: "Sonnet".to_string(),
            percent: clamp(util),
            resets_at: json["seven_day_sonnet"]["resets_at"]
                .as_str()
                .map(String::from),
        });
    }
    if let Some(util) = json["seven_day_opus"]["utilization"].as_f64() {
        models.push(ModelPayload {
            name: "Opus".to_string(),
            percent: clamp(util),
            resets_at: json["seven_day_opus"]["resets_at"]
                .as_str()
                .map(String::from),
        });
    }

    UsagePayload {
        status: "ok".to_string(),
        session_percent: clamp(json["five_hour"]["utilization"].as_f64().unwrap_or(0.0)),
        session_resets_at: json["five_hour"]["resets_at"]
            .as_str()
            .map(String::from),
        weekly_percent: clamp(json["seven_day"]["utilization"].as_f64().unwrap_or(0.0)),
        weekly_resets_at: json["seven_day"]["resets_at"]
            .as_str()
            .map(String::from),
        models,
        last_updated_at: now_millis(),
        error_message: None,
    }
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

async fn do_refresh_cycle(app: &AppHandle) {
    let payload = fetch_usage_payload().await;
    update_tray_icon(app, &payload);
    let _ = app.emit("usage_updated", &payload);
}

/// Emits the last cached payload to the frontend (used when popup is shown).
pub fn emit_cached_payload(app: &AppHandle) {
    if let Ok(cached) = LAST_PAYLOAD.lock() {
        if let Some(ref payload) = *cached {
            let _ = app.emit("usage_updated", payload);
        }
    }
}

// --- Polling management ---

/// Starts (or restarts) the native polling timer. Called from setup and from frontend.
pub fn start_polling_internal(app: AppHandle, interval_secs: u64) {
    if let Ok(mut handle) = POLLING_HANDLE.lock() {
        if let Some(h) = handle.take() {
            h.abort();
        }

        let jh = tauri::async_runtime::spawn(async move {
            do_refresh_cycle(&app).await;

            loop {
                tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
                do_refresh_cycle(&app).await;
            }
        });

        *handle = Some(jh);
    }
}

// --- Tauri commands ---

/// Starts or restarts auto-refresh with the given interval
#[tauri::command]
pub async fn start_auto_refresh(app: AppHandle, interval_secs: u64) -> Result<(), String> {
    start_polling_internal(app, interval_secs);
    Ok(())
}

/// Triggers a single immediate refresh and returns the data to the caller.
/// Skips the API call if data was fetched less than MIN_FETCH_INTERVAL_SECS ago.
#[tauri::command]
pub async fn trigger_refresh(app: AppHandle) -> Result<UsagePayload, String> {
    if let Ok(last) = LAST_FETCH.lock() {
        if let Some(t) = *last {
            if t.elapsed().as_secs() < MIN_FETCH_INTERVAL_SECS {
                if let Ok(cached) = LAST_PAYLOAD.lock() {
                    if let Some(ref payload) = *cached {
                        return Ok(payload.clone());
                    }
                }
            }
        }
    }

    let payload = fetch_usage_payload().await;
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
pub async fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// Extracts access token from keychain JSON
pub fn parse_keychain_json(json_str: &str) -> Result<String, String> {
    let json: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| format!("Invalid JSON: {}", e))?;

    json.get("claudeAiOauth")
        .and_then(|oauth| oauth.get("accessToken"))
        .and_then(|token| token.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "No accessToken found in keychain data".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_keychain_json_valid() {
        let json = r#"{"claudeAiOauth": {"accessToken": "sk-ant-test123"}}"#;
        let result = parse_keychain_json(json);
        assert_eq!(result.unwrap(), "sk-ant-test123");
    }

    #[test]
    fn test_parse_keychain_json_missing_oauth() {
        let json = r#"{"otherKey": "value"}"#;
        let result = parse_keychain_json(json);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No accessToken"));
    }

    #[test]
    fn test_parse_keychain_json_missing_token() {
        let json = r#"{"claudeAiOauth": {"refreshToken": "rt-123"}}"#;
        let result = parse_keychain_json(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_keychain_json_invalid_json() {
        let result = parse_keychain_json("not json");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid JSON"));
    }

    #[test]
    fn test_parse_api_response_ok() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{
                "five_hour": {"utilization": 45.0, "resets_at": "2024-01-01T00:00:00Z"},
                "seven_day": {"utilization": 67.0, "resets_at": "2024-01-07T00:00:00Z"},
                "seven_day_sonnet": {"utilization": 30.0},
                "seven_day_opus": {"utilization": 80.0}
            }"#,
        )
        .unwrap();
        let payload = parse_api_response(&json);
        assert_eq!(payload.status, "ok");
        assert_eq!(payload.session_percent, 45);
        assert_eq!(payload.weekly_percent, 67);
        assert_eq!(payload.models.len(), 2);
        assert_eq!(payload.models[0].name, "Sonnet");
        assert_eq!(payload.models[0].percent, 30);
        assert_eq!(payload.models[1].name, "Opus");
        assert_eq!(payload.models[1].percent, 80);
    }

    #[test]
    fn test_parse_api_response_clamps() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{"five_hour": {"utilization": 150.0}, "seven_day": {"utilization": -10.0}}"#,
        )
        .unwrap();
        let payload = parse_api_response(&json);
        assert_eq!(payload.session_percent, 100);
        assert_eq!(payload.weekly_percent, 0);
    }
}
