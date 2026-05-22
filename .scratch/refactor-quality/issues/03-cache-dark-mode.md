# 03 — Cache dark-mode detection per redraw cycle

Status: ready-for-agent
Phase: A

## Problem

`tray_icon::menubar_is_dark()` shells out to `defaults read -g AppleInterfaceStyle` every icon redraw. Called twice per `generate_icon` call (once for each `draw_line`) — wait, no, it's called once in `generate_icon`. But every 2 minutes (polling) + every popup click.

Fork+exec for a value that changes only when user toggles system appearance.

## Change

Two options:

**Option A (minimum):** memoize once-per-process with `LazyLock<bool>`. Won't react to appearance changes until app restart. Acceptable: appearance toggles are rare.

**Option B (better):** memoize with TTL (~30s). Re-detects on next icon render after the TTL.

Recommended: **Option B**. Same code volume.

```rust
static DARK_MODE_CACHE: LazyLock<Mutex<Option<(bool, Instant)>>> =
    LazyLock::new(|| Mutex::new(None));
const DARK_MODE_TTL_SECS: u64 = 30;

fn menubar_is_dark_cached() -> bool { ... }
```

Replace `menubar_is_dark()` call in `generate_icon` with `menubar_is_dark_cached()`.

## Verify

- `cargo check`
- `pnpm tauri dev` → toggle macOS appearance → wait ≤30s → next refresh shows correct text color.

## Out of scope

- Don't switch to a notification-based approach (NSDistributedNotificationCenter etc.) — overkill.
