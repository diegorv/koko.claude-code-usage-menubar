//! Kimi (api.kimi.com) usage provider: pure translation from an HTTP response
//! to a `ProviderPayload`. Mirrors parser.rs — same status classification,
//! same shape_warning rationale (a silent reshape should surface, not render
//! less data). Has no side effects; the fetch layer owns the keychain read.

use crate::parser::{ModelPayload, ProviderExtra, ProviderPayload, ProviderStatus};

const KIMI_ID: &str = "kimi";
const KIMI_TITLE: &str = "Kimi Usage";

pub fn classify(status: u16, retry_after: Option<u64>, body: &str) -> ProviderPayload {
    match status {
        200..=299 => match serde_json::from_str::<serde_json::Value>(body) {
            Ok(json) => parse_api_response(&json),
            Err(e) => error_payload(
                ProviderStatus::Error,
                &format!("Invalid JSON: {}", e),
            ),
        },
        // The key stays in the keychain on auth errors — the user updates it
        // from the popup's Settings panel, it is never deleted here.
        401 | 403 => error_payload(
            ProviderStatus::AuthError,
            "API key inválida. Update it in Settings.",
        ),
        429 => {
            let msg = match retry_after {
                Some(secs) => format!("Rate limited. Try again in {}s.", secs),
                None => "Rate limited. Please try again later.".to_string(),
            };
            error_payload(ProviderStatus::RateLimited, &msg)
        }
        s => error_payload(ProviderStatus::Error, &format!("HTTP {}: {}", s, body)),
    }
}

/// Transport-level failures (request never completed, body unreadable) get the
/// same shape as classified HTTP errors. `message` must never contain the key.
pub fn error_payload(status: ProviderStatus, message: &str) -> ProviderPayload {
    ProviderPayload {
        id: KIMI_ID.to_string(),
        title: KIMI_TITLE.to_string(),
        status,
        session_percent: 0,
        session_resets_at: None,
        weekly_percent: 0,
        weekly_resets_at: None,
        models: vec![],
        extra: ProviderExtra::Parallel {
            used: 0,
            limit: 0,
        },
        error_message: Some(message.to_string()),
        shape_warning: None,
    }
}

pub(crate) fn parse_api_response(json: &serde_json::Value) -> ProviderPayload {
    // `limits[0]` is the session window (300 minutes in every observed
    // response) and `usage` is the weekly bucket. Both are load-bearing — if
    // either goes missing the API reshaped, which must warn rather than
    // quietly show zeros. An empty limits array warns too: unlike Claude's
    // `limits: []` (legitimately "nothing reported"), Kimi's only session
    // metric comes from limits[0], so empty means drift, never a real account.
    let session_entry = json["limits"].as_array().and_then(|l| l.first());
    let shape_warning = (session_entry.is_none() || !json["usage"].is_object()).then(|| {
        "Unexpected API response shape — some usage data may be missing.".to_string()
    });

    let detail = session_entry
        .map(|entry| &entry["detail"])
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    ProviderPayload {
        id: KIMI_ID.to_string(),
        title: KIMI_TITLE.to_string(),
        status: ProviderStatus::Ok,
        session_percent: quota_percent(&detail),
        session_resets_at: detail["resetTime"].as_str().map(String::from),
        weekly_percent: quota_percent(&json["usage"]),
        weekly_resets_at: json["usage"]["resetTime"].as_str().map(String::from),
        // Kimi reports no per-model breakdown.
        models: Vec::<ModelPayload>::new(),
        extra: ProviderExtra::Parallel {
            used: json["parallel"]["details"]
                .as_array()
                .map(|d| d.len() as u32)
                .unwrap_or(0),
            limit: quota_number(&json["parallel"]["limit"]).unwrap_or(0.0) as u32,
        },
        error_message: None,
        shape_warning,
    }
}

/// Quota numbers arrive as JSON strings ("limit": "100"). Percentages are
/// computed, not provided: used/limit*100, clamped like the Claude parser.
fn quota_percent(quota: &serde_json::Value) -> u32 {
    let limit = quota_number(&quota["limit"]).unwrap_or(0.0);
    if limit <= 0.0 {
        return 0;
    }
    let used = quota_number(&quota["used"]).unwrap_or(0.0);
    (used / limit * 100.0).max(0.0).min(100.0).round() as u32
}

fn quota_number(value: &serde_json::Value) -> Option<f64> {
    // Strings are the observed shape; numbers are tolerated in case the API
    // normalizes them — a silent reshape is exactly what shape_warning can't
    // catch if the keys stay but the types change.
    value
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .or_else(|| value.as_f64())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from a live `/coding/v1/usages` response on 2026-07-29.
    /// Pinned in full — unknown keys (`user`, `totalQuota`, `authentication`,
    /// `subType`, `domain`) are ignored, and quota numbers stay strings.
    const REAL_SHAPE_BODY: &str = include_str!("../fixtures/kimi_usage_response.json");

    #[test]
    fn parses_captured_live_response() {
        let payload = classify(200, None, REAL_SHAPE_BODY);

        assert_eq!(payload.status, ProviderStatus::Ok);
        assert_eq!(payload.id, "kimi");
        assert_eq!(payload.title, "Kimi Usage");

        // limits[0].detail: "96"/"100" → 96%.
        assert_eq!(payload.session_percent, 96);
        assert_eq!(
            payload.session_resets_at.as_deref(),
            Some("2026-07-29T22:59:17.868440Z")
        );

        // usage: "19"/"100" → 19%; its resetTime is treated as weekly.
        assert_eq!(payload.weekly_percent, 19);
        assert_eq!(
            payload.weekly_resets_at.as_deref(),
            Some("2026-08-04T11:59:17.868440Z")
        );

        assert!(payload.models.is_empty());
        assert!(matches!(
            payload.extra,
            ProviderExtra::Parallel {
                used: 6,
                limit: 30
            }
        ));
        assert_eq!(payload.error_message, None);
    }

    #[test]
    fn captured_response_has_no_shape_warning() {
        let payload = classify(200, None, REAL_SHAPE_BODY);
        assert_eq!(payload.shape_warning, None);
    }

    #[test]
    fn missing_limits_array_warns() {
        let payload = parse_api_response(
            &serde_json::json!({"usage": {"limit": "100", "used": "19"}}),
        );
        assert!(payload.shape_warning.is_some());
        // Weekly still parsed — a shape change shouldn't blank the popup.
        assert_eq!(payload.weekly_percent, 19);
        assert_eq!(payload.session_percent, 0);
    }

    #[test]
    fn missing_usage_object_warns() {
        let payload = parse_api_response(
            &serde_json::json!({"limits": [{"detail": {"limit": "100", "used": "50"}}]}),
        );
        assert!(payload.shape_warning.is_some());
        assert_eq!(payload.session_percent, 50);
        assert_eq!(payload.weekly_percent, 0);
    }

    #[test]
    fn empty_limits_array_warns() {
        // Kimi's only session metric is limits[0]; empty means drift.
        let payload = parse_api_response(
            &serde_json::json!({"limits": [], "usage": {"limit": "100", "used": "19"}}),
        );
        assert!(payload.shape_warning.is_some());
    }

    #[test]
    fn percent_is_computed_and_rounded() {
        let payload = parse_api_response(&serde_json::json!({
            "limits": [{"detail": {"limit": "3", "used": "1"}}],
            "usage": {"limit": "100", "used": "19"},
        }));
        // 1/3 = 33.33… → 33.
        assert_eq!(payload.session_percent, 33);
    }

    #[test]
    fn percent_is_clamped_at_100() {
        let payload = parse_api_response(&serde_json::json!({
            "limits": [{"detail": {"limit": "100", "used": "140"}}],
            "usage": {"limit": "100", "used": "19"},
        }));
        assert_eq!(payload.session_percent, 100);
    }

    #[test]
    fn zero_limit_yields_zero_percent() {
        let payload = parse_api_response(&serde_json::json!({
            "limits": [{"detail": {"limit": "0", "used": "0"}}],
            "usage": {"limit": "100", "used": "19"},
        }));
        assert_eq!(payload.session_percent, 0);
    }

    #[test]
    fn numeric_quotas_are_tolerated() {
        let payload = parse_api_response(&serde_json::json!({
            "limits": [{"detail": {"limit": 100, "used": 40}}],
            "usage": {"limit": "100", "used": "19"},
        }));
        assert_eq!(payload.session_percent, 40);
    }

    #[test]
    fn missing_parallel_reports_zero_of_zero() {
        let payload = parse_api_response(&serde_json::json!({
            "limits": [{"detail": {"limit": "100", "used": "50"}}],
            "usage": {"limit": "100", "used": "19"},
        }));
        assert!(matches!(
            payload.extra,
            ProviderExtra::Parallel {
                used: 0,
                limit: 0
            }
        ));
    }

    #[test]
    fn classify_401_returns_auth_error_and_keeps_message_keyless() {
        let payload = classify(401, None, "");
        assert_eq!(payload.status, ProviderStatus::AuthError);
        assert_eq!(payload.id, "kimi");
        assert!(payload.error_message.unwrap().contains("API key inválida"));
    }

    #[test]
    fn classify_429_without_retry_after() {
        let payload = classify(429, None, "");
        assert_eq!(payload.status, ProviderStatus::RateLimited);
        assert!(payload.error_message.unwrap().contains("later"));
    }

    #[test]
    fn classify_429_with_retry_after() {
        let payload = classify(429, Some(42), "");
        assert!(payload.error_message.unwrap().contains("42s"));
    }

    #[test]
    fn classify_5xx_includes_status_and_body() {
        let payload = classify(503, None, "upstream down");
        assert_eq!(payload.status, ProviderStatus::Error);
        let msg = payload.error_message.unwrap();
        assert!(msg.contains("HTTP 503"));
        assert!(msg.contains("upstream down"));
    }

    #[test]
    fn classify_200_with_invalid_json_returns_error() {
        let payload = classify(200, None, "not json");
        assert_eq!(payload.status, ProviderStatus::Error);
        assert!(payload.error_message.unwrap().contains("Invalid JSON"));
    }

    /// Pins the wire format the frontend consumes for a Kimi provider.
    #[test]
    fn serialized_payload_has_expected_shape() {
        let payload = classify(200, None, REAL_SHAPE_BODY);
        let value = serde_json::to_value(&payload).unwrap();

        assert_eq!(value["id"], "kimi");
        assert_eq!(value["title"], "Kimi Usage");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["sessionPercent"], 96);
        assert_eq!(value["weeklyPercent"], 19);
        assert_eq!(
            value["extra"],
            serde_json::json!({"kind": "parallel", "used": 6, "limit": 30})
        );
        assert_eq!(value["models"], serde_json::json!([]));
        assert_eq!(value["errorMessage"], serde_json::Value::Null);
        assert_eq!(value["shapeWarning"], serde_json::Value::Null);
    }
}
