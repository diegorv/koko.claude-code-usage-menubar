#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsagePayload {
    pub status: String,
    pub session_percent: u32,
    pub session_resets_at: Option<String>,
    pub weekly_percent: u32,
    pub weekly_resets_at: Option<String>,
    pub models: Vec<ModelPayload>,
    pub extra_usage_enabled: bool,
    pub extra_usage_percent: u32,
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
    pub(crate) fn error(status: &str, message: &str) -> Self {
        Self {
            status: status.to_string(),
            session_percent: 0,
            session_resets_at: None,
            weekly_percent: 0,
            weekly_resets_at: None,
            models: vec![],
            extra_usage_enabled: false,
            extra_usage_percent: 0,
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

/// Pure translation from an HTTP response to a `UsagePayload`. Has no side
/// effects — callers handle cache invalidation / persistence based on the
/// returned `status`.
pub fn classify(status: u16, retry_after: Option<u64>, body: &str) -> UsagePayload {
    match status {
        200..=299 => parse_success_body(body),
        401 | 403 => UsagePayload::error(
            "unauthorized",
            "Token expired. Run \"claude login\" to re-authenticate.",
        ),
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

fn parse_success_body(body: &str) -> UsagePayload {
    match serde_json::from_str::<serde_json::Value>(body) {
        Ok(json) => parse_api_response(&json),
        Err(e) => UsagePayload::error("error", &format!("Invalid JSON: {}", e)),
    }
}

pub(crate) fn parse_api_response(json: &serde_json::Value) -> UsagePayload {
    let clamp = |v: f64| v.max(0.0).min(100.0).round() as u32;

    // Per-model limits live in `limits[]` as `weekly_scoped` entries carrying
    // `scope.model.display_name`. The old `seven_day_sonnet` / `seven_day_opus`
    // buckets are deprecated and always null. Don't filter on `is_active` —
    // only the session limit is ever active, so it would hide every model.
    let models = json["limits"]
        .as_array()
        .map(|limits| {
            limits
                .iter()
                .filter(|limit| limit["kind"] == "weekly_scoped")
                .filter_map(|limit| {
                    let name = limit["scope"]["model"]["display_name"].as_str()?;
                    Some(ModelPayload {
                        name: name.to_string(),
                        percent: clamp(limit["percent"].as_f64().unwrap_or(0.0)),
                        resets_at: limit["resets_at"].as_str().map(String::from),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

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
        extra_usage_enabled: json["extra_usage"]["is_enabled"]
            .as_bool()
            .unwrap_or(false),
        extra_usage_percent: clamp(json["extra_usage"]["utilization"].as_f64().unwrap_or(0.0)),
        last_updated_at: now_millis(),
        error_message: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK_BODY: &str = r#"{
        "five_hour": {"utilization": 45.0, "resets_at": "2024-01-01T00:00:00Z"},
        "seven_day": {"utilization": 67.0, "resets_at": "2024-01-07T00:00:00Z"},
        "seven_day_sonnet": null,
        "seven_day_opus": null,
        "limits": [
            {"kind": "session", "percent": 45, "scope": null},
            {"kind": "weekly_all", "percent": 67, "scope": null},
            {"kind": "weekly_scoped", "percent": 30,
             "scope": {"model": {"id": null, "display_name": "Sonnet"}, "surface": null}},
            {"kind": "weekly_scoped", "percent": 80, "resets_at": "2024-01-07T00:00:00Z",
             "scope": {"model": {"id": null, "display_name": "Opus"}, "surface": null}}
        ]
    }"#;

    #[test]
    fn parse_api_response_ok() {
        let json: serde_json::Value = serde_json::from_str(OK_BODY).unwrap();
        let payload = parse_api_response(&json);
        assert_eq!(payload.status, "ok");
        assert_eq!(payload.session_percent, 45);
        assert_eq!(payload.weekly_percent, 67);
        assert_eq!(payload.models.len(), 2);
        assert_eq!(payload.models[0].name, "Sonnet");
        assert_eq!(payload.models[0].percent, 30);
        assert_eq!(payload.models[1].name, "Opus");
        assert_eq!(payload.models[1].percent, 80);
        assert_eq!(
            payload.models[1].resets_at.as_deref(),
            Some("2024-01-07T00:00:00Z")
        );
    }

    #[test]
    fn parse_api_response_skips_scoped_without_model_name() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{
                "limits": [
                    {"kind": "weekly_scoped", "percent": 10, "scope": {"surface": "code"}},
                    {"kind": "weekly_scoped", "percent": 20,
                     "scope": {"model": {"display_name": "Fable"}}}
                ]
            }"#,
        )
        .unwrap();
        let payload = parse_api_response(&json);
        assert_eq!(payload.models.len(), 1);
        assert_eq!(payload.models[0].name, "Fable");
        assert_eq!(payload.models[0].percent, 20);
    }

    #[test]
    fn parse_api_response_without_limits_has_no_models() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{"five_hour": {"utilization": 1.0}, "seven_day": {"utilization": 2.0}}"#,
        )
        .unwrap();
        let payload = parse_api_response(&json);
        assert!(payload.models.is_empty());
    }

    #[test]
    fn parse_api_response_ignores_deprecated_per_model_buckets() {
        // Old shape: data lived in seven_day_sonnet/seven_day_opus. Now deprecated
        // (always null in the live API); only limits[] weekly_scoped counts.
        let json: serde_json::Value = serde_json::from_str(
            r#"{
                "seven_day_sonnet": {"utilization": 30.0},
                "seven_day_opus": {"utilization": 80.0},
                "limits": []
            }"#,
        )
        .unwrap();
        let payload = parse_api_response(&json);
        assert!(payload.models.is_empty());
    }

    #[test]
    fn parse_api_response_clamps() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{"five_hour": {"utilization": 150.0}, "seven_day": {"utilization": -10.0}}"#,
        )
        .unwrap();
        let payload = parse_api_response(&json);
        assert_eq!(payload.session_percent, 100);
        assert_eq!(payload.weekly_percent, 0);
    }

    #[test]
    fn classify_200_returns_ok() {
        let payload = classify(200, None, OK_BODY);
        assert_eq!(payload.status, "ok");
        assert_eq!(payload.session_percent, 45);
    }

    #[test]
    fn classify_401_returns_unauthorized() {
        let payload = classify(401, None, "");
        assert_eq!(payload.status, "unauthorized");
        assert!(payload.error_message.unwrap().contains("Token expired"));
    }

    #[test]
    fn classify_403_returns_unauthorized() {
        let payload = classify(403, None, "");
        assert_eq!(payload.status, "unauthorized");
    }

    #[test]
    fn classify_429_without_retry_after() {
        let payload = classify(429, None, "");
        assert_eq!(payload.status, "error");
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
        let msg = payload.error_message.unwrap();
        assert!(msg.contains("HTTP 503"));
        assert!(msg.contains("upstream down"));
    }

    #[test]
    fn classify_500_includes_body() {
        let payload = classify(500, None, "boom");
        let msg = payload.error_message.unwrap();
        assert!(msg.contains("HTTP 500"));
        assert!(msg.contains("boom"));
    }

    #[test]
    fn classify_200_with_invalid_json_returns_error() {
        let payload = classify(200, None, "not json");
        assert_eq!(payload.status, "error");
        assert!(payload.error_message.unwrap().contains("Invalid JSON"));
    }

    #[test]
    fn classify_200_with_empty_body_returns_error() {
        let payload = classify(200, None, "");
        assert_eq!(payload.status, "error");
        assert!(payload.error_message.unwrap().contains("Invalid JSON"));
    }

    #[test]
    fn parse_api_response_extra_usage_enabled() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{
                "five_hour": {"utilization": 10.0},
                "seven_day": {"utilization": 20.0},
                "extra_usage": {"is_enabled": true, "utilization": 42.0}
            }"#,
        )
        .unwrap();
        let payload = parse_api_response(&json);
        assert!(payload.extra_usage_enabled);
        assert_eq!(payload.extra_usage_percent, 42);
    }

    #[test]
    fn parse_api_response_extra_usage_disabled() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{
                "five_hour": {"utilization": 10.0},
                "seven_day": {"utilization": 20.0},
                "extra_usage": {"is_enabled": false, "utilization": 0.0}
            }"#,
        )
        .unwrap();
        let payload = parse_api_response(&json);
        assert!(!payload.extra_usage_enabled);
        assert_eq!(payload.extra_usage_percent, 0);
    }

    #[test]
    fn parse_api_response_extra_usage_missing() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{"five_hour": {"utilization": 10.0}, "seven_day": {"utilization": 20.0}}"#,
        )
        .unwrap();
        let payload = parse_api_response(&json);
        assert!(!payload.extra_usage_enabled);
        assert_eq!(payload.extra_usage_percent, 0);
    }
}
