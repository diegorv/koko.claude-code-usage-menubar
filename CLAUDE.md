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
2. `Cargo.toml`: `tauri = { features = ["macos-private-api", ...] }`
3. `WebviewWindowBuilder`:
   - `.transparent(true)`
   - `.background_color(tauri::window::Color(0, 0, 0, 0))` — **required**, not optional. `transparent(true)` alone only makes the NSWindow transparent; the WKWebView on top stays opaque gray unless you also set an alpha-0 background color (wry only calls `webview.setOpaque(false)` when `background_color` is explicitly set).
   - `.shadow(false)` — with `shadow(true)` on an undecorated transparent window you hit the Sonoma shadow-ghosting bug ([tauri#8255](https://github.com/tauri-apps/tauri/issues/8255)).
   - `.effects(EffectsBuilder::new().effect(WindowEffect::Popover).state(WindowEffectState::Active).radius(12.0).build())` — this gives you the native `NSVisualEffectView` vibrancy (same component macOS menus use), with real window-level rounded corners and automatic light/dark adaptation.
4. CSS: don't try to simulate glassmorphism with `backdrop-filter` + `rgba` backgrounds. The native effect does it better and avoids the gray rectangle bleed-through. Keep the container CSS minimal — just padding and text color — and let the native effect show through.

Window effects only apply when the window is created, so after changing this you must fully restart `pnpm tauri dev` — not just HMR.

### Polling and rate limits

The Anthropic `/api/oauth/usage` endpoint is a plain GET, not inference. 2-minute polling (the default) = 30 req/h, well under any reasonable limit. The code already handles 429 with `Retry-After`. Don't lower the interval below ~30s without a reason.

## Dev workflow

- `pnpm tauri dev` — runs Vite + cargo. Changes to Rust require a restart; Svelte hot-reloads.
- Frontend console is via Safari → Develop → (app name) → popup. Capabilities must allow devtools on that window label.
- `cargo check` from `src-tauri/` for quick Rust validation without rebuilding the app.
