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
        cached_or_fetch(&self.cached, TOKEN_CACHE_TTL_SECS, read_token_from_keychain)
    }

    pub fn invalidate(&self) {
        if let Ok(mut cache) = self.cached.lock() {
            *cache = None;
        }
    }
}

/// Returns the cached token if it's younger than `ttl_secs`. Otherwise calls
/// `reader`, stores the result, and returns it. Free-standing so tests can
/// inject a counting reader without touching the keychain.
fn cached_or_fetch<F>(
    cache: &Mutex<Option<(String, Instant)>>,
    ttl_secs: u64,
    reader: F,
) -> Result<String, String>
where
    F: FnOnce() -> Result<String, String>,
{
    {
        let guard = cache.lock().map_err(|e| format!("Lock error: {}", e))?;
        if let Some((ref token, ref cached_at)) = *guard {
            if cached_at.elapsed().as_secs() < ttl_secs {
                return Ok(token.clone());
            }
        }
    }

    let token = reader()?;

    {
        let mut guard = cache.lock().map_err(|e| format!("Lock error: {}", e))?;
        *guard = Some((token.clone(), Instant::now()));
    }

    Ok(token)
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn cached_or_fetch_calls_reader_when_empty() {
        let cache = Mutex::new(None);
        let calls = AtomicUsize::new(0);
        let result = cached_or_fetch(&cache, 60, || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok("tok-a".to_string())
        });
        assert_eq!(result.unwrap(), "tok-a");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    fn counting_reader(calls: &AtomicUsize) -> Result<String, String> {
        let n = calls.fetch_add(1, Ordering::SeqCst);
        Ok(format!("tok-{}", n))
    }

    #[test]
    fn cached_or_fetch_skips_reader_within_ttl() {
        let cache = Mutex::new(None);
        let calls = AtomicUsize::new(0);

        let first = cached_or_fetch(&cache, 60, || counting_reader(&calls)).unwrap();
        let second = cached_or_fetch(&cache, 60, || counting_reader(&calls)).unwrap();

        assert_eq!(first, "tok-0");
        assert_eq!(second, "tok-0"); // same value: cache hit
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cached_or_fetch_refetches_after_ttl_expires() {
        let cache = Mutex::new(None);
        let calls = AtomicUsize::new(0);

        cached_or_fetch(&cache, 0, || counting_reader(&calls)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let second = cached_or_fetch(&cache, 0, || counting_reader(&calls)).unwrap();

        assert_eq!(second, "tok-1");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn cached_or_fetch_propagates_reader_error() {
        let cache = Mutex::new(None);
        let result = cached_or_fetch(&cache, 60, || Err("boom".to_string()));
        assert_eq!(result.unwrap_err(), "boom");
    }

    #[test]
    fn invalidate_forces_next_call_to_refetch() {
        let token_cache = TokenCache::new();
        let calls = AtomicUsize::new(0);

        // Seed the cache via the free fn (bypasses keychain access).
        cached_or_fetch(&token_cache.cached, 60, || counting_reader(&calls)).unwrap();
        token_cache.invalidate();
        let after = cached_or_fetch(&token_cache.cached, 60, || counting_reader(&calls)).unwrap();

        assert_eq!(after, "tok-1");
    }

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
