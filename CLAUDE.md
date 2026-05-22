# Claude Code Usage Menubar

Tauri v2 (Rust) + SvelteKit macOS menubar app that shows Claude usage percentages in the tray icon and opens a popup on click.

## Architecture notes

- **Native polling lives in Rust** ([commands.rs](src-tauri/src/commands.rs)) — not in the WebView — so it runs even when the popup is hidden. The frontend only displays data; it doesn't own the refresh loop.
- **Two data paths into the popup**: (1) `invoke('trigger_refresh')` on mount, and (2) `listen('usage_updated')` for push updates from Rust-side polling. Both must work for the popup to show data immediately on first open.
- **`trigger_refresh` has a 30s throttle** with `LAST_FETCH` + `LAST_PAYLOAD` caches. When throttled, it returns the cached payload instead of refetching. The frontend mirrors this with a 30s cooldown on the Refresh button (bouncing dots animation while disabled).
- **Tray icon is generated in Rust** ([tray_icon.rs](src-tauri/src/tray_icon.rs)) as an RGBA image with the percentages baked in — no native menu, click toggles the popup window.

## Tauri gotchas (learned the hard way)

### Per-window capabilities

Any window you create at runtime (not just the ones in `tauri.conf.json`) **must be listed in [src-tauri/capabilities/default.json](src-tauri/capabilities/default.json)** under `windows`. Otherwise `event.listen()`, devtools, and other APIs throw at runtime and the frontend silently breaks.

Symptom: `event.listen not allowed on window "popup"` in the webview console, then any code after the `await listen(...)` never runs.

Fix: add the window label (e.g. `"popup"`) to the `windows` array.

### Transparent windows on macOS — the native glassmorphism path

To get a popup that looks like a native macOS menu (blurred background, rounded corners, no gray rectangle):

1. `tauri.conf.json`: `"macOSPrivateApi": true`
2. `Cargo.toml`: `tauri = { features = ["macos-private-api", ...] }` and `window-vibrancy = "0.7"` under `[target.'cfg(target_os = "macos")'.dependencies]`
3. `WebviewWindowBuilder`:
   - `.transparent(true)`
   - `.background_color(tauri::window::Color(0, 0, 0, 0))` — **required**, not optional. `transparent(true)` alone only makes the NSWindow transparent; the WKWebView on top stays opaque gray unless you also set an alpha-0 background color (wry only calls `webview.setOpaque(false)` when `background_color` is explicitly set).
   - `.shadow(false)` — with `shadow(true)` on an undecorated transparent window you hit the Sonoma shadow-ghosting bug ([tauri#8255](https://github.com/tauri-apps/tauri/issues/8255)).
4. **After the window is built**, call `apply_liquid_glass` from the `window-vibrancy` crate instead of Tauri's built-in `WindowEffect` enum. The built-in `Popover`/`HudWindow`/`Sidebar`/`Selection` effects all produce a heavily tinted `NSVisualEffectView` that looks opaque over dark backgrounds — they do not give real glassmorphism. The `window-vibrancy` crate exposes `NSGlassEffectView` (macOS 26.0+, the Liquid Glass material used by Control Center, the Dock, etc.) which is genuinely translucent over any background. Example:
   ```rust
   #[cfg(target_os = "macos")]
   use window_vibrancy::{apply_liquid_glass, NSGlassEffectViewStyle};

   #[cfg(target_os = "macos")]
   {
       let _ = apply_liquid_glass(
           &window,
           NSGlassEffectViewStyle::Clear,   // real see-through glass
           Some((20, 20, 25, 180)),         // dark tint to keep text legible over light backdrops
           Some(12.0),                      // corner radius
       );
   }
   ```
   - `NSGlassEffectViewStyle::Clear` is the most translucent variant. By itself it's invisible over light backdrops (text sums out), so pair it with a dark `tint_color` RGBA. The tint is a fixed overlay on top of the glass — the blur stays intact, you're just protecting contrast.
   - Use the *published* crate (`0.7.x` on crates.io), not the `dev` branch — the API shape changed. In 0.7.1 you pass `&window` directly, the crate does the `raw_window_handle` dance internally.
   - Requires macOS 26.0 (Tahoe) or newer. On earlier macOS versions `apply_liquid_glass` returns `Err(UnsupportedPlatformVersion)` — fall back to `apply_vibrancy` if you need to support older releases.
5. CSS: don't try to simulate glassmorphism with `backdrop-filter` + `rgba` backgrounds. It reintroduces the gray rectangle bleed-through the native effect avoids. Keep the container CSS minimal — just padding and text color — and let the native effect show through. Also make sure `html, body { background: transparent !important; }`.

Window effects only apply when the window is created, so after changing any of this you must fully restart `pnpm tauri dev` — not just HMR.

### Polling and rate limits

The Anthropic `/api/oauth/usage` endpoint is a plain GET, not inference. 2-minute polling (the default) = 30 req/h, well under any reasonable limit. The code already handles 429 with `Retry-After`. Don't lower the interval below ~30s without a reason.

## Dev workflow

- `pnpm tauri dev` — runs Vite + cargo. Changes to Rust require a restart; Svelte hot-reloads.
- Frontend console is via Safari → Develop → (app name) → popup. Capabilities must allow devtools on that window label.
- `cargo check` from `src-tauri/` for quick Rust validation without rebuilding the app.

## Agent skills

### Issue tracker

Local markdown under `.scratch/<feature>/`. See `docs/agents/issue-tracker.md`.

### Triage labels

Canonical defaults (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context (`CONTEXT.md` + `docs/adr/` at repo root). See `docs/agents/domain.md`.
