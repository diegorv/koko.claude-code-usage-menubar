use std::sync::Mutex;
use std::time::Instant;

// 24 hours — only re-reads keychain on auth errors
const TOKEN_CACHE_TTL_SECS: u64 = 86400;

#[derive(Default)]
pub struct TokenCache {
    cached: Mutex<Option<(String, Instant)>>,
}

impl TokenCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_or_read(&self) -> Result<String, String> {
        {
            let cache = self.cached.lock().map_err(|e| format!("Lock error: {}", e))?;
            if let Some((ref token, ref cached_at)) = *cache {
                if cached_at.elapsed().as_secs() < TOKEN_CACHE_TTL_SECS {
                    return Ok(token.clone());
                }
            }
        }

        let token = read_token_from_keychain()?;

        {
            let mut cache = self.cached.lock().map_err(|e| format!("Lock error: {}", e))?;
            *cache = Some((token.clone(), Instant::now()));
        }

        Ok(token)
    }

    pub fn invalidate(&self) {
        if let Ok(mut cache) = self.cached.lock() {
            *cache = None;
        }
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

/// Extracts access token from keychain JSON
pub(crate) fn parse_keychain_json(json_str: &str) -> Result<String, String> {
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
}
