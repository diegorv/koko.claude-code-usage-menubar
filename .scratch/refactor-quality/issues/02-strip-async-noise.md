# 02 — Strip cosmetic `async` from sync Tauri commands

Status: ready-for-agent
Phase: A

## Problem

Two `#[tauri::command] async fn`s in `commands.rs` have zero awaits:

- `start_auto_refresh(app, interval_secs) -> Result<(), String>`
- `quit_app(app)`

Misleading: signature implies I/O. Reader has to chase the body to confirm it's sync.

## Change

Drop `async` from both. Tauri handles sync commands identically.

## Verify

- `cargo check`.
- Frontend `invoke('start_auto_refresh', ...)` and `invoke('quit_app')` continue to work — Tauri promises a Promise either way.
- `pnpm tauri dev` → change interval, click Quit.

## Out of scope

- Don't touch `trigger_refresh` — it's genuinely async.
- Don't change return type or error contract.
