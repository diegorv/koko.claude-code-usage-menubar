use std::sync::Mutex;
use std::time::Instant;

// 24 hours — the upper bound on how long a token is reused without re-reading.
const TOKEN_CACHE_TTL_SECS: u64 = 86400;

// Every keychain read can raise a macOS authorization prompt, so never read
// more than once per this interval. Without it, a token that the API rejects
// makes each poll cycle invalidate and re-read, turning an expired token into
// one prompt every polling interval until Claude Code rotates it.
const MIN_KEYCHAIN_READ_INTERVAL_SECS: u64 = 600;

struct Entry {
    token: String,
    read_at: Instant,
    /// The API rejected this token, so it should be replaced when the backoff
    /// window allows another keychain read.
    stale: bool,
}

#[derive(Debug, PartialEq)]
enum Decision {
    UseCached,
    ReadKeychain,
}

#[derive(Default)]
pub struct TokenCache {
    cached: Mutex<Option<Entry>>,
}

impl TokenCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_or_read(&self) -> Result<String, String> {
        self.get_or_read_with(read_token_from_keychain)
    }

    /// Marks the cached token as rejected. Deliberately keeps the value: the
    /// replacement only arrives once Claude Code rotates it, and dropping it
    /// here would force a prompting keychain read on the very next cycle.
    pub fn invalidate(&self) {
        if let Ok(mut cache) = self.cached.lock() {
            if let Some(entry) = cache.as_mut() {
                entry.stale = true;
            }
        }
    }

    /// Free-standing reader injection so tests can count reads without
    /// touching the keychain.
    fn get_or_read_with<F>(&self, reader: F) -> Result<String, String>
    where
        F: FnOnce() -> Result<String, String>,
    {
        {
            let guard = self.cached.lock().map_err(|e| format!("Lock error: {}", e))?;
            if let Some(entry) = guard.as_ref() {
                let decision = decide(
                    entry.read_at.elapsed().as_secs(),
                    entry.stale,
                    TOKEN_CACHE_TTL_SECS,
                    MIN_KEYCHAIN_READ_INTERVAL_SECS,
                );
                if decision == Decision::UseCached {
                    return Ok(entry.token.clone());
                }
            }
        }

        let token = reader()?;

        {
            let mut guard = self.cached.lock().map_err(|e| format!("Lock error: {}", e))?;
            *guard = Some(Entry {
                token: token.clone(),
                read_at: Instant::now(),
                stale: false,
            });
        }

        Ok(token)
    }
}

/// Whether a cached token can be reused. A fresh token is reused until the TTL;
/// a rejected one is reused only until the backoff window opens, so a bad token
/// costs failed requests rather than a stream of keychain prompts.
fn decide(age_secs: u64, stale: bool, ttl_secs: u64, min_read_interval_secs: u64) -> Decision {
    if age_secs < min_read_interval_secs {
        return Decision::UseCached;
    }
    if stale || age_secs >= ttl_secs {
        return Decision::ReadKeychain;
    }
    Decision::UseCached
}

/// A hung `security` call would block the caller, so every invocation is
/// bounded. A targeted lookup answers in ~10ms; three seconds is pure headroom.
#[cfg(target_os = "macos")]
const SECURITY_CMD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Reads the token by shelling out to `/usr/bin/security` rather than calling
/// the Keychain API in-process.
///
/// The item's ACL grants access per requesting binary, matched against that
/// binary's designated requirement. This app is ad-hoc signed, so its
/// requirement is a hash of its own code and changes on every build — an
/// "Always Allow" grant can never stay attached to it, and macOS prompts for
/// the login password again and again. `/usr/bin/security` is Apple-signed
/// with a stable requirement, so a grant given to it holds permanently.
#[cfg(target_os = "macos")]
fn read_token_from_keychain() -> Result<String, String> {
    use std::io::Read;
    use std::process::{Command, Stdio};

    let username = std::env::var("USER").map_err(|_| "Could not get username".to_string())?;

    let mut child = Command::new("/usr/bin/security")
        .args([
            "find-generic-password",
            "-s",
            "Claude Code-credentials",
            "-a",
            &username,
            "-w",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run security: {}", e))?;

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => return Err(format!("Failed to wait for security: {}", e)),
        }
        if started.elapsed() >= SECURITY_CMD_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Timed out reading the keychain".to_string());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };

    if !status.success() {
        // Only stderr — stdout carries the secret.
        let mut stderr = String::new();
        if let Some(mut pipe) = child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr);
        }
        let detail = stderr.trim();
        return Err(if detail.is_empty() {
            "Failed to read keychain".to_string()
        } else {
            format!("Failed to read keychain: {}", detail)
        });
    }

    let mut stdout = String::new();
    child
        .stdout
        .take()
        .ok_or_else(|| "No output from security".to_string())?
        .read_to_string(&mut stdout)
        .map_err(|e| format!("Failed to read security output: {}", e))?;

    parse_keychain_json(stdout.trim())
}

#[cfg(not(target_os = "macos"))]
fn read_token_from_keychain() -> Result<String, String> {
    Err("Keychain access only available on macOS".to_string())
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

    const TTL: u64 = 86400;
    const MIN: u64 = 600;

    #[test]
    fn decide_reuses_a_fresh_token() {
        assert_eq!(decide(0, false, TTL, MIN), Decision::UseCached);
        assert_eq!(decide(TTL - 1, false, TTL, MIN), Decision::UseCached);
    }

    #[test]
    fn decide_reads_again_once_the_ttl_expires() {
        assert_eq!(decide(TTL, false, TTL, MIN), Decision::ReadKeychain);
    }

    #[test]
    fn decide_holds_a_rejected_token_through_the_backoff_window() {
        // The prompt storm: without this, every poll after a 401 re-read the
        // keychain. Inside the window the stale token is reused instead.
        assert_eq!(decide(0, true, TTL, MIN), Decision::UseCached);
        assert_eq!(decide(MIN - 1, true, TTL, MIN), Decision::UseCached);
    }

    #[test]
    fn decide_replaces_a_rejected_token_once_the_window_opens() {
        assert_eq!(decide(MIN, true, TTL, MIN), Decision::ReadKeychain);
    }

    fn counting_reader(calls: &AtomicUsize) -> Result<String, String> {
        let n = calls.fetch_add(1, Ordering::SeqCst);
        Ok(format!("tok-{}", n))
    }

    #[test]
    fn reads_the_keychain_when_empty() {
        let cache = TokenCache::new();
        let calls = AtomicUsize::new(0);
        let token = cache.get_or_read_with(|| counting_reader(&calls)).unwrap();
        assert_eq!(token, "tok-0");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn reuses_the_cached_token_on_the_next_call() {
        let cache = TokenCache::new();
        let calls = AtomicUsize::new(0);
        cache.get_or_read_with(|| counting_reader(&calls)).unwrap();
        let second = cache.get_or_read_with(|| counting_reader(&calls)).unwrap();
        assert_eq!(second, "tok-0");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn invalidate_does_not_trigger_an_immediate_reread() {
        // Regression guard for the prompt storm: a 401 must not cost a
        // keychain read on the following poll.
        let cache = TokenCache::new();
        let calls = AtomicUsize::new(0);
        cache.get_or_read_with(|| counting_reader(&calls)).unwrap();

        cache.invalidate();
        let after = cache.get_or_read_with(|| counting_reader(&calls)).unwrap();

        assert_eq!(after, "tok-0", "stale token should be reused inside the backoff window");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "no second keychain read");
    }

    #[test]
    fn invalidate_marks_the_entry_stale_without_dropping_it() {
        let cache = TokenCache::new();
        let calls = AtomicUsize::new(0);
        cache.get_or_read_with(|| counting_reader(&calls)).unwrap();
        cache.invalidate();

        let guard = cache.cached.lock().unwrap();
        let entry = guard.as_ref().expect("entry kept");
        assert!(entry.stale);
        assert_eq!(entry.token, "tok-0");
    }

    #[test]
    fn propagates_reader_error() {
        let cache = TokenCache::new();
        let result = cache.get_or_read_with(|| Err("boom".to_string()));
        assert_eq!(result.unwrap_err(), "boom");
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

