//! ChatGPT/Codex plan usage provider (chatgpt.com/backend-api/wham/usage):
//! pure translation from an HTTP response to a `ProviderPayload`. Mirrors
//! kimi_parser.rs — same status classification, same drift rules. The endpoint
//! is undocumented (it is what Codex CLI reads for its own rate-limit display),
//! so a silent reshape is more likely here than anywhere, and shape_warning is
//! how it surfaces.

use crate::parser::{truncate_body, ModelPayload, ProviderExtra, ProviderPayload, ProviderStatus};

const GPT_ID: &str = "gpt";
const GPT_TITLE: &str = "GPT Usage";

/// The popup labels the rows "Session (5h)" and "Weekly"; these are what make
/// those labels true.
const SESSION_WINDOW_SECS: u64 = 5 * 60 * 60;
const WEEKLY_WINDOW_SECS: u64 = 7 * 24 * 60 * 60;

pub fn classify(status: u16, retry_after: Option<u64>, body: &str) -> ProviderPayload {
    match status {
        200..=299 => match serde_json::from_str::<serde_json::Value>(body) {
            Ok(json) => parse_api_response(&json),
            Err(e) => error_payload(ProviderStatus::Error, &format!("Invalid JSON: {}", e)),
        },
        // The token belongs to Codex CLI, which refreshes it when it runs.
        // Nothing here refreshes or deletes it — see state/gpt_auth.rs.
        401 | 403 => error_payload(
            ProviderStatus::AuthError,
            "ChatGPT login expired. Run Codex (or \"codex login\") to refresh it.",
        ),
        429 => {
            let msg = match retry_after {
                Some(secs) => format!("Rate limited. Try again in {}s.", secs),
                None => "Rate limited. Please try again later.".to_string(),
            };
            error_payload(ProviderStatus::RateLimited, &msg)
        }
        s => error_payload(
            ProviderStatus::Error,
            &format!("HTTP {}: {}", s, truncate_body(body)),
        ),
    }
}

/// Transport-level failures get the same shape as classified HTTP errors.
/// `message` must never contain the token.
pub fn error_payload(status: ProviderStatus, message: &str) -> ProviderPayload {
    ProviderPayload {
        id: GPT_ID.to_string(),
        title: GPT_TITLE.to_string(),
        status,
        session_percent: Some(0),
        session_resets_at: None,
        weekly_percent: 0,
        weekly_resets_at: None,
        models: vec![],
        extra: ProviderExtra::None,
        error_message: Some(message.to_string()),
        shape_warning: None,
    }
}

/// Finds a window by its length, not by `primary_window`/`secondary_window`.
/// Which slot holds which window is an assumption the day it changes, the
/// numbers would swap under the right labels with nothing to warn — the same
/// lesson as Kimi's session bucket.
fn find_window(json: &serde_json::Value, secs: u64) -> Option<&serde_json::Value> {
    ["primary_window", "secondary_window"]
        .iter()
        .map(|key| &json["rate_limit"][*key])
        .find(|window| window["limit_window_seconds"].as_u64() == Some(secs))
}

pub(crate) fn parse_api_response(json: &serde_json::Value) -> ProviderPayload {
    let session = find_window(json, SESSION_WINDOW_SECS);
    let weekly = find_window(json, WEEKLY_WINDOW_SECS);
    // A missing session window is not drift: since July 2026 some plans get
    // only the weekly window, in `primary_window`, with `secondary_window:
    // null`. What still warns is a missing weekly window, or a window of a
    // length we don't know — that is a resize, and dropping it silently would
    // hide it.
    let unknown_window = ["primary_window", "secondary_window"]
        .iter()
        .map(|key| &json["rate_limit"][*key])
        .filter(|window| !window.is_null())
        .any(|window| {
            !matches!(
                window["limit_window_seconds"].as_u64(),
                Some(SESSION_WINDOW_SECS | WEEKLY_WINDOW_SECS)
            )
        });
    let shape_warning = (weekly.is_none() || unknown_window).then(|| {
        "Unexpected API response shape — some usage data may be missing.".to_string()
    });

    ProviderPayload {
        id: GPT_ID.to_string(),
        title: GPT_TITLE.to_string(),
        status: ProviderStatus::Ok,
        session_percent: session.map(used_percent),
        session_resets_at: session.and_then(resets_at),
        weekly_percent: weekly.map(used_percent).unwrap_or(0),
        weekly_resets_at: weekly.and_then(resets_at),
        // Separate per-model limits (GPT-5.3-Codex-Spark so far), the
        // counterpart of Claude's weekly_scoped entries. Shown by their weekly
        // window, like Claude's models; an entry without one is skipped. Not
        // every plan has any, so their absence is not drift.
        models: json["additional_rate_limits"]
            .as_array()
            .map(|limits| {
                limits
                    .iter()
                    .filter_map(|limit| {
                        let name = limit["limit_name"].as_str()?;
                        let weekly = find_window(limit, WEEKLY_WINDOW_SECS)?;
                        Some(ModelPayload {
                            name: name.to_string(),
                            percent: used_percent(weekly),
                            resets_at: resets_at(weekly),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        extra: ProviderExtra::None,
        error_message: None,
        shape_warning,
    }
}

fn used_percent(window: &serde_json::Value) -> u32 {
    window["used_percent"]
        .as_f64()
        .unwrap_or(0.0)
        .max(0.0)
        .min(100.0)
        .round() as u32
}

/// `reset_at` is Unix seconds; the popup parses ISO 8601 like the other two
/// providers send.
fn resets_at(window: &serde_json::Value) -> Option<String> {
    window["reset_at"].as_i64().map(unix_to_rfc3339)
}

/// Civil-from-days (Howard Hinnant's algorithm), UTC. Small enough not to be
/// worth a date crate for one field.
fn unix_to_rfc3339(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year,
        month,
        day,
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-written, not captured: modelled on the shape Codex CLI reads.
    /// Replace with a neutralised live capture in fixtures/ once one exists,
    /// the way parser.rs and kimi_parser.rs pin theirs.
    const OK_BODY: &str = r#"{
        "plan_type": "plus",
        "rate_limit": {
            "allowed": true,
            "limit_reached": false,
            "primary_window": {"used_percent": 12, "limit_window_seconds": 18000,
                               "reset_after_seconds": 3600, "reset_at": 1700000000},
            "secondary_window": {"used_percent": 40, "limit_window_seconds": 604800,
                                 "reset_after_seconds": 86400, "reset_at": 1700086400}
        },
        "credits": {"has_credits": false, "unlimited": false, "balance": null}
    }"#;

    #[test]
    fn parses_session_and_weekly() {
        let payload = classify(200, None, OK_BODY);
        assert_eq!(payload.status, ProviderStatus::Ok);
        assert_eq!(payload.id, "gpt");
        assert_eq!(payload.title, "GPT Usage");
        assert_eq!(payload.session_percent, Some(12));
        assert_eq!(payload.session_resets_at.as_deref(), Some("2023-11-14T22:13:20Z"));
        assert_eq!(payload.weekly_percent, 40);
        assert_eq!(payload.weekly_resets_at.as_deref(), Some("2023-11-15T22:13:20Z"));
        assert!(payload.models.is_empty());
        assert_eq!(payload.shape_warning, None);
    }

    #[test]
    fn windows_are_found_by_length_not_by_slot() {
        let payload = parse_api_response(&serde_json::json!({
            "rate_limit": {
                "primary_window": {"used_percent": 40, "limit_window_seconds": 604800},
                "secondary_window": {"used_percent": 12, "limit_window_seconds": 18000}
            }
        }));
        assert_eq!(payload.session_percent, Some(12));
        assert_eq!(payload.weekly_percent, 40);
        assert_eq!(payload.shape_warning, None);
    }

    /// The shape a live `prolite` account returned in September 2026: the
    /// weekly window in `primary_window` and no session window at all.
    #[test]
    fn weekly_only_plan_has_no_session_and_no_warning() {
        let payload = parse_api_response(&serde_json::json!({
            "plan_type": "prolite",
            "rate_limit": {
                "allowed": true,
                "limit_reached": false,
                "primary_window": {"used_percent": 1, "limit_window_seconds": 604800,
                                   "reset_after_seconds": 510847, "reset_at": 1789908822},
                "secondary_window": null
            }
        }));
        assert_eq!(payload.session_percent, None);
        assert_eq!(payload.session_resets_at, None);
        assert_eq!(payload.weekly_percent, 1);
        assert_eq!(payload.shape_warning, None);

        let value = serde_json::to_value(&payload).unwrap();
        assert_eq!(value["sessionPercent"], serde_json::Value::Null);
    }

    /// Shape of the live `additional_rate_limits` entry, September 2026.
    #[test]
    fn additional_rate_limits_become_models_by_weekly_window() {
        let payload = parse_api_response(&serde_json::json!({
            "rate_limit": {
                "primary_window": {"used_percent": 1, "limit_window_seconds": 604800},
                "secondary_window": null
            },
            "additional_rate_limits": [
                {
                    "limit_name": "GPT-5.3-Codex-Spark",
                    "metered_feature": "codex_bengalfox",
                    "rate_limit": {
                        "primary_window": {"used_percent": 70, "limit_window_seconds": 18000,
                                           "reset_at": 1789415976},
                        "secondary_window": {"used_percent": 25, "limit_window_seconds": 604800,
                                             "reset_at": 1700000000}
                    }
                },
                {"limit_name": "No weekly", "rate_limit": {
                    "primary_window": {"used_percent": 5, "limit_window_seconds": 18000}
                }}
            ]
        }));
        assert_eq!(payload.models.len(), 1);
        assert_eq!(payload.models[0].name, "GPT-5.3-Codex-Spark");
        assert_eq!(payload.models[0].percent, 25);
        assert_eq!(payload.models[0].resets_at.as_deref(), Some("2023-11-14T22:13:20Z"));
        assert_eq!(payload.shape_warning, None);
    }

    #[test]
    fn missing_rate_limit_warns() {
        let payload = parse_api_response(&serde_json::json!({"plan_type": "plus"}));
        assert!(payload.shape_warning.is_some());
        assert_eq!(payload.session_percent, None);
    }

    #[test]
    fn a_resized_window_warns_instead_of_relabelling() {
        let payload = parse_api_response(&serde_json::json!({
            "rate_limit": {
                "primary_window": {"used_percent": 12, "limit_window_seconds": 14400},
                "secondary_window": {"used_percent": 40, "limit_window_seconds": 604800}
            }
        }));
        assert!(payload.shape_warning.is_some());
        assert_eq!(payload.session_percent, None);
        // Weekly is independent and still parsed.
        assert_eq!(payload.weekly_percent, 40);
    }

    #[test]
    fn percent_is_rounded_and_clamped() {
        let payload = parse_api_response(&serde_json::json!({
            "rate_limit": {
                "primary_window": {"used_percent": 33.4, "limit_window_seconds": 18000},
                "secondary_window": {"used_percent": 140, "limit_window_seconds": 604800}
            }
        }));
        assert_eq!(payload.session_percent, Some(33));
        assert_eq!(payload.weekly_percent, 100);
    }

    #[test]
    fn unix_to_rfc3339_known_dates() {
        assert_eq!(unix_to_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(unix_to_rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(unix_to_rfc3339(1_700_000_000), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn classify_401_returns_auth_error() {
        let payload = classify(401, None, "");
        assert_eq!(payload.status, ProviderStatus::AuthError);
        assert_eq!(payload.id, "gpt");
        assert!(payload.error_message.unwrap().contains("codex login"));
    }

    #[test]
    fn classify_429_with_retry_after() {
        let payload = classify(429, Some(42), "");
        assert_eq!(payload.status, ProviderStatus::RateLimited);
        assert!(payload.error_message.unwrap().contains("42s"));
    }

    #[test]
    fn classify_5xx_truncates_body() {
        let payload = classify(502, None, &"x".repeat(5_000));
        assert_eq!(payload.status, ProviderStatus::Error);
        let msg = payload.error_message.unwrap();
        assert!(msg.contains("HTTP 502"));
        assert!(msg.ends_with('…'));
    }

    #[test]
    fn classify_200_with_invalid_json_returns_error() {
        let payload = classify(200, None, "<html>");
        assert_eq!(payload.status, ProviderStatus::Error);
        assert!(payload.error_message.unwrap().contains("Invalid JSON"));
    }

    /// Pins the wire format the frontend consumes for a GPT provider.
    #[test]
    fn serialized_payload_has_expected_shape() {
        let value = serde_json::to_value(classify(200, None, OK_BODY)).unwrap();
        assert_eq!(value["id"], "gpt");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["sessionPercent"], 12);
        assert_eq!(value["weeklyPercent"], 40);
        assert_eq!(value["extra"], serde_json::json!({"kind": "none"}));
        assert_eq!(value["models"], serde_json::json!([]));
    }
}
