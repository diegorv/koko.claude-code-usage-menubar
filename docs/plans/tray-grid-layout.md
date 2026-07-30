# Plan: session *and* weekly on the tray, per provider

Status: implemented.

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
| row label | 13 | `C`/`K` plus gap; new |
| session label | 13 | `S` (7.56pt) + `LABEL_GAP`; **no colon** — it bought nothing at this width |
| weekly label | 17 | `W` (11.77pt) + `LABEL_GAP` |
| label gap | 5 | new; the same gap after every label |
| value | 32 | 4 chars, unchanged `VALUE_CHARS`; `100%` is the worst case |
| bar gap | 3 | was 1 |
| bar | 24 | 5 segments of 4pt with 1pt gaps; `NUM_SEGMENTS` drops 10 → 5 |
| session cell | 72 | `13 + 32 + 3 + 24` |
| weekly cell | 76 | `17 + 32 + 3 + 24` |
| cell gap | 6 | new |
| **design width** | **169** | `1 + 13 + 72 + 6 + 76 + 1`; before, 102 |

`DESIGN_HEIGHT`, `LINE_HEIGHT`, `SCALE`, `FONT_SIZE`, `MAX_ROWS`: unchanged.

No label width derives from `CHAR_ADVANCE`. That constant is the *digit* advance;
capitals are wider and differ from each other, so each label column is sized from
its own glyph plus the shared `LABEL_GAP`. Two rounds of guessing got this wrong —
first at 8pt, where `CS12%` and `W100%` rendered with the glyphs touching, then at
a single 13pt bucket width, which left `W` flush against its percentage while `S`
had 5pt to spare. Nothing in the test module can catch either: every assertion
passes on a layout whose text overlaps.

Two `#[ignore]`d tests replace the guessing, and are what to reach for the next
time these metrics move:

- `print_glyph_advances` — the real advance of every glyph the icon draws, in the
  font actually loaded. Source of the numbers in the table above.
- `dump_icon_png` — writes the icon out for a human to look at.

Each bucket letter owns its column, so per-column widths cost nothing in
alignment: the `S` column is always `S`, the `W` column always `W`, and the
percentages still line up down the icon. The row-label column is the one
exception — it carries both `C` (8.74pt) and `K` (7.65pt), so the gap after it
varies by ~1pt between rows. Sizing it per letter would shift the whole session
column between rows, which is a worse trade.

Bars lose half their resolution — 20% per segment instead of 10%. The number beside
each bar stays exact, and the bar was always the glanceable indicator, not the
readout. That is the price of a second column at a width worth paying.

One consequence needed fixing: `round(5 × 0.07) == 0`, so every percentage under
10% painted a completely idle bar, which ten segments never did. `filled_segments`
now floors any non-zero percentage at one lit segment.

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

Width is fixed at 169 whether one provider is ok or two. A Claude-only install pays
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

- `generate_icon` takes grid rows — `TrayRow`, i.e.
  `(provider label, session 0.0..=1.0, weekly 0.0..=1.0, provider color)` — instead
  of the old `(label, pct, color)`.
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

- `single_provider_keeps_session_weekly_rows` → `single_provider_gets_one_row_with_both_figures`:
  one grid row carrying *both* percentages, labelled `C`.
- `non_ok_kimi_keeps_claude_session_weekly_rows` and
  `non_ok_claude_keeps_kimi_session_weekly_rows` → `..._keeps_claudes_row` /
  `..._keeps_kimis_row`: same, asserting the survivor's own label and color (the
  Kimi case is the one that used to read as Claude's numbers).
- `two_providers_show_weekly_rows_with_provider_labels` →
  `two_providers_show_session_and_weekly_per_provider`, asserting both figures per
  row. It is no longer weekly-only, so the old name lied.
- `rows_are_capped_at_max_rows`, `both_providers_non_ok_skips_update`,
  `non_ok_claude_skips_update`, `empty_providers_skips_update`, both tooltip tests:
  unchanged behaviour, mechanical fixes only where they index the tuple.

`tray_icon.rs`:

- `sw_rows` helper → `grid_rows`.
- `two_rows_paint_both_bands` keeps its point: both 11px bands must have painted
  pixels with two rows.
- `a_single_row_paints_the_centre_band` — the new vertical-centring path, which
  nothing else covers; also asserts the band above it stays empty.
- `a_row_paints_both_cells` — both bar regions painted, so a cell landing at the
  wrong X is caught the way `two_rows_paint_both_bands` catches a wrong Y.
- `an_empty_session_leaves_the_weekly_bar_alone` — the two cells are independent.
- `a_nonzero_percentage_lights_a_segment` — the five-segment rounding floor.
- `test_rows_fit_design_height` stays; `cells_fit_design_width` is its width twin,
  written as `const` assertions so a bad layout fails the build rather than a run.
- `dump_icon_png` and `print_glyph_advances`, both `#[ignore]`d — see "The layout".

## Out of scope

The popup, the payload, the tooltip text, the polling loop, and the
`ProviderStatus::Disabled` dead branch. Nothing about this changes what is fetched —
only what is painted.
