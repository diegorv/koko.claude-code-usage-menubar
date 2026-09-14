use std::sync::LazyLock;
use image::Rgba;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::gpt_parser;
use crate::kimi_parser;
use crate::parser::{self, ProviderPayload, ProviderStatus, UsagePayload};
use crate::state::{PayloadCache, ProviderSettings, ProviderSettingsState, TokenCache, UsagePoller};

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
const GPT_API_URL: &str = "https://chatgpt.com/backend-api/wham/usage";

// --- Usage fetching (internal) ---

/// Runs a blocking keychain call on the blocking pool. Both providers start
/// with one, and `/usr/bin/security` can take up to SECURITY_CMD_TIMEOUT to
/// answer — long enough to matter for a worker thread.
async fn on_keychain_thread<F, R>(work: F) -> Result<R, String>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| format!("Keychain task failed: {}", e))
}

/// Fetches the enabled providers concurrently and assembles one payload, in a
/// fixed order: Claude, Kimi, GPT. A provider that is disabled in Settings is
/// never fetched — no keychain read, no request — and is omitted, as is Kimi
/// without a key and GPT without a Codex login, so a keyless install with
/// default settings behaves exactly as a Claude-only build.
///
/// Takes the `AppHandle` rather than the state refs so each provider's
/// keychain read can be moved to the blocking pool, which needs an owned
/// 'static handle. Without that the `join!` below was concurrent in name
/// only: it polls in order, and each branch opened with a synchronous
/// subprocess, so the reads ran back to back before any request went out.
async fn fetch_usage_payload(app: &AppHandle) -> UsagePayload {
    let payload_cache = app.state::<PayloadCache>();
    payload_cache.mark_fetch_start();

    let settings = app.state::<ProviderSettingsState>().get();
    let (claude, kimi, gpt) = tokio::join!(
        async {
            if settings.claude.enabled {
                Some(fetch_claude_provider(app.clone()).await)
            } else {
                None
            }
        },
        async {
            if settings.kimi.enabled {
                fetch_kimi_provider().await
            } else {
                None
            }
        },
        async {
            if settings.gpt.enabled {
                fetch_gpt_provider().await
            } else {
                None
            }
        },
    );

    let providers = [claude, kimi, gpt].into_iter().flatten().collect();
    let payload = UsagePayload::new(providers);

    // Stored unconditionally. Keeping the last *ok* payload instead — which is
    // what the single-provider version did — meant a failing Claude froze the
    // whole cache, including a Kimi provider that had just answered fine: the
    // popup opened on a payload that could be days old and said so only in a
    // stale `lastUpdatedAt`, and a Kimi key removed while Claude was down went
    // on being rendered from that frozen copy. The cache is now simply "what
    // the last fetch produced", which is the only thing it can honestly claim.
    //
    // The tray still freezes on error — see `tray_rows` — because a tray icon
    // is one baked image with no way to say "this is stale".
    payload_cache.store(payload.clone());

    payload
}

async fn fetch_claude_provider(app: AppHandle) -> ProviderPayload {
    let read_app = app.clone();
    let token = match on_keychain_thread(move || read_app.state::<TokenCache>().get_or_read()).await
    {
        Ok(Ok(t)) => t,
        Err(e) | Ok(Err(e)) => {
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
        app.state::<TokenCache>().invalidate();
    }

    provider
}

/// `None` when no key is usable — the provider is omitted from the payload,
/// never rendered as an error. A keychain infra failure reads as "no key":
/// the alternative is an error row every poll for a condition that usually
/// heals on the next cycle. On 401 the key is kept (kimi_parser classifies;
/// nobody deletes).
async fn fetch_kimi_provider() -> Option<ProviderPayload> {
    let key = match on_keychain_thread(crate::state::kimi_key::read).await {
        Ok(Ok(Some(key))) => key,
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

/// `None` when Codex CLI has never been logged in on this machine (no auth
/// file). An auth file without a ChatGPT login is shown as an auth error:
/// the user turned GPT on, so silence would look like a bug.
///
/// The auth file is a plain read of a small local file, not a subprocess, so
/// unlike the keychain reads it stays inline.
async fn fetch_gpt_provider() -> Option<ProviderPayload> {
    let creds = match crate::state::gpt_auth::read() {
        Ok(Some(creds)) => creds,
        Ok(None) => return None,
        Err(e) => return Some(gpt_parser::error_payload(ProviderStatus::AuthError, &e)),
    };

    let mut request = HTTP_CLIENT
        .get(GPT_API_URL)
        .header("Authorization", format!("Bearer {}", creds.access_token))
        .header("Accept", "application/json");
    // Selects the workspace when the login belongs to more than one.
    if let Some(account_id) = &creds.account_id {
        request = request.header("ChatGPT-Account-Id", account_id);
    }

    let response = match request.send().await {
        Ok(r) => r,
        Err(e) => {
            return Some(gpt_parser::error_payload(
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
            return Some(gpt_parser::error_payload(
                ProviderStatus::Error,
                &format!("Failed to read response: {}", e),
            ))
        }
    };

    Some(gpt_parser::classify(status, retry_after, &body))
}

// --- Refresh cycle ---

/// Per-provider tray identity, in one place: the one-letter row label, the
/// bar color, and the name the tooltip uses.
fn tray_identity(id: &str) -> (char, Rgba<u8>, &'static str) {
    match id {
        "kimi" => ('K', crate::tray_icon::COLOR_KIMI, "Kimi"),
        "gpt" => ('G', crate::tray_icon::COLOR_GPT, "GPT"),
        _ => ('C', crate::tray_icon::COLOR_CLAUDE, "Claude"),
    }
}

/// The ok providers that actually reach the icon, cap included. Both the rows
/// and the tooltip are derived from this, so the tooltip can never name a
/// provider the icon isn't showing.
///
/// Only providers picked for the tray in Settings are eligible. A picked
/// provider that is failing drops out with no stand-in: borrowing an unpicked
/// one would put on the icon numbers the user chose to keep in the popup. The
/// exception is a payload holding none of the picks at all (a pick with no
/// key or no Codex login) — then payload order decides, as it did before the
/// setting existed, instead of the icon never updating again.
fn painted_providers<'a>(
    payload: &'a UsagePayload,
    settings: &ProviderSettings,
) -> Vec<&'a ProviderPayload> {
    let any_picked = payload.providers.iter().any(|p| settings.in_tray(&p.id));
    payload
        .providers
        .iter()
        .filter(|p| !any_picked || settings.in_tray(&p.id))
        .filter(|p| p.status == ProviderStatus::Ok)
        .take(crate::tray_icon::MAX_ROWS)
        .collect()
}

/// Picks what the tray shows, as `(label, session, weekly, color)` grid rows,
/// percentages in 0.0..=1.0. One row per painted provider, carrying both of
/// its figures — a second provider costs a row, not the session numbers. A
/// non-ok provider drops out instead of freezing the others: a bad Kimi key
/// must not pin stale Claude data.
///
/// Returns None when nothing is paintable, leaving the icon untouched: it's a
/// single baked image showing nothing fresh, so the freeze-on-error semantics
/// the Claude-only tray has always had still apply. At most MAX_ROWS rows are
/// returned — more would paint outside the 22px design height.
fn tray_rows(
    payload: &UsagePayload,
    settings: &ProviderSettings,
) -> Option<Vec<crate::tray_icon::TrayRow>> {
    let painted = painted_providers(payload, settings);
    if painted.is_empty() {
        return None;
    }
    Some(
        painted
            .iter()
            .map(|p| {
                let (label, color, _) = tray_identity(&p.id);
                (
                    label,
                    p.session_percent.map(|s| s as f64 / 100.0),
                    p.weekly_percent as f64 / 100.0,
                    color,
                )
            })
            .collect(),
    )
}

/// Names exactly the providers the icon is showing.
fn tray_tooltip(payload: &UsagePayload, settings: &ProviderSettings) -> String {
    let names: Vec<&str> = painted_providers(payload, settings)
        .iter()
        .map(|p| tray_identity(&p.id).2)
        .collect();
    format!("{} Usage", names.join(" + "))
}

fn update_tray_icon(app: &AppHandle, payload: &UsagePayload) {
    let settings = app.state::<ProviderSettingsState>().get();
    let Some(rows) = tray_rows(payload, &settings) else {
        return;
    };
    let icon = crate::tray_icon::generate_icon(rows);
    if let Some(tray) = app.tray_by_id("main-tray") {
        let _ = tray.set_icon(Some(icon));
        let _ = tray.set_title(None::<&str>);
        // The builder-time tooltip predates the second provider.
        let _ = tray.set_tooltip(Some(tray_tooltip(payload, &settings)));
    }
}

pub async fn do_refresh_cycle(app: &AppHandle) {
    let payload = fetch_usage_payload(app).await;
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
pub async fn trigger_refresh(app: AppHandle) -> Result<UsagePayload, String> {
    if let Some(cached) = app
        .state::<PayloadCache>()
        .cached_if_fresh(MIN_FETCH_INTERVAL_SECS)
    {
        return Ok(cached);
    }

    let payload = fetch_usage_payload(&app).await;
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

/// Replaces the provider toggles and spawns a refresh cycle, so enabling a
/// provider fills its popup section and a tray pick repaints the icon without
/// waiting for the next poll — same throttle rationale as `save_kimi_key`.
/// The frontend persists the settings; this only updates the copy the poller
/// reads.
#[tauri::command]
pub async fn set_provider_settings(
    app: AppHandle,
    settings: ProviderSettings,
) -> Result<(), String> {
    app.state::<ProviderSettingsState>().set(settings);
    tauri::async_runtime::spawn(async move {
        do_refresh_cycle(&app).await;
    });
    Ok(())
}

// --- Kimi API key management ---

/// These three are async for the same reason `on_keychain_thread` exists:
/// `#[tauri::command]` on a *non-async* fn compiles to a blocking handler that
/// runs inline on the IPC thread, so a slow keychain would freeze the app.
///
/// Stores the Kimi API key in the macOS Keychain (updates in place). On
/// success, spawns a refresh cycle so the popup gains the Kimi section via
/// `usage_updated` immediately — `trigger_refresh`'s 30s throttle would
/// otherwise keep serving the pre-save payload and the save would look
/// failed. `do_refresh_cycle` is the poll path and has no throttle.
#[tauri::command]
pub async fn save_kimi_key(app: AppHandle, key: String) -> Result<(), String> {
    on_keychain_thread(move || crate::state::kimi_key::save(&key)).await??;
    tauri::async_runtime::spawn(async move {
        do_refresh_cycle(&app).await;
    });
    Ok(())
}

/// Removes the Kimi API key from the macOS Keychain. On success, spawns a
/// refresh cycle so the popup drops the Kimi section immediately (same
/// throttle rationale as `save_kimi_key`).
#[tauri::command]
pub async fn delete_kimi_key(app: AppHandle) -> Result<(), String> {
    on_keychain_thread(crate::state::kimi_key::remove).await??;
    tauri::async_runtime::spawn(async move {
        do_refresh_cycle(&app).await;
    });
    Ok(())
}

/// Whether a Kimi API key is stored. The key itself never crosses the IPC
/// boundary — booleans only.
///
/// A keychain infrastructure failure (a timed-out `security` call, a denied
/// authorization) is an `Err`, not a `false`. Collapsing the two told the
/// user "Not saved" about a key that was sitting in the keychain the whole
/// time, and the obvious next move — paste it again — is the one thing that
/// cannot help.
#[tauri::command]
pub async fn has_kimi_key() -> Result<bool, String> {
    on_keychain_thread(crate::state::kimi_key::exists).await?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::ProviderExtra;
    use crate::state::provider_settings::ProviderToggle;

    fn provider(id: &str, status: ProviderStatus, session: u32, weekly: u32) -> ProviderPayload {
        ProviderPayload {
            id: id.to_string(),
            title: format!("{} Usage", id),
            status,
            session_percent: Some(session),
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

    fn defaults() -> ProviderSettings {
        ProviderSettings::default()
    }

    /// All three enabled, with the given tray picks.
    fn tray_picks(claude: bool, kimi: bool, gpt: bool) -> ProviderSettings {
        let on = |tray| ProviderToggle { enabled: true, tray };
        ProviderSettings {
            claude: on(claude),
            kimi: on(kimi),
            gpt: on(gpt),
        }
    }

    fn labels(rows: &[crate::tray_icon::TrayRow]) -> Vec<char> {
        rows.iter().map(|r| r.0).collect()
    }

    #[test]
    fn single_provider_gets_one_row_with_both_figures() {
        let payload = UsagePayload::new(vec![provider("claude", ProviderStatus::Ok, 45, 67)]);
        let rows = tray_rows(&payload, &defaults()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, 'C');
        assert_eq!(rows[0].1, Some(45.0 / 100.0));
        assert_eq!(rows[0].2, 67.0 / 100.0);
        assert_eq!(rows[0].3, crate::tray_icon::COLOR_CLAUDE);
    }

    #[test]
    fn two_providers_show_session_and_weekly_per_provider() {
        let payload = UsagePayload::new(vec![
            provider("claude", ProviderStatus::Ok, 45, 67),
            provider("kimi", ProviderStatus::Ok, 96, 19),
        ]);
        let rows = tray_rows(&payload, &defaults()).unwrap();
        assert_eq!(rows.len(), 2);
        // Both figures survive the second provider — the whole point of the
        // grid layout. A second provider costs a row, not the session numbers.
        assert_eq!(rows[0].0, 'C');
        assert_eq!(rows[0].1, Some(45.0 / 100.0));
        assert_eq!(rows[0].2, 67.0 / 100.0);
        assert_eq!(rows[1].0, 'K');
        assert_eq!(rows[1].1, Some(96.0 / 100.0));
        assert_eq!(rows[1].2, 19.0 / 100.0);
        // Distinct per-provider colors — the whole row is painted in one.
        assert_eq!(rows[0].3, crate::tray_icon::COLOR_CLAUDE);
        assert_eq!(rows[1].3, crate::tray_icon::COLOR_KIMI);
        assert_ne!(rows[0].3, rows[1].3);
    }

    #[test]
    fn non_ok_claude_skips_update() {
        // Single provider, nothing fresh to show — the freeze-on-error
        // semantics the Claude-only tray has always had.
        let payload = UsagePayload::new(vec![provider("claude", ProviderStatus::AuthError, 0, 0)]);
        assert!(tray_rows(&payload, &defaults()).is_none());
    }

    #[test]
    fn non_ok_kimi_keeps_claudes_row() {
        // A bad Kimi key must not freeze fresh Claude data: the failing
        // provider drops out and the survivor keeps its whole row.
        let payload = UsagePayload::new(vec![
            provider("claude", ProviderStatus::Ok, 45, 67),
            provider("kimi", ProviderStatus::RateLimited, 0, 0),
        ]);
        let rows = tray_rows(&payload, &defaults()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, 'C');
        assert_eq!(rows[0].1, Some(45.0 / 100.0));
        assert_eq!(rows[0].2, 67.0 / 100.0);
        assert_eq!(rows[0].3, crate::tray_icon::COLOR_CLAUDE);
    }

    #[test]
    fn non_ok_claude_keeps_kimis_row() {
        let payload = UsagePayload::new(vec![
            provider("claude", ProviderStatus::AuthError, 0, 0),
            provider("kimi", ProviderStatus::Ok, 96, 19),
        ]);
        let rows = tray_rows(&payload, &defaults()).unwrap();
        assert_eq!(rows.len(), 1);
        // Kimi's label and color, not Claude's: this row is the whole icon, so
        // painting it in Claude's purple made Kimi's numbers read as Claude's.
        assert_eq!(rows[0].0, 'K');
        assert_eq!(rows[0].1, Some(96.0 / 100.0));
        assert_eq!(rows[0].2, 19.0 / 100.0);
        assert_eq!(rows[0].3, crate::tray_icon::COLOR_KIMI);
    }

    #[test]
    fn weekly_only_provider_paints_no_session_figure() {
        // No session window is carried through as None, so the icon can say
        // "--" instead of a 0% that looks like a real, idle figure.
        let mut gpt = provider("gpt", ProviderStatus::Ok, 0, 1);
        gpt.session_percent = None;
        let payload = UsagePayload::new(vec![gpt]);
        let rows = tray_rows(&payload, &tray_picks(false, false, true)).unwrap();
        assert_eq!(rows[0].0, 'G');
        assert_eq!(rows[0].1, None);
        assert_eq!(rows[0].2, 1.0 / 100.0);
    }

    #[test]
    fn tooltip_names_only_the_providers_on_the_icon() {
        let both = UsagePayload::new(vec![
            provider("claude", ProviderStatus::Ok, 45, 67),
            provider("kimi", ProviderStatus::Ok, 96, 19),
        ]);
        assert_eq!(tray_tooltip(&both, &defaults()), "Claude + Kimi Usage");

        // Kimi is configured but failing, so it is not on the icon and must
        // not be in the tooltip either.
        let kimi_down = UsagePayload::new(vec![
            provider("claude", ProviderStatus::Ok, 45, 67),
            provider("kimi", ProviderStatus::AuthError, 0, 0),
        ]);
        assert_eq!(tray_tooltip(&kimi_down, &defaults()), "Claude Usage");

        let claude_down = UsagePayload::new(vec![
            provider("claude", ProviderStatus::AuthError, 0, 0),
            provider("kimi", ProviderStatus::Ok, 96, 19),
        ]);
        assert_eq!(tray_tooltip(&claude_down, &defaults()), "Kimi Usage");
    }

    #[test]
    fn tooltip_respects_the_row_cap() {
        let payload = UsagePayload::new(vec![
            provider("claude", ProviderStatus::Ok, 45, 67),
            provider("kimi", ProviderStatus::Ok, 96, 19),
            provider("gpt", ProviderStatus::Ok, 10, 20),
        ]);
        assert_eq!(
            tray_tooltip(&payload, &tray_picks(true, true, true)),
            "Claude + Kimi Usage"
        );
    }

    #[test]
    fn both_providers_non_ok_skips_update() {
        let payload = UsagePayload::new(vec![
            provider("claude", ProviderStatus::Error, 0, 0),
            provider("kimi", ProviderStatus::AuthError, 0, 0),
        ]);
        assert!(tray_rows(&payload, &defaults()).is_none());
    }

    #[test]
    fn rows_are_capped_at_max_rows() {
        // A third ok provider would paint below the 22px design height — it
        // drops out instead of clipping silently, even if picked.
        let payload = UsagePayload::new(vec![
            provider("claude", ProviderStatus::Ok, 45, 67),
            provider("kimi", ProviderStatus::Ok, 96, 19),
            provider("gpt", ProviderStatus::Ok, 10, 20),
        ]);
        let rows = tray_rows(&payload, &tray_picks(true, true, true)).unwrap();
        assert_eq!(rows.len(), crate::tray_icon::MAX_ROWS);
        assert_eq!(labels(&rows), vec!['C', 'K']);
    }

    #[test]
    fn with_all_three_on_the_unpicked_one_stays_in_the_popup() {
        let payload = UsagePayload::new(vec![
            provider("claude", ProviderStatus::Ok, 45, 67),
            provider("kimi", ProviderStatus::Ok, 96, 19),
            provider("gpt", ProviderStatus::Ok, 10, 20),
        ]);
        let settings = tray_picks(false, true, true);
        let rows = tray_rows(&payload, &settings).unwrap();
        assert_eq!(labels(&rows), vec!['K', 'G']);
        assert_eq!(rows[1].1, Some(10.0 / 100.0));
        assert_eq!(rows[1].3, crate::tray_icon::COLOR_GPT);
        assert_eq!(tray_tooltip(&payload, &settings), "Kimi + GPT Usage");
    }

    #[test]
    fn an_unpicked_provider_stays_off_the_tray_even_with_a_free_slot() {
        let payload = UsagePayload::new(vec![
            provider("claude", ProviderStatus::Ok, 45, 67),
            provider("kimi", ProviderStatus::Ok, 96, 19),
        ]);
        let rows = tray_rows(&payload, &tray_picks(true, false, false)).unwrap();
        assert_eq!(labels(&rows), vec!['C']);
    }

    #[test]
    fn a_failing_pick_does_not_borrow_an_unpicked_provider() {
        // The user kept Kimi in the popup only; a Claude outage must not put
        // Kimi's numbers on the icon in Claude's place.
        let payload = UsagePayload::new(vec![
            provider("claude", ProviderStatus::AuthError, 0, 0),
            provider("kimi", ProviderStatus::Ok, 96, 19),
        ]);
        assert!(tray_rows(&payload, &tray_picks(true, false, false)).is_none());
    }

    #[test]
    fn no_pick_in_the_payload_falls_back_to_payload_order() {
        // GPT is the only pick but has no Codex login, so it isn't in the
        // payload. Freezing forever would be worse than showing what's there.
        let payload = UsagePayload::new(vec![
            provider("claude", ProviderStatus::Ok, 45, 67),
            provider("kimi", ProviderStatus::Ok, 96, 19),
        ]);
        let rows = tray_rows(&payload, &tray_picks(false, false, true)).unwrap();
        assert_eq!(labels(&rows), vec!['C', 'K']);
    }

    #[test]
    fn empty_providers_skips_update() {
        let payload = UsagePayload::new(vec![]);
        assert!(tray_rows(&payload, &defaults()).is_none());
    }
}
