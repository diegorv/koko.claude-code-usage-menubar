# 01 — Kimi API key: keychain storage + popup settings UI

Status: done

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
- [x] Saving twice updates the key (no duplicate-item error)
- [x] Remove clears the key and the indicator flips back
- [x] The key value is never sent to the frontend after saving (only booleans)
- [x] `cargo check` and `pnpm check` pass

## Blocked by

None - can start immediately

## Out of scope

- No usage fetching yet (that's issue 03).
- Don't touch `token_cache.rs` semantics — only mirror its subprocess pattern.

## Comments

Shipped in c6d0e03, then revised by the follow-up review branch.

Deviations from the spec above, both deliberate:

- **`capabilities/default.json` was not touched.** In Tauri v2 the ACL covers
  core and plugin commands; commands registered through `generate_handler!`
  need no entry. The acceptance criterion was written from the v1 model.
- **The save no longer passes the key as `-w <key>`.** Process arguments are
  readable by any process running as the same user, and security(1) recommends
  against it, so the secret goes over stdin instead. See CLAUDE.md.

Left unchecked because they need a running app, not a test run:

- Save a key, fully restart, confirm `has_kimi_key` is true with no password
  prompt at any point. This is the criterion that actually validates the
  `/usr/bin/security` choice, and only a real restart exercises it.
