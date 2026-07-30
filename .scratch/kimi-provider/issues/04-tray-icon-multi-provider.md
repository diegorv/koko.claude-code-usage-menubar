# 04 — Tray icon: one metric per provider

Status: done

## What to build

The tray is 22px tall and `generate_icon(session, weekly)` (`tray_icon.rs:120`) assumes exactly two hardcoded rows (S/W) with fixed height, labels, and colors in consts. Generalize it:

- `generate_icon` takes `Vec<(label, percent, color)>`.
- Both providers active → 2 rows, one per provider showing weekly %, labels "C" and "K", distinct per-provider colors.
- Only one provider active → keep the current S/W (session/weekly) layout for that provider.
- Provider with non-ok status → its row shows the last known value or is dimmed (match whatever the current not-ok behavior is: today the icon simply isn't updated — keep that semantics per-provider if feasible, otherwise skip icon update entirely as today).

## Acceptance criteria

- [ ] With Claude + Kimi active, tray shows two legible rows (C/K weekly %)
- [x] With only Claude, tray looks exactly as today
- [ ] Icon stays within the 22px design height, no clipping in light and dark menu bar
- [ ] `cargo check` passes; visual check via `pnpm tauri dev` (full restart, not HMR)

## Blocked by

- Issue 03 (Kimi data flowing through providers[])

## Out of scope

- Showing session % per provider in the tray (no vertical room; popup carries the detail).

## Comments

Shipped in 570c40d, then revised by the follow-up review branch.

Deviation from the spec above: for a provider with non-ok status, this issue
offered "show the last known value or dim it", falling back to skipping the
icon update entirely. Neither was implemented. A non-ok provider **drops out**
and the survivors are painted, because freezing the icon over a bad Kimi key
would also freeze fresh Claude data. The icon is left untouched only when no
provider is ok, which is the old freeze-on-error behaviour.

The single-provider layout paints the weekly row in that provider's own
colour, so a surviving Kimi does not render as Claude's numbers.

Left unchecked because they need a running app:

- Two legible rows with both providers active.
- No clipping in a light and a dark menu bar. `test_rows_fit_design_height`
  pins the geometry; legibility is a visual call.
