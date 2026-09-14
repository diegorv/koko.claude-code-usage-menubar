# Claude Code Usage Menubar

Tauri v2 (Rust) + SvelteKit macOS menubar app that shows Claude usage percentages in the tray icon and opens a popup on click.

## Architecture notes

- **Native polling lives in Rust** ([commands.rs](src-tauri/src/commands.rs)) — not in the WebView — so it runs even when the popup is hidden. The frontend only displays data; it doesn't own the refresh loop.
- **Two data paths into the popup**: (1) `invoke('trigger_refresh')` on mount, and (2) `listen('usage_updated')` for push updates from Rust-side polling. Both must work for the popup to show data immediately on first open.
- **`trigger_refresh` has a 30s throttle** with `LAST_FETCH` + `LAST_PAYLOAD` caches. When throttled, it returns the cached payload instead of refetching. The frontend mirrors this with a 30s cooldown on the Refresh button (bouncing dots animation while disabled).
- **The payload cache stores every fetch, ok or not.** It used to keep only the last *ok* payload, which was defensible with one provider ("keep the last known-good numbers") and wrong with two: a failing Claude froze the whole cache, so a Kimi provider that had just answered fine was served from a copy that could be days old, and a Kimi key removed while Claude was down went on rendering. The cache means "what the last fetch produced" — nothing more. The tray still freezes on error (see `tray_rows`), because a baked icon has no way to say "this is stale".
- **The payload is `providers: Vec<ProviderPayload>`**, in the fixed order Claude, Kimi, GPT. A provider turned off in Settings, or with no credentials, is *omitted* from the array, not marked `disabled` — so a keyless install with default settings produces a payload byte-identical to a Claude-only build. `ProviderStatus::Disabled` exists but nothing emits it. Don't index `providers[0]` as "Claude": Claude can be turned off.
- **Provider toggles live in two places on purpose.** The frontend persists them (`providers` in settings.json, [providers.ts](src/lib/providers.ts)); Rust holds a copy in `ProviderSettingsState`, loaded from that file at startup and replaced by `set_provider_settings`, because the poller runs with the popup closed. A disabled provider is never fetched — no keychain read, no request. GPT defaults **off**: its credentials are found on disk, not pasted, so defaulting it on would start sending a token to a new host just because Codex CLI is installed.
- **The tray derives rows and tooltip from the same list** (`painted_providers` in commands.rs): ok providers picked for the tray only, capped at `MAX_ROWS`. Deriving the tooltip separately is how it ended up naming a provider that wasn't on the icon. A failing pick drops out with no stand-in — borrowing an unpicked provider would put popup-only numbers on the icon. Only when *none* of the payload's providers is picked does payload order decide, so the icon can't freeze forever.
- **Kimi's session bucket is found by its window, not by `limits[0]`.** The 300-minute entry (`window.duration == 300 && window.timeUnit == "TIME_UNIT_MINUTE"`) is what makes the popup's "Session (5h)" label true. Indexing position would silently report another bucket's numbers the day Kimi prepends one — and `shape_warning` could not catch it, because `limits[0]` would still exist. No match counts as drift and warns. Same lesson as the Claude `limits[]` note below; it applies to every provider.
- **GPT windows are found by length, not by slot.** `chatgpt.com/backend-api/wham/usage` is undocumented; the session window is whichever of `rate_limit.primary_window` / `secondary_window` has `limit_window_seconds == 18000`, weekly is `604800`. No match warns. The test body in `gpt_parser.rs` is hand-written, not captured — replace it with a neutralised live capture in `fixtures/` when one exists.
- **`~/.codex/auth.json` is read-only to this app** ([gpt_auth.rs](src-tauri/src/state/gpt_auth.rs)). Codex CLI owns and refreshes that token; refreshing or writing it from here would race Codex and could log the user out. It is re-read every poll (no cache), which is what picks up a rotated token. Errors from reading it are fixed strings — never forward a serde diagnostic or any file contents.
- **Tray icon is generated in Rust** ([tray_icon.rs](src-tauri/src/tray_icon.rs)) as an RGBA image with the percentages baked in — no native menu, click toggles the popup window.
- **Per-model usage comes from `limits[]`, not the `seven_day_*` keys.** The API reshaped twice in July 2026. `seven_day_sonnet` / `seven_day_opus` are still present but permanently `null`; per-model figures now arrive as `limits[]` entries with `kind: "weekly_scoped"`, carrying `scope.model.display_name` and an integer `percent`. Iterate the array — never assume a fixed model set. Do **not** filter on `is_active`: only the session limit is ever `true`, so filtering hides every model. A captured payload is pinned in `src-tauri/fixtures/usage_response.json`; both reshapes were silent (200 OK, just less data), which is why `parse_api_response` sets a `shape_warning` when `limits` is missing entirely.

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

### Reading the keychain without a password prompt

**Do not "modernise" [token_cache.rs](src-tauri/src/state/token_cache.rs) to call the Keychain API in-process.** It reads the OAuth token by shelling out to `/usr/bin/security`, and that is deliberate.

macOS grants keychain access per requesting binary, matched against that binary's **designated requirement**. An ad-hoc signed binary — which is what every unsigned local build is — has a DR that is a literal hash of its own code. One byte of Rust changes it, macOS sees a different application, and the "Always Allow" grant no longer applies. Using `security_framework` in-process meant a login-password dialog several times a day. `/usr/bin/security` is Apple-signed with a stable DR, so a grant given to it holds permanently.

Two more things kept the prompts coming, both fixed and both easy to reintroduce:

- `invalidate()` must **not** drop the cached token. The API rejects a token whenever Claude Code rotates it, and the replacement only appears in the keychain some time later. Dropping the value made every poll re-read the keychain — one prompt per polling interval until rotation finished. It now marks the entry stale and keeps the value, behind a 10-minute floor between reads.
- Every `security` invocation is bounded by a timeout with a kill fallback. A targeted lookup answers in ~10ms, but the subprocess has been observed to hang on some macOS 26.x setups.

Errors from that subprocess report **stderr only** — stdout carries the secret.

#### The same rules apply to [kimi_key.rs](src-tauri/src/state/kimi_key.rs), plus three of its own

`token_cache.rs` only ever *reads*. `kimi_key.rs` also writes and deletes, which brings problems the token path never had:

- **The key must never be a subprocess argument.** Process arguments are readable by any process running as the same user (`ps -ww`), and `security(1)` says so itself: `-w password  Specify password to be added. Put at end of command to be prompted (recommended)`. So `save_args()` ends in a bare `-w` and the secret goes to the child's stdin. If you ever "simplify" that back to `-w <key>`, you have reintroduced the leak.
- **Write the key to stdin twice.** `security` prompts for the value and then for a confirmation, and reads both from stdin when it has no terminal. A single line makes the two reads disagree — and it then stores an **empty password while still exiting 0**. That silent-success mode is why `save()` reads the value back before reporting success. The two prompts land on *stderr*, so they are stripped before any error string is built.
- **`exists()` must not pass `-w`.** `find-generic-password` answers existence with its exit status; `-w` makes `security` decrypt the secret and print it, only to be discarded. `read()` has its own arg list for that.

Unlike `token_cache.rs`, `read()` has **no minimum interval between keychain reads** and is called once per poll. That is deliberate and not an oversight: the 10-minute floor exists because the Claude item's ACL belongs to another application's binary, so each read can prompt. This item is created by `/usr/bin/security` itself — per the man page, "the application which creates an item is trusted to access its data without warning" — so reads never prompt.

### Tauri commands: sync means the main thread

`#[tauri::command]` on a **non-async** fn compiles to `ExecutionContext::Blocking` (see `tauri-macros/src/command/wrapper.rs`), which runs the handler inline on the IPC thread. Anything that shells out — every keychain call here — freezes the whole app, tray included, for as long as the subprocess takes, and `/usr/bin/security` is bounded at 3s, not 3ms.

Keychain commands are therefore `async fn` and hand the blocking call to `tauri::async_runtime::spawn_blocking` (`on_keychain_thread` in commands.rs).

The same trap applies inside `tokio::join!`: it polls its branches in order on one task, so two "concurrent" fetches that each *begin* with a synchronous subprocess call run their subprocesses back to back. `fetch_usage_payload` takes the `AppHandle` rather than the state refs precisely so both keychain reads can be moved to the blocking pool, which needs an owned `'static` handle.

### Signing local builds

`pnpm tauri build` produces an ad-hoc signed app unless the identity is in the environment:

```bash
export APPLE_SIGNING_IDENTITY="Developer ID Application: … (TEAMID)"
pnpm tauri build
```

`pnpm build:mac` ([scripts/build-signed.sh](scripts/build-signed.sh)) does that export for you: it takes the single "Developer ID Application" identity from `security find-identity -v -p codesigning`, and exits instead of building ad-hoc when there are none or several. An identity already in the environment wins.

Deliberately not in `tauri.conf.json`: hardcoding one developer's identity would break every other build. CI passes the same variable from repository secrets.

Check a build with `codesign -d -r- <app> | grep -c cdhash` — it must print `0`. A `cdhash` in the designated requirement means the build is still ad-hoc, and any keychain grant it holds will die on the next rebuild. See [docs/plans/stable-code-signing.md](docs/plans/stable-code-signing.md).

Setting `APPLE_SIGNING_IDENTITY` as a GitHub secret, watch for a trailing newline — `gh secret set` from an interactive paste captures one, and the build then fails with `certificate from APPLE_CERTIFICATE … does not match provided identity`. Use `printf '%s' '…' | gh secret set NAME`.

### Polling and rate limits

The Anthropic `/api/oauth/usage` endpoint is a plain GET, not inference. 2-minute polling (the default) = 30 req/h, well under any reasonable limit. The code already handles 429 with `Retry-After`. Don't lower the interval below ~30s without a reason.

## Dev workflow

- `pnpm tauri dev` — runs Vite + cargo. Changes to Rust require a restart; Svelte hot-reloads.
- Frontend console is via Safari → Develop → (app name) → popup. Capabilities must allow devtools on that window label.
- `cargo check` from `src-tauri/` for quick Rust validation without rebuilding the app.

### Dependencies

[pnpm-workspace.yaml](pnpm-workspace.yaml) enforces a 7-day quarantine (`minimumReleaseAge`): a version published less than a week ago will not resolve. `pnpm update --latest` can therefore leave `package.json` pointing at a floor no mature version satisfies, and the next install dies with `ERR_PNPM_NO_MATURE_MATCHING_VERSION`. Lower the offending floor rather than relaxing the policy. Run `pnpm outdated` (`scripts/check-outdated-quarantine.mjs`) to see what is actually installable today instead of what npm advertises.

Both `overrides` entries and the `chokidar` trust waiver were verified to be load-bearing — dropping either breaks something concretely. Don't prune them as dead config without re-testing.

**TypeScript is capped below 7.** TS 7 changed its module export shape and svelte-check still reads `typescript.default`, so `pnpm check` crashes before it type-checks anything. `pnpm outdated` cannot see that and will keep advertising the upgrade. Verify by running `pnpm check`, not by trusting the version bump.

`pnpm audit` runs through `scripts/quarantine-aware-audit.mjs` in CI: for up to a week after an advisory lands, the patch exists but the quarantine refuses it, and a plain audit would fail the job for a state the repo cannot fix.

## Agent skills

### Issue tracker

Local markdown under `.scratch/<feature>/`. See `docs/agents/issue-tracker.md`.

### Triage labels

Canonical defaults (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context (`CONTEXT.md` + `docs/adr/` at repo root). See `docs/agents/domain.md`.
