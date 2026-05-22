# Claude Code Usage Menubar

[![License: MIT][license-badge]][license-url] [![Platform: macOS][platform-badge]][platform-url] [![Built with Claude Code][claude-badge]][claude-url]

A tiny macOS menubar app that shows your [Claude Code](https://docs.anthropic.com/en/docs/claude-code) usage percentages directly in the tray icon, with a native popup on click. Built with Tauri 2 and SvelteKit.

The percentages are baked into the tray icon image itself — no native menu, no extra clicks — so you can glance at your remaining quota the same way you check the time.

> [!CAUTION]
> Personal project under active development. **macOS only** (uses private APIs, vibrancy, and the Liquid Glass material from macOS 26). No plans to support other platforms.

> [!NOTE]
> Pull requests and external contributions are not being accepted at this time — this is a solo project. Feel free to fork under the MIT license.

> [!TIP]
> Polls the public `/api/oauth/usage` endpoint every 2 minutes (~30 req/h). It is not an inference call and doesn't affect your usage quota.

## Features

- **Tray icon with live percentages** — the numbers are rendered into the icon pixels in Rust ([tray_icon.rs](src-tauri/src/tray_icon.rs)), so you read your usage without opening anything
- **Native Liquid Glass popup** — real `NSGlassEffectView` translucency (macOS 26+) via the `window-vibrancy` crate, not a CSS fake
- **Auto-adapting icon color** — tray text switches between light and dark to match the macOS appearance
- **Segmented progress bars** in the popup that mirror the tray icon layout
- **Rust-side polling** — the refresh loop lives in the Rust backend, not the WebView, so it keeps running while the popup is hidden
- **Throttled manual refresh** — 30s cooldown on the Refresh button with a bouncing-dots animation, backed by `LAST_FETCH` + `LAST_PAYLOAD` caches
- **429-aware** — respects `Retry-After` headers from the Anthropic API

## Stack

| Layer | Technology |
|-------|-----------|
| Frontend | Svelte 5 (runes), SvelteKit, TypeScript |
| Backend | Tauri 2 (Rust) |
| HTTP | reqwest + tokio |
| Tray rendering | image + imageproc + ab_glyph |
| macOS effects | window-vibrancy (`NSGlassEffectView`) |
| Storage | tauri-plugin-store |
| Package manager | bun |

## Getting Started

### Prerequisites

- macOS 26 (Tahoe) or newer for the Liquid Glass popup — older macOS versions will need a fallback to `apply_vibrancy`
- [Rust](https://rustup.rs) toolchain
- [Bun](https://bun.sh)

### Quick start

```bash
# 1. Install frontend dependencies
bun install

# 2. Run in dev mode (Vite + cargo)
bun run tauri dev
```

### Commands

```bash
bun run tauri dev           # Run app in dev mode (frontend + Tauri)
bun run dev                 # Run frontend only (no Tauri window)
bun run build               # Build frontend for production
bun run tauri build         # Build the full desktop app (release + bundle)
bun run check               # TypeScript + Svelte type checking
bun run test                # Run frontend tests (vitest)
cargo check --manifest-path src-tauri/Cargo.toml   # Quick Rust validation
cargo test  --manifest-path src-tauri/Cargo.toml   # Run Rust tests
```

## Project Structure

```
src/
  lib/
    components/     # Popup UI (Svelte 5)
    store.svelte.ts # Reactive store fed by `usage_updated` events
    usage.ts        # Frontend types + invoke wrappers
  routes/           # SvelteKit popup route

src-tauri/src/
  commands.rs       # Tauri commands: trigger_refresh, polling loop, 30s throttle
  tray_icon.rs      # RGBA tray icon generator with baked-in percentages
  lib.rs            # Setup: tray, popup window, Liquid Glass material
```

## Architecture notes

A couple of decisions worth knowing if you want to hack on this:

- **The refresh loop lives in Rust, not the WebView.** The popup is hidden most of the time, so any polling done from JavaScript would stop the moment the window unloads. `commands.rs` owns the timer and pushes data to the popup with `emit("usage_updated", ...)`.
- **Two data paths into the popup, on purpose.** On mount, the frontend calls `invoke('trigger_refresh')` and also subscribes to `listen('usage_updated')`. Both must work for data to show up immediately on first open.
- **The popup window needs to be in the capabilities file.** Runtime-created windows (`popup`) must be added to `src-tauri/capabilities/default.json` under `windows`, otherwise `event.listen` silently throws and the UI never receives data.

See [CLAUDE.md](CLAUDE.md) for the longer write-up — including the macOS transparent-window gotcha (`background_color(Color(0,0,0,0))` is required, not optional) and why the Sonoma shadow bug forces `shadow(false)`.

## Privacy

- No analytics, no telemetry, no accounts
- The only outbound network call is `GET https://api.anthropic.com/api/oauth/usage`, scoped by CSP in [tauri.conf.json](src-tauri/tauri.conf.json)
- Your OAuth token stays in the macOS Keychain via `security-framework`

## IDE Setup

[VS Code](https://code.visualstudio.com/) + [Svelte](https://marketplace.visualstudio.com/items?itemName=svelte.svelte-vscode) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

## License

Licensed under the [MIT License](LICENSE).

<!-- ─── Badge reference definitions ────────────────────────────── -->

[license-badge]: https://img.shields.io/badge/license-MIT-blue
[license-url]: ./LICENSE
[platform-badge]: https://img.shields.io/badge/platform-macOS-lightgrey?logo=apple&logoColor=white
[platform-url]: https://github.com/diegorv/claude-code-usage-menubar
[claude-badge]: https://img.shields.io/badge/built%20with-Claude%20Code-D97757?logo=anthropic&logoColor=white
[claude-url]: https://docs.anthropic.com/en/docs/claude-code
