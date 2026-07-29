// Storage for the Kimi API key. Mirrors the token_cache.rs pattern: all
// keychain access goes through `/usr/bin/security` (Apple-signed, stable
// designated requirement, so a grant holds across rebuilds) — never the
// in-process Keychain API, never plaintext files.
//
// The key itself never leaves this module: commands expose only booleans,
// subprocess stdout is discarded unread (for `find -w` it carries the
// secret), and error strings are built from stderr only.

const SERVICE_NAME: &str = "koko-kimi-api-key";
const ACCOUNT_NAME: &str = "kimi";

/// A hung `security` call would block the caller, so every invocation is
/// bounded — same rationale as token_cache.rs.
#[cfg(target_os = "macos")]
const SECURITY_CMD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

fn save_args(key: &str) -> Vec<String> {
    // -U updates in place, so saving twice rotates the key instead of
    // failing with a duplicate-item error.
    vec![
        "add-generic-password".to_string(),
        "-s".to_string(),
        SERVICE_NAME.to_string(),
        "-a".to_string(),
        ACCOUNT_NAME.to_string(),
        "-w".to_string(),
        key.to_string(),
        "-U".to_string(),
    ]
}

fn delete_args() -> Vec<String> {
    vec![
        "delete-generic-password".to_string(),
        "-s".to_string(),
        SERVICE_NAME.to_string(),
    ]
}

fn exists_args() -> Vec<String> {
    vec![
        "find-generic-password".to_string(),
        "-s".to_string(),
        SERVICE_NAME.to_string(),
        "-w".to_string(),
    ]
}

/// stderr-only error strings: stdout of a keychain command can carry the
/// secret, so it must never reach an error message.
fn describe_failure(action: &str, stderr: &str) -> String {
    let detail = stderr.trim();
    if detail.is_empty() {
        format!("Failed to {}", action)
    } else {
        format!("Failed to {}: {}", action, detail)
    }
}

pub fn save(key: &str) -> Result<(), String> {
    // Trimmed: keys are pasted, and paste often carries a trailing newline.
    let key = key.trim();
    if key.is_empty() {
        return Err("Kimi API key is empty".to_string());
    }
    let outcome = run_security(&save_args(key))?;
    if outcome.success {
        Ok(())
    } else {
        Err(describe_failure("save the Kimi API key", &outcome.stderr))
    }
}

pub fn remove() -> Result<(), String> {
    let outcome = run_security(&delete_args())?;
    if outcome.success {
        Ok(())
    } else {
        Err(describe_failure("remove the Kimi API key", &outcome.stderr))
    }
}

/// Whether a key is stored. Exit status only — the item's data is never read.
pub fn exists() -> Result<bool, String> {
    let outcome = run_security(&exists_args())?;
    Ok(outcome.success)
}

struct SecurityOutcome {
    success: bool,
    stderr: String,
}

#[cfg(target_os = "macos")]
fn run_security(args: &[String]) -> Result<SecurityOutcome, String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::Instant;

    let mut child = Command::new("/usr/bin/security")
        .args(args)
        // stdout is nulled, never piped: for `find -w` it carries the secret.
        .stdout(Stdio::null())
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
            return Err("Timed out accessing the keychain".to_string());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };

    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }

    Ok(SecurityOutcome {
        success: status.success(),
        stderr,
    })
}

#[cfg(not(target_os = "macos"))]
fn run_security(_args: &[String]) -> Result<SecurityOutcome, String> {
    Err("Keychain access only available on macOS".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_args_update_the_item_in_place() {
        assert_eq!(
            save_args("sk-kimi-test"),
            vec![
                "add-generic-password",
                "-s",
                "koko-kimi-api-key",
                "-a",
                "kimi",
                "-w",
                "sk-kimi-test",
                "-U",
            ]
        );
    }

    #[test]
    fn delete_args_target_only_the_service() {
        assert_eq!(
            delete_args(),
            vec!["delete-generic-password", "-s", "koko-kimi-api-key"]
        );
    }

    #[test]
    fn exists_args_read_only_the_service() {
        assert_eq!(
            exists_args(),
            vec!["find-generic-password", "-s", "koko-kimi-api-key", "-w"]
        );
    }

    #[test]
    fn failure_message_carries_stderr_detail() {
        assert_eq!(
            describe_failure("save the Kimi API key", "  boom\n"),
            "Failed to save the Kimi API key: boom"
        );
    }

    #[test]
    fn failure_message_without_stderr_stays_generic() {
        assert_eq!(
            describe_failure("remove the Kimi API key", ""),
            "Failed to remove the Kimi API key"
        );
    }

    #[test]
    fn failure_message_never_contains_the_key() {
        // Error strings are built from stderr alone, so a key passed to the
        // subprocess (argv/stdout) can never leak into a message.
        let msg = describe_failure("save the Kimi API key", "write failed");
        assert!(!msg.contains("sk-kimi-test"));
    }

    #[test]
    fn save_rejects_a_blank_key_before_touching_the_keychain() {
        let err = save("  \n ").unwrap_err();
        assert!(err.contains("empty"), "unexpected error: {}", err);
    }
}
