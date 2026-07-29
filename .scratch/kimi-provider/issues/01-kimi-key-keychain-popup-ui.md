# 01 — Kimi API key: keychain storage + popup settings UI

Status: ready-for-agent

## What to build

End-to-end key management so the user can paste their Kimi API key in the popup and have it persisted in the macOS Keychain:

- New Rust module (sibling of `token_cache.rs`) that shells out to `/usr/bin/security`:
  - save: `add-generic-password -s "koko-kimi-api-key" -a kimi -w <key> -U`
  - delete: `delete-generic-password -s "koko-kimi-api-key"`
  - exists check: `find-generic-password -s "koko-kimi-api-key" -w` (exit status only)
- Follow the same hard rules as `token_cache.rs`: subprocess with timeout + kill fallback, errors report stderr only (stdout carries the secret). Never return the key to the frontend.
- Tauri commands: `save_kimi_key`, `delete_kimi_key`, `has_kimi_key` (returns bool). Register them in `capabilities/default.json`.
- Popup: expandable settings section in the footer with a password input, Save/Remove buttons, and a saved/not-saved indicator.

## Acceptance criteria

- [ ] Save a key, fully restart the app → `has_kimi_key` returns true, no password prompt at any point
- [ ] Saving twice updates the key (no duplicate-item error)
- [ ] Remove clears the key and the indicator flips back
- [ ] The key value is never sent to the frontend after saving (only booleans)
- [ ] `cargo check` and `pnpm check` pass

## Blocked by

None - can start immediately

## Out of scope

- No usage fetching yet (that's issue 03).
- Don't touch `token_cache.rs` semantics — only mirror its subprocess pattern.
