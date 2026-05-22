# 01 — Extract `update_tray_icon` helper

Status: ready-for-agent
Phase: A

## Problem

Tray-icon-update logic is duplicated in `commands.rs`:

- `do_refresh_cycle` (lines ~245-253)
- `trigger_refresh` (lines ~316-323)

Both compute `session_pct` + `weekly_pct`, call `generate_icon`, look up `tray_by_id("main-tray")`, set icon, clear title.

## Change

Extract:

```rust
fn update_tray_icon(app: &AppHandle, payload: &UsagePayload) {
    if payload.status != "ok" { return; }
    let session = payload.session_percent as f64 / 100.0;
    let weekly  = payload.weekly_percent  as f64 / 100.0;
    let icon = crate::tray_icon::generate_icon(session, weekly);
    if let Some(tray) = app.tray_by_id("main-tray") {
        let _ = tray.set_icon(Some(icon));
        let _ = tray.set_title(None::<&str>);
    }
}
```

Replace both call sites.

## Verify

- `cargo check` from `src-tauri/`
- `pnpm tauri dev` → click tray, confirm icon redraws with percentages.

## Out of scope

- Don't change the "status != ok skips icon update" semantics.
- Don't move it out of `commands.rs` yet (that's ticket 04).
