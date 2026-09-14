# Claude Code Usage Menubar

[![License: MIT][license-badge]][license-url] [![Platform: macOS][platform-badge]][platform-url] [![Built with Claude Code][claude-badge]][claude-url]

A tiny macOS menubar app that shows your [Claude Code](https://docs.anthropic.com/en/docs/claude-code) usage percentages directly in the tray icon, with a native popup on click. Built with Tauri 2 and SvelteKit.

The percentages are baked into the tray icon image itself — no native menu, no extra clicks — so you can glance at your remaining quota the same way you check the time.

> [!CAUTION]
> Personal project under active development. **macOS only** (uses private APIs, vibrancy, and the Liquid Glass material from macOS 26). No plans to support other platforms.

> [!NOTE]
> Pull requests and external contributions are not being accepted at this time — this is a solo project. Feel free to fork under the MIT license.

> [!TIP]
> Polls the public `/api/oauth/usage` endpoint every 2 minutes (~30 req/h), plus the Kimi usages endpoint if you configure a key and the ChatGPT usage endpoint if you turn GPT on, at the same interval. None is an inference call, and none affects your usage quota.

## Features

- **Tray icon with live percentages** — the numbers are rendered into the icon pixels in Rust ([tray_icon.rs](src-tauri/src/tray_icon.rs)), so you read your usage without opening anything
- **Native Liquid Glass popup** — real `NSGlassEffectView` translucency (macOS 26+) via the `window-vibrancy` crate, not a CSS fake
- **Auto-adapting icon color** — tray text switches between light and dark to match the macOS appearance
- **Segmented progress bars** in the popup that mirror the tray icon layout
- **Rust-side polling** — the refresh loop lives in the Rust backend, not the WebView, so it keeps running while the popup is hidden
- **Throttled manual refresh** — 30s cooldown on the Refresh button with a bouncing-dots animation, backed by `LAST_FETCH` + `LAST_PAYLOAD` caches
- **429-aware** — respects `Retry-After` headers from both provider APIs
- **Optional Kimi provider** — paste a Kimi API key in Settings and the popup grows a second section; the tray switches to one weekly row per provider (`C`/`K`). Without a key the app is Claude-only and makes no request to Kimi
- **Optional GPT provider** — shows your ChatGPT plan's 5h and weekly limits (the ones Codex CLI tracks), using the login Codex CLI already keeps on disk. Off by default
- **Per-provider toggles** — in Settings, turn each provider on or off and pick which ones go in the tray. The icon fits two rows (`C`/`K`/`G`), so with all three on, one stays in the popup only

## Adding a Kimi key

Optional. Open the popup, click **Settings**, paste your Kimi API key and hit **Save**.

The key is stored in the macOS Keychain under the service `koko-kimi-api-key`, never in a file and never in the app's own storage. It stays on the Rust side — only booleans (`Saved` / `Not saved`) ever cross into the WebView. **Remove** deletes it and the popup drops the Kimi section on the next refresh.

## Using GPT

Optional, and off by default. Log in to Codex CLI with your ChatGPT account (`codex login`), then open **Settings** and turn **GPT** on.

The app only *reads* `~/.codex/auth.json` (or `$CODEX_HOME/auth.json`); it never refreshes or rewrites the token — Codex CLI owns that file. If the login expires, running Codex refreshes it and the next poll picks it up. The endpoint is undocumented, so a shape change shows up as a warning in the popup rather than as wrong numbers.

## Stack

| Layer | Technology |
|-------|-----------|
| Frontend | Svelte 5 (runes), SvelteKit, TypeScript |
| Backend | Tauri 2 (Rust) |
| HTTP | reqwest + tokio |
| Tray rendering | image + imageproc + ab_glyph |
| macOS effects | window-vibrancy (`NSGlassEffectView`) |
| Storage | tauri-plugin-store |
| Package manager | pnpm |

## Getting Started

### Prerequisites

- macOS 26 (Tahoe) or newer for the Liquid Glass popup — older macOS versions will need a fallback to `apply_vibrancy`
- [Rust](https://rustup.rs) toolchain
- [pnpm](https://pnpm.io)

### Quick start

```bash
# 1. Install frontend dependencies
pnpm install

# 2. Run in dev mode (Vite + cargo)
pnpm tauri dev
```

### Commands

```bash
pnpm tauri dev              # Run app in dev mode (frontend + Tauri)
pnpm dev                    # Run frontend only (no Tauri window)
pnpm build                  # Build frontend for production
pnpm tauri build            # Build the full desktop app (release + bundle)
pnpm check                  # TypeScript + Svelte type checking
pnpm test                   # Run frontend tests (vitest)
cargo check --manifest-path src-tauri/Cargo.toml   # Quick Rust validation
cargo test  --manifest-path src-tauri/Cargo.toml   # Run Rust tests
```

### Production build

```bash
export APPLE_SIGNING_IDENTITY="Developer ID Application: … (TEAMID)"
pnpm tauri build
```

The app lands in `src-tauri/target/release/bundle/macos/` and the disk image in `bundle/dmg/`.

Without `APPLE_SIGNING_IDENTITY` the build is ad-hoc signed, and any keychain "Always Allow" grant it holds dies on the next rebuild — macOS starts asking for your password again. Check a build with:

```bash
codesign -d -r- "src-tauri/target/release/bundle/macos/<App>.app" 2>&1 | grep -c cdhash   # must print 0
```

## Project Structure

```
src/
  lib/
    components/     # Popup UI (Svelte 5)
    store.svelte.ts # Reactive store fed by `usage_updated` events
    usage.ts        # Frontend types + invoke wrappers
    providers.ts    # Provider on/off + tray picks, persisted in settings.json
  routes/           # SvelteKit popup route

src-tauri/src/
  commands.rs       # Tauri commands, per-provider fetch, tray rows, 30s throttle
  parser.rs         # Shared payload types + the Claude response parser
  kimi_parser.rs    # The Kimi response parser
  gpt_parser.rs     # The ChatGPT/Codex plan response parser
  tray_icon.rs      # RGBA tray icon generator with baked-in percentages
  lib.rs            # Setup: tray, popup window, Liquid Glass material
  state/
    token_cache.rs  # Claude OAuth token, read via /usr/bin/security
    kimi_key.rs     # Kimi API key, stored via /usr/bin/security
    gpt_auth.rs     # ChatGPT token, read (never written) from Codex CLI's auth.json
    provider_settings.rs # Which providers are fetched and which reach the tray
    payload_cache.rs# Last payload + fetch timestamp behind the 30s throttle
    poller.rs       # The native refresh timer
```

## Architecture notes

A couple of decisions worth knowing if you want to hack on this:

- **The refresh loop lives in Rust, not the WebView.** The popup is hidden most of the time, so any polling done from JavaScript would stop the moment the window unloads. `commands.rs` owns the timer and pushes data to the popup with `emit("usage_updated", ...)`.
- **Two data paths into the popup, on purpose.** On mount, the frontend calls `invoke('trigger_refresh')` and also subscribes to `listen('usage_updated')`. Both must work for data to show up immediately on first open.
- **The popup window needs to be in the capabilities file.** Runtime-created windows (`popup`) must be added to `src-tauri/capabilities/default.json` under `windows`, otherwise `event.listen` silently throws and the UI never receives data.

See [CLAUDE.md](CLAUDE.md) for the longer write-up — including the macOS transparent-window gotcha (`background_color(Color(0,0,0,0))` is required, not optional) and why the Sonoma shadow bug forces `shadow(false)`.

## Privacy

- No analytics, no telemetry, no accounts
- Up to three outbound calls, all plain GETs and all made from Rust: `https://api.anthropic.com/api/oauth/usage` while Claude is on, `https://api.kimi.com/coding/v1/usages` only when a Kimi key is configured, and `https://chatgpt.com/backend-api/wham/usage` only when GPT is turned on. A provider turned off in Settings is not contacted at all. The allowed hosts are enforced by the [privacy workflow](.github/workflows/privacy.yml), which fails CI on any new host — not by the CSP in `tauri.conf.json`, which governs the WebView and never sees these requests
- Your Claude OAuth token and your Kimi API key both stay in the macOS Keychain, read and written by shelling out to `/usr/bin/security`. Neither ever crosses into the WebView, and the Kimi key is passed to that subprocess on stdin so it never appears in a process argument list
- The ChatGPT token is read from Codex CLI's `auth.json` on the Rust side and sent only to `chatgpt.com`. It never crosses into the WebView, and error messages never quote that file

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
