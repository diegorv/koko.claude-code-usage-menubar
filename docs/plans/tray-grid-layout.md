# Plan: session *and* weekly on the tray, per provider

Status: designed, not implemented.

## Why

The tray shows two rows and no more, because `MAX_ROWS` is 2 and a menubar icon is
22 design points tall. With one provider those rows are `S:`/`W:` — session and
weekly for that provider. With two they become `C:`/`K:` — **weekly only**, one row
per provider, because there was no third and fourth row for the session figures.

So adding Kimi silently cost every session number on the icon. The popup still has
them, but the popup needs a click; the icon is the whole point of the app.

Rows cannot grow: the height is fixed by the menubar. Width can. This plan spends
width to get all four numbers back.

## The layout

One row per provider. Two cells per row: session on the left, weekly on the right.

```
C  S 12% ▮▯▯▯▯   W 45% ▮▮▮▯▯
K  S 30% ▮▮▯▯▯   W 60% ▮▮▮▯▯
```

Metrics, in design points, alongside the constants they replace in
[src-tauri/src/tray_icon.rs](../../src-tauri/src/tray_icon.rs):

| what | value | note |
|---|---|---|
| row label | 10 | `C`/`K` plus gap; new |
| bucket label | 8 | `S`/`W`, one char, **no colon** — the colon bought nothing at this width |
| value | 32 | 4 chars, unchanged `VALUE_CHARS`; `100%` is the worst case |
| bar | 24 | 5 segments of 4pt with 1pt gaps; `NUM_SEGMENTS` drops 10 → 5 |
| cell | 65 | `8 + 32 + 1 + 24` |
| cell gap | 6 | new |
| **design width** | **148** | `1 + 10 + 65 + 6 + 65 + 1`; today 102 |

`DESIGN_HEIGHT`, `LINE_HEIGHT`, `SCALE`, `FONT_SIZE`, `MAX_ROWS`: unchanged.

Bars lose half their resolution — 20% per segment instead of 10%. The number beside
each bar stays exact, and the bar was always the glanceable indicator, not the
readout. That is the price of a second column at a width worth paying.

## Colors

Unchanged convention, new distribution of work:

- Session bar: `COLOR_SESSION` (blue) in every row.
- Weekly bar: the provider's color — `COLOR_WEEKLY` (purple) for Claude,
  `COLOR_KIMI` (teal) for Kimi.
- `bar_color` escalates each bar to amber at 80% and red at 95% **independently**,
  so a maxed session sits red next to a calm weekly.

Provider identity now comes from the row's letter, not from color. Color still
separates the two *buckets* — which is what it did in the original `S`/`W` layout.

## One provider

Width is fixed at 148 whether one provider is ok or two. A Claude-only install pays
the full width and gets a single grid row, centred vertically at
`line_y = (DESIGN_HEIGHT - LINE_HEIGHT) / 2 = 5`.

The alternative — narrow icon for one provider, wide for two — was rejected: a
provider drops off the icon whenever it errors, so the menubar item would resize
every time a Kimi key hit a 401 and recovered, shoving every icon to its right.

This also deletes the `S:`/`W:` two-row layout entirely. A single grid row already
carries both numbers, so the special case in `tray_rows` for `ok.len() == 1` goes
away with it.

## Code changes

**[src-tauri/src/tray_icon.rs](../../src-tauri/src/tray_icon.rs)**

- `generate_icon` takes grid rows, `(char, f64, f64, Rgba<u8>)` —
  `(provider label, session 0.0..=1.0, weekly 0.0..=1.0, provider color)` — instead
  of today's `(label, pct, color)`.
- `draw_line` becomes a row painter: row label, then two cells at fixed X offsets.
  Factor the cell (bucket label + value + bar) into its own function so both columns
  paint through the same code.
- New width constants per the table above; `NUM_SEGMENTS` 10 → 5.
- Vertical centring when `rows.len() == 1`.

**[src-tauri/src/commands.rs](../../src-tauri/src/commands.rs)**

- `tray_rows` loses the `ok.len() == 1` branch and becomes a plain map over
  `painted_providers`: `(label, session_percent / 100, weekly_percent / 100, color)`.
- `painted_providers`, `tray_identity`, `tray_tooltip`, the `None`-on-no-ok-provider
  freeze: unchanged.

**[src-tauri/src/lib.rs](../../src-tauri/src/lib.rs)** — the startup placeholder icon
(`('S', 0.0, …), ('W', 0.0, …)`) becomes one empty grid row, `('C', 0.0, 0.0, COLOR_WEEKLY)`.

## Tests

Rewrite, not extend — the row tuple changes shape, so every existing assertion moves.

`commands.rs`:

- `single_provider_keeps_session_weekly_rows` → one grid row carrying *both*
  percentages, labelled `C`.
- `non_ok_kimi_keeps_claude_session_weekly_rows` and
  `non_ok_claude_keeps_kimi_session_weekly_rows` → same, asserting the survivor's own
  label and color (the Kimi case is the one that used to read as Claude's numbers).
- `two_providers_show_weekly_rows_with_provider_labels` → now asserts session *and*
  weekly per row. Rename: it is no longer weekly-only.
- `rows_are_capped_at_max_rows`, `both_providers_non_ok_skips_update`,
  `non_ok_claude_skips_update`, `empty_providers_skips_update`, both tooltip tests:
  unchanged behaviour, mechanical fixes only where they index the tuple.

`tray_icon.rs`:

- `sw_rows` helper → `grid_rows`.
- `two_rows_paint_both_bands` keeps its point: both 11px bands must have painted
  pixels with two rows.
- Add: a single row paints in the *centre* band, not the top one — that is the new
  vertical-centring path, and nothing else covers it.
- Add: a row paints in all four cell regions, so a cell landing at the wrong X is
  caught the way `two_rows_paint_both_bands` catches a wrong Y.
- `test_rows_fit_design_height` stays; add its width twin — the rightmost cell's bar
  must end inside `DESIGN_WIDTH`.

## Out of scope

The popup, the payload, the tooltip text, the polling loop, and the
`ProviderStatus::Disabled` dead branch. Nothing about this changes what is fetched —
only what is painted.
