// Storage for the Kimi API key. Mirrors the token_cache.rs pattern: all
// keychain access goes through `/usr/bin/security` (Apple-signed, stable
// designated requirement, so a grant holds across rebuilds) — never the
// in-process Keychain API, never plaintext files.
//
// The boundary is IPC, not this module: `read` returns the key into Rust
// memory for the fetch layer, but the key must never cross to the frontend —
// commands expose only booleans. Subprocess stdout carries the secret on
// `find -w`, so it is piped only by `read` and nulled on every other
// invocation, and error strings are built from stderr only.
//
// The key also never appears in the subprocess's argv. Process arguments are
// readable by any process running as the same user (`ps -ww`), and security(1)
// says so itself: "-w password  Specify password to be added. Put at end of
// command to be prompted (recommended)". So `save` passes a bare `-w` and
// writes the secret to the child's stdin instead.

const SERVICE_NAME: &str = "koko-kimi-api-key";
const ACCOUNT_NAME: &str = "kimi";

/// A hung `security` call would block the caller, so every invocation is
/// bounded — same rationale as token_cache.rs.
#[cfg(target_os = "macos")]
const SECURITY_CMD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// No key here — a bare trailing `-w` makes `security` read the secret from
/// stdin instead of argv. `-U` updates in place, so saving twice rotates the
/// key instead of failing with a duplicate-item error.
fn save_args() -> Vec<String> {
    vec![
        "add-generic-password".to_string(),
        "-s".to_string(),
        SERVICE_NAME.to_string(),
        "-a".to_string(),
        ACCOUNT_NAME.to_string(),
        "-U".to_string(),
        "-w".to_string(),
    ]
}

/// `security` prompts for the secret twice (value, then confirmation) and
/// reads both from stdin when it has no terminal. A single line makes the two
/// reads disagree, and the command then stores an *empty* password while still
/// exiting 0 — so the key goes in twice.
fn save_stdin(key: &str) -> String {
    format!("{}\n{}\n", key, key)
}

/// Those two prompts are written to stderr, which is where error detail comes
/// from. They are noise, not failure, so they are dropped before an error
/// string is built. Worst case if Apple ever localizes them: a noisier message.
fn strip_password_prompts(stderr: &str) -> String {
    stderr
        .replace("password data for new item: ", "")
        .replace("retype password for new item: ", "")
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
    let outcome = run_security_with_stdin(&save_args(), &save_stdin(key))?;
    if !outcome.success {
        return Err(describe_failure(
            "save the Kimi API key",
            &strip_password_prompts(&outcome.stderr),
        ));
    }
    // Exit 0 does not mean the right value landed: if `security` ever reads
    // the secret from somewhere other than our stdin it stores an empty
    // password and still succeeds. Read it back rather than report a save
    // that silently stored nothing.
    match read() {
        Ok(Some(stored)) if stored == key => Ok(()),
        Ok(_) => Err("Saved the Kimi API key but read it back empty".to_string()),
        Err(e) => Err(e),
    }
}

pub fn remove() -> Result<(), String> {
    let outcome = run_security(&delete_args())?;
    remove_result(&outcome)
}

/// Deleting an absent key is success — the desired end state already holds.
/// `security` reports errSecItemNotFound with the SecKeychainSearchCopyNext
/// symbol on stderr; surfacing it as an error would only punish a retry.
/// Match the symbol, not the English sentence — the sentence is localized.
fn remove_result(outcome: &SecurityOutcome) -> Result<(), String> {
    if outcome.success || outcome.stderr.contains("SecKeychainSearchCopyNext") {
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

/// Reads the stored key into process memory. Unlike `exists`, stdout — the
/// secret — is piped instead of nulled. The value is for the Rust fetch layer
/// only: it must never cross IPC, and error strings stay stderr-only.
///
/// `Ok(None)` covers both "no item stored" (errSecItemNotFound) and a blank
/// value; infra failures (spawn, timeout) surface as `Err` so the caller can
/// decide — the fetch layer treats them as "no usable key" and omits the
/// provider rather than erroring every cycle.
///
/// Called once per poll cycle with no caching floor (unlike token_cache's
/// 10-minute minimum): this item is created by `/usr/bin/security` itself, so
/// its ACL trusts that binary and reads never prompt. The floor exists for
/// the Claude item, whose ACL belongs to another app's binary.
pub fn read() -> Result<Option<String>, String> {
    let outcome = run_security_capturing_stdout(&exists_args())?;
    if !outcome.success {
        return Ok(None);
    }
    let key = outcome.stdout.unwrap_or_default().trim().to_string();
    if key.is_empty() {
        return Ok(None);
    }
    Ok(Some(key))
}

struct SecurityOutcome {
    success: bool,
    stderr: String,
    /// Present only when the run captured stdout — see `read`.
    stdout: Option<String>,
}

#[cfg(target_os = "macos")]
fn run_security(args: &[String]) -> Result<SecurityOutcome, String> {
    run_security_impl(args, false, None)
}

/// Same as `run_security`, but stdout is piped and returned. Only `read`
/// uses this: for every other invocation stdout can carry the secret, so it
/// stays nulled.
#[cfg(target_os = "macos")]
fn run_security_capturing_stdout(args: &[String]) -> Result<SecurityOutcome, String> {
    run_security_impl(args, true, None)
}

/// Same as `run_security`, but `stdin_payload` is written to the child. Only
/// `save` uses this — it is how the secret stays out of argv.
#[cfg(target_os = "macos")]
fn run_security_with_stdin(
    args: &[String],
    stdin_payload: &str,
) -> Result<SecurityOutcome, String> {
    run_security_impl(args, false, Some(stdin_payload))
}

#[cfg(target_os = "macos")]
fn run_security_impl(
    args: &[String],
    capture_stdout: bool,
    stdin_payload: Option<&str>,
) -> Result<SecurityOutcome, String> {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    use std::time::Instant;

    let stdout_cfg = if capture_stdout {
        Stdio::piped()
    } else {
        // stdout is nulled, never piped: for `find -w` it carries the secret.
        Stdio::null()
    };

    let stdin_cfg = if stdin_payload.is_some() {
        Stdio::piped()
    } else {
        // Nothing to send, and an inherited stdin would let `security` block
        // on a prompt we never intended to answer.
        Stdio::null()
    };

    let mut child = Command::new("/usr/bin/security")
        .args(args)
        .stdin(stdin_cfg)
        .stdout(stdout_cfg)
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run security: {}", e))?;

    if let Some(payload) = stdin_payload {
        // Dropped right after writing so `security` sees EOF and stops
        // waiting for more input. The payload is far below the pipe buffer,
        // so this write cannot block.
        if let Some(mut pipe) = child.stdin.take() {
            let _ = pipe.write_all(payload.as_bytes());
        }
    }

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

    let stdout = if capture_stdout {
        let mut buf = String::new();
        if let Some(mut pipe) = child.stdout.take() {
            let _ = pipe.read_to_string(&mut buf);
        }
        Some(buf)
    } else {
        None
    };

    Ok(SecurityOutcome {
        success: status.success(),
        stderr,
        stdout,
    })
}

#[cfg(not(target_os = "macos"))]
fn run_security(_args: &[String]) -> Result<SecurityOutcome, String> {
    Err("Keychain access only available on macOS".to_string())
}

#[cfg(not(target_os = "macos"))]
fn run_security_capturing_stdout(_args: &[String]) -> Result<SecurityOutcome, String> {
    Err("Keychain access only available on macOS".to_string())
}

#[cfg(not(target_os = "macos"))]
fn run_security_with_stdin(_args: &[String], _stdin_payload: &str) -> Result<SecurityOutcome, String> {
    Err("Keychain access only available on macOS".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_args_update_the_item_in_place_without_the_key() {
        // `-w` is bare and last: the secret goes over stdin, never argv, which
        // any process running as this user could read with `ps -ww`.
        let args = save_args();
        assert_eq!(
            args,
            vec![
                "add-generic-password",
                "-s",
                "koko-kimi-api-key",
                "-a",
                "kimi",
                "-U",
                "-w",
            ]
        );
        assert_eq!(args.last().unwrap(), "-w");
    }

    #[test]
    fn save_stdin_repeats_the_key_for_the_confirmation_prompt() {
        // One line would make the value and confirmation reads disagree, and
        // `security` then stores an empty password while still exiting 0.
        assert_eq!(save_stdin("sk-kimi-test"), "sk-kimi-test\nsk-kimi-test\n");
    }

    #[test]
    fn password_prompts_are_stripped_from_error_detail() {
        let noisy = "password data for new item: retype password for new item: security: boom";
        assert_eq!(strip_password_prompts(noisy), "security: boom");
    }

    #[test]
    fn save_failure_message_carries_no_prompt_noise() {
        let stderr = "password data for new item: retype password for new item: ";
        assert_eq!(
            describe_failure("save the Kimi API key", &strip_password_prompts(stderr)),
            "Failed to save the Kimi API key"
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
    fn remove_treats_a_missing_item_as_success() {
        let not_found = SecurityOutcome {
            success: false,
            stderr: "security: SecKeychainSearchCopyNext: The specified item could not be found in the keychain.".to_string(),
            stdout: None,
        };
        assert!(remove_result(&not_found).is_ok());
    }

    #[test]
    fn remove_treats_a_missing_item_as_success_in_any_locale() {
        // Same errSecItemNotFound, localized sentence — the symbol is stable.
        let not_found = SecurityOutcome {
            success: false,
            stderr: "security: SecKeychainSearchCopyNext: O item especificado não foi encontrado.".to_string(),
            stdout: None,
        };
        assert!(remove_result(&not_found).is_ok());
    }

    #[test]
    fn remove_surfaces_real_failures() {
        let denied = SecurityOutcome {
            success: false,
            stderr: "authorization denied".to_string(),
            stdout: None,
        };
        let err = remove_result(&denied).unwrap_err();
        assert!(err.contains("authorization denied"));
        assert!(err.contains("remove the Kimi API key"));
    }

    #[test]
    fn save_rejects_a_blank_key_before_touching_the_keychain() {
        let err = save("  \n ").unwrap_err();
        assert!(err.contains("empty"), "unexpected error: {}", err);
    }
}
