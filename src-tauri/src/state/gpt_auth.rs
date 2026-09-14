//! Reads the ChatGPT OAuth token Codex CLI keeps in `$CODEX_HOME/auth.json`
//! (default `~/.codex/auth.json`).
//!
//! Read-only on purpose. Codex CLI owns this file and refreshes the token in
//! it; refreshing or writing it back from here would race that and could log
//! the user out of Codex. Reading it on every poll is what picks up a token
//! Codex has just rotated — there is no cache to go stale.

use std::path::PathBuf;

pub struct GptCredentials {
    pub access_token: String,
    pub account_id: Option<String>,
}

fn auth_path() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CODEX_HOME") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir).join("auth.json"));
        }
    }
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".codex").join("auth.json"))
}

/// `Ok(None)` when there is no auth file at all — Codex was never logged in
/// on this machine, and the provider is omitted rather than shown as an error.
/// `Err` when the file exists but holds no usable ChatGPT login.
pub fn read() -> Result<Option<GptCredentials>, String> {
    let Some(path) = auth_path() else {
        return Ok(None);
    };
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Failed to read the Codex auth file: {}", e)),
    };
    parse(&contents).map(Some)
}

/// Error strings are fixed text: the file holds secrets, so nothing from its
/// contents — not even a JSON parser's diagnostic — is forwarded.
pub(crate) fn parse(contents: &str) -> Result<GptCredentials, String> {
    let json: serde_json::Value = serde_json::from_str(contents)
        .map_err(|_| "The Codex auth file is not valid JSON.".to_string())?;
    // An API-key login (`OPENAI_API_KEY` with no `tokens`) has no plan limits
    // to report, so it counts as "not logged in with ChatGPT".
    let access_token = json["tokens"]["access_token"]
        .as_str()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            "No ChatGPT login found. Run \"codex login\" with your ChatGPT account.".to_string()
        })?;
    Ok(GptCredentials {
        access_token: access_token.to_string(),
        account_id: json["tokens"]["account_id"].as_str().map(String::from),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_token_and_account_id() {
        let creds = parse(
            r#"{"OPENAI_API_KEY": null,
                "tokens": {"id_token": "i", "access_token": "a", "refresh_token": "r", "account_id": "acc"},
                "last_refresh": "2026-09-01T00:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(creds.access_token, "a");
        assert_eq!(creds.account_id.as_deref(), Some("acc"));
    }

    #[test]
    fn account_id_is_optional() {
        let creds = parse(r#"{"tokens": {"access_token": "a"}}"#).unwrap();
        assert_eq!(creds.account_id, None);
    }

    #[test]
    fn api_key_login_is_not_a_chatgpt_login() {
        let err = parse(r#"{"OPENAI_API_KEY": "sk-secret", "tokens": null}"#)
            .err()
            .unwrap();
        assert!(err.contains("codex login"));
        assert!(!err.contains("sk-secret"));
    }

    #[test]
    fn empty_token_is_rejected() {
        assert!(parse(r#"{"tokens": {"access_token": ""}}"#).is_err());
    }

    #[test]
    fn invalid_json_error_does_not_echo_the_file() {
        let err = parse(r#"{"tokens": {"access_token": "secret-token""#).err().unwrap();
        assert!(!err.contains("secret-token"));
    }
}
