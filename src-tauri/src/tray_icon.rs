use ab_glyph::FontArc;
use image::{Rgba, RgbaImage};
use imageproc::drawing::{draw_filled_rect_mut, draw_text_mut};
use imageproc::rect::Rect;
use std::sync::{LazyLock, Mutex};
use std::time::Instant;
use tauri::image::Image;

static FONT: LazyLock<FontArc> = LazyLock::new(|| {
    // Try system San Francisco font first
    if let Ok(font_data) = std::fs::read("/System/Library/Fonts/SFNS.ttf") {
        if let Ok(font) = FontArc::try_from_vec(font_data) {
            return font;
        }
    }
    // Fallback to bundled font
    let fallback = include_bytes!("../fonts/JetBrainsMono-Bold.ttf");
    FontArc::try_from_vec(fallback.to_vec()).expect("bundled fallback font is invalid")
});

// Scale factor for retina displays (2x for standard Retina)
const SCALE: u32 = 2;

// Layout constants (in design points, multiplied by SCALE for final pixels)
const DESIGN_HEIGHT: u32 = 22;
const LINE_HEIGHT: u32 = 11;
const LINE_GAP: u32 = 0;
const LINE1_Y: u32 = 0;
/// Rows that fit the design height: one provider per row, each carrying both
/// its session and its weekly figure.
pub(crate) const MAX_ROWS: usize = 2;

// Bar layout. Five segments, not ten: two bars per row is what buys the
// session numbers back, and 20%-per-segment is the price. The percentage
// beside each bar stays exact.
const NUM_SEGMENTS: u32 = 5;
const SEGMENT_WIDTH: u32 = 4;
const SEGMENT_GAP: u32 = 1;
const SEGMENT_HEIGHT: u32 = 10;
const BAR_GAP_FROM_TEXT: u32 = 3;

// Text metrics
const CHAR_ADVANCE: u32 = 8; // approximate advance per character for SF Pro at this size
const VALUE_CHARS: u32 = 4; // "06%" to "100%"
const VALUE_WIDTH: u32 = VALUE_CHARS * CHAR_ADVANCE;
const BAR_TOTAL_WIDTH: u32 = NUM_SEGMENTS * (SEGMENT_WIDTH + SEGMENT_GAP) - SEGMENT_GAP;

// Grid layout: `<provider> <S cell> <W cell>`, one row per provider.
const MARGIN: u32 = 1;
// CHAR_ADVANCE is the digit advance; capitals are wider, so the two label
// widths carry their own slack rather than deriving from it. Verified by
// rendering: at 8pt each, "CS12%" ran together with no gap at all.
const ROW_LABEL_WIDTH: u32 = 13; // 'C'/'K' glyph plus the gap before the first cell
const BUCKET_LABEL_WIDTH: u32 = 13; // 'S'/'W' — 'W' is the widest glyph on the icon
const CELL_WIDTH: u32 = BUCKET_LABEL_WIDTH + VALUE_WIDTH + BAR_GAP_FROM_TEXT + BAR_TOTAL_WIDTH;
const CELL_GAP: u32 = 6;
const SESSION_CELL_X: u32 = MARGIN + ROW_LABEL_WIDTH;
const WEEKLY_CELL_X: u32 = SESSION_CELL_X + CELL_WIDTH + CELL_GAP;
const DESIGN_WIDTH: u32 = WEEKLY_CELL_X + CELL_WIDTH + MARGIN;

// Final pixel dimensions
const ICON_WIDTH: u32 = DESIGN_WIDTH * SCALE;
const ICON_HEIGHT: u32 = DESIGN_HEIGHT * SCALE;

// Font size in pixels (scaled)
const FONT_SIZE: f32 = 15.0 * SCALE as f32;
// Vertical offset to compensate for font ascender space above cap-height glyphs
const TEXT_Y_OFFSET: i32 = -3 * SCALE as i32;

// Colors
pub(crate) const COLOR_SESSION: Rgba<u8> = Rgba([107, 127, 224, 255]);
pub(crate) const COLOR_WEEKLY: Rgba<u8> = Rgba([192, 96, 208, 255]);
// Kimi's identity color in the two-provider layout — the teal the popup
// already uses (#4db6a0), distinct from Claude's blue/purple and the warning
// amber, and proven legible on light and dark surfaces.
pub(crate) const COLOR_KIMI: Rgba<u8> = Rgba([77, 182, 160, 255]);
const COLOR_SEGMENT_OFF: Rgba<u8> = Rgba([140, 140, 140, 80]);
const COLOR_WARNING: Rgba<u8> = Rgba([224, 160, 48, 255]);
const COLOR_CRITICAL: Rgba<u8> = Rgba([224, 80, 80, 255]);

// Percentages at or above these switch the bar away from its identity color.
// Kept in sync with WARNING_THRESHOLD / CRITICAL_THRESHOLD in src/lib/usage.ts.
const WARNING_THRESHOLD: u32 = 80;
const CRITICAL_THRESHOLD: u32 = 95;

/// Filled segments for a percentage. Anything above zero lights at least one:
/// with five segments a plain round leaves everything under 10% looking
/// completely idle, which ten segments never did.
fn filled_segments(clamped: f64) -> u32 {
    if clamped <= 0.0 {
        return 0;
    }
    ((NUM_SEGMENTS as f64 * clamped).round() as u32).max(1)
}

/// Filled-segment color for a percentage: the row's identity color normally,
/// escalating to amber then red so a near-limit bar is obvious at a glance.
fn bar_color(pct_int: u32, base: Rgba<u8>) -> Rgba<u8> {
    if pct_int >= CRITICAL_THRESHOLD {
        COLOR_CRITICAL
    } else if pct_int >= WARNING_THRESHOLD {
        COLOR_WARNING
    } else {
        base
    }
}
const COLOR_TEXT_DARK_BG: Rgba<u8> = Rgba([235, 235, 235, 235]);
const COLOR_TEXT_LIGHT_BG: Rgba<u8> = Rgba([0, 0, 0, 220]);

#[cfg(target_os = "macos")]
fn menubar_is_dark() -> bool {
    // `defaults read -g AppleInterfaceStyle` -> "Dark" when in Dark mode, errors otherwise.
    // Menubar follows the global appearance, so this is sufficient.
    std::process::Command::new("defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim().eq_ignore_ascii_case("Dark"))
        .unwrap_or(true)
}

#[cfg(not(target_os = "macos"))]
fn menubar_is_dark() -> bool {
    true
}

// Cache the appearance lookup so we don't fork+exec `defaults` on every redraw.
// 30s TTL — appearance toggles are rare; one cycle of staleness is fine.
const DARK_MODE_TTL_SECS: u64 = 30;
static DARK_MODE_CACHE: LazyLock<Mutex<Option<(bool, Instant)>>> =
    LazyLock::new(|| Mutex::new(None));

fn menubar_is_dark_cached() -> bool {
    if let Ok(cache) = DARK_MODE_CACHE.lock() {
        if let Some((value, ref ts)) = *cache {
            if ts.elapsed().as_secs() < DARK_MODE_TTL_SECS {
                return value;
            }
        }
    }
    let value = menubar_is_dark();
    if let Ok(mut cache) = DARK_MODE_CACHE.lock() {
        *cache = Some((value, Instant::now()));
    }
    value
}

/// One painted row: provider label, session and weekly in 0.0..=1.0, and the
/// provider's identity color.
pub type TrayRow = (char, f64, f64, Rgba<u8>);

/// Generates a dynamic tray icon, one row per provider, top to bottom. Each
/// row paints two cells: session on the left, weekly on the right.
/// At most MAX_ROWS fit the 22px design height; callers never pass more.
pub fn generate_icon(rows: Vec<TrayRow>) -> Image<'static> {
    let font = &*FONT;
    let mut img = RgbaImage::new(ICON_WIDTH, ICON_HEIGHT);
    let text_color = if menubar_is_dark_cached() { COLOR_TEXT_DARK_BG } else { COLOR_TEXT_LIGHT_BG };

    debug_assert!(rows.len() <= MAX_ROWS);
    // The icon keeps its full width whatever the provider count, so a lone row
    // pinned to the top would sit off-balance in the menubar. Centre it.
    let first_line_y = if rows.len() == 1 {
        (DESIGN_HEIGHT - LINE_HEIGHT) / 2
    } else {
        LINE1_Y
    };
    for (i, &(label, session, weekly, color)) in rows.iter().enumerate() {
        let line_y = first_line_y + i as u32 * (LINE_HEIGHT + LINE_GAP);
        draw_row(&mut img, font, line_y, label, session, weekly, color, text_color);
    }

    Image::new_owned(img.into_raw(), ICON_WIDTH, ICON_HEIGHT)
}

/// Pseudo-bold: draw twice with a 1px horizontal offset.
fn draw_bold_text(img: &mut RgbaImage, font: &FontArc, x: i32, y: i32, color: Rgba<u8>, text: &str) {
    draw_text_mut(img, color, x, y, FONT_SIZE, font, text);
    draw_text_mut(img, color, x + 1, y, FONT_SIZE, font, text);
}

#[allow(clippy::too_many_arguments)]
fn draw_row(
    img: &mut RgbaImage,
    font: &FontArc,
    line_y: u32,
    label: char,
    session: f64,
    weekly: f64,
    color: Rgba<u8>,
    text_color: Rgba<u8>,
) {
    let text_y = (line_y * SCALE) as i32 + TEXT_Y_OFFSET;
    draw_bold_text(img, font, (MARGIN * SCALE) as i32, text_y, text_color, &label.to_string());

    // Session keeps its own color in every row; weekly carries the provider's,
    // so the two buckets stay distinguishable while the row label identifies
    // the provider.
    draw_cell(img, font, line_y, SESSION_CELL_X, 'S', session, COLOR_SESSION, text_color);
    draw_cell(img, font, line_y, WEEKLY_CELL_X, 'W', weekly, color, text_color);
}

#[allow(clippy::too_many_arguments)]
fn draw_cell(
    img: &mut RgbaImage,
    font: &FontArc,
    line_y: u32,
    cell_x: u32,
    label: char,
    pct: f64,
    color: Rgba<u8>,
    text_color: Rgba<u8>,
) {
    let clamped = pct.clamp(0.0, 1.0);
    let pct_int = (clamped * 100.0).round() as u32;
    let value_str = format!("{:02}%", pct_int);

    // Draw label and value separately so percentages align vertically
    let text_y = (line_y * SCALE) as i32 + TEXT_Y_OFFSET;
    let label_x = (cell_x * SCALE) as i32;
    let value_x = ((cell_x + BUCKET_LABEL_WIDTH) * SCALE) as i32;

    draw_bold_text(img, font, label_x, text_y, text_color, &label.to_string());
    draw_bold_text(img, font, value_x, text_y, text_color, &value_str);

    // Draw segmented bar
    let bar_x = cell_x + BUCKET_LABEL_WIDTH + VALUE_WIDTH + BAR_GAP_FROM_TEXT;
    let bar_y = line_y + (LINE_HEIGHT - SEGMENT_HEIGHT) / 2;
    let filled = filled_segments(clamped);

    let filled_color = bar_color(pct_int, color);

    for i in 0..NUM_SEGMENTS {
        let seg_x = (bar_x + i * (SEGMENT_WIDTH + SEGMENT_GAP)) * SCALE;
        let seg_y = bar_y * SCALE;
        let seg_color = if i < filled { filled_color } else { COLOR_SEGMENT_OFF };

        draw_filled_rect_mut(
            img,
            Rect::at(seg_x as i32, seg_y as i32).of_size(SEGMENT_WIDTH * SCALE, SEGMENT_HEIGHT * SCALE),
            seg_color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One grid row per provider, the shape every caller passes.
    fn grid_rows(pairs: &[(char, f64, f64)]) -> Vec<(char, f64, f64, Rgba<u8>)> {
        pairs
            .iter()
            .map(|&(label, session, weekly)| (label, session, weekly, COLOR_WEEKLY))
            .collect()
    }

    /// Row-relative Y of the band a row at `row_index` paints into, in pixels,
    /// for a `row_count`-row icon — mirrors `generate_icon`'s centring.
    fn band_pixels(row_count: usize, row_index: usize) -> (usize, usize) {
        let first = if row_count == 1 { (DESIGN_HEIGHT - LINE_HEIGHT) / 2 } else { LINE1_Y };
        let top = (first + row_index as u32 * (LINE_HEIGHT + LINE_GAP)) * SCALE;
        (top as usize, (top + LINE_HEIGHT * SCALE) as usize)
    }

    /// True when any pixel in the rectangle is painted.
    fn region_painted(rgba: &[u8], x0: u32, x1: u32, y0: usize, y1: usize) -> bool {
        (y0..y1).any(|y| {
            (x0..x1).any(|x| {
                let i = (y * ICON_WIDTH as usize + x as usize) * 4;
                rgba[i..i + 4].iter().any(|&b| b != 0)
            })
        })
    }

    #[test]
    fn test_generate_icon_dimensions() {
        let icon = generate_icon(grid_rows(&[('C', 0.5, 0.3)]));
        assert_eq!(icon.width(), ICON_WIDTH);
        assert_eq!(icon.height(), ICON_HEIGHT);
    }

    #[test]
    fn test_generate_icon_not_empty() {
        let icon = generate_icon(grid_rows(&[('C', 0.5, 0.3)]));
        let rgba = icon.rgba();
        assert!(rgba.iter().any(|&b| b != 0));
    }

    #[test]
    fn test_generate_icon_zero() {
        let _icon = generate_icon(grid_rows(&[('C', 0.0, 0.0), ('K', 0.0, 0.0)]));
    }

    #[test]
    fn test_generate_icon_full() {
        let _icon = generate_icon(grid_rows(&[('C', 1.0, 1.0), ('K', 1.0, 1.0)]));
    }

    #[test]
    fn test_generate_icon_over_range() {
        let _icon = generate_icon(grid_rows(&[('C', 1.5, -0.5)]));
    }

    #[test]
    fn test_rows_fit_design_height() {
        for i in 0..MAX_ROWS {
            let bottom = LINE1_Y + i as u32 * (LINE_HEIGHT + LINE_GAP) + LINE_HEIGHT;
            assert!(bottom <= DESIGN_HEIGHT);
        }
    }

    #[test]
    fn cells_fit_design_width() {
        // The rightmost bar must end inside the icon — the height twin of
        // test_rows_fit_design_height, for the axis the grid actually grew on.
        // Both are const: a bad layout fails the build, not the test run.
        const LAST_BAR_END: u32 =
            WEEKLY_CELL_X + BUCKET_LABEL_WIDTH + VALUE_WIDTH + BAR_GAP_FROM_TEXT + BAR_TOTAL_WIDTH;
        const { assert!(LAST_BAR_END <= DESIGN_WIDTH) };
        // And the two cells must not overlap.
        const { assert!(SESSION_CELL_X + CELL_WIDTH <= WEEKLY_CELL_X) };
    }

    #[test]
    fn two_rows_paint_both_bands() {
        // Full bars render in COLOR_CRITICAL regardless of font availability,
        // so each 11px band must contain painted pixels — catches a row
        // landing at the wrong Y or not rendering at all.
        let icon = generate_icon(grid_rows(&[('C', 1.0, 1.0), ('K', 1.0, 1.0)]));
        let rgba = icon.rgba();
        let band_bytes = (ICON_WIDTH * LINE_HEIGHT * SCALE * 4) as usize;
        let (top, bottom) = rgba.split_at(band_bytes);
        assert!(top.iter().any(|&b| b != 0));
        assert!(bottom.iter().any(|&b| b != 0));
    }

    #[test]
    fn a_single_row_paints_the_centre_band() {
        // One provider centres vertically instead of hugging the top.
        let icon = generate_icon(grid_rows(&[('C', 1.0, 1.0)]));
        let rgba = icon.rgba();
        let (top, bottom) = band_pixels(1, 0);
        assert!(region_painted(rgba, 0, ICON_WIDTH, top, bottom));
        // Nothing above the centred band.
        assert!(!region_painted(rgba, 0, ICON_WIDTH, 0, top));
    }

    #[test]
    fn a_row_paints_both_cells() {
        // Both bars, at their own X ranges — catches a cell landing at the
        // wrong X the way two_rows_paint_both_bands catches a wrong Y.
        let icon = generate_icon(grid_rows(&[('C', 1.0, 1.0)]));
        let rgba = icon.rgba();
        let (top, bottom) = band_pixels(1, 0);
        let bar_offset = BUCKET_LABEL_WIDTH + VALUE_WIDTH + BAR_GAP_FROM_TEXT;
        for cell_x in [SESSION_CELL_X, WEEKLY_CELL_X] {
            let bar_x = (cell_x + bar_offset) * SCALE;
            assert!(region_painted(rgba, bar_x, bar_x + BAR_TOTAL_WIDTH * SCALE, top, bottom));
        }
    }

    #[test]
    fn an_empty_session_leaves_the_weekly_bar_alone() {
        // The two cells are independent: a 0% session must not blank the
        // weekly bar beside it.
        let icon = generate_icon(grid_rows(&[('C', 0.0, 1.0)]));
        let rgba = icon.rgba();
        let (top, bottom) = band_pixels(1, 0);
        let bar_x = (WEEKLY_CELL_X + BUCKET_LABEL_WIDTH + VALUE_WIDTH + BAR_GAP_FROM_TEXT) * SCALE;
        assert!(region_painted(rgba, bar_x, bar_x + BAR_TOTAL_WIDTH * SCALE, top, bottom));
    }

    /// Writes the icon out so a human can look at it. The label widths are
    /// glyph-metric guesses that only an eye can check — every assertion in
    /// this module passes on a layout whose text overlaps.
    ///
    /// `ICON_DUMP_DIR=/tmp cargo test --lib dump_icon_png -- --ignored`
    #[test]
    #[ignore]
    fn dump_icon_png() {
        let dir = std::env::var("ICON_DUMP_DIR").unwrap();
        for (name, rows) in [
            ("two", vec![('C', 0.12, 0.45, COLOR_WEEKLY), ('K', 0.30, 1.0, COLOR_KIMI)]),
            ("one", vec![('C', 0.07, 0.96, COLOR_WEEKLY)]),
        ] {
            let icon = generate_icon(rows);
            let img = RgbaImage::from_raw(ICON_WIDTH, ICON_HEIGHT, icon.rgba().to_vec()).unwrap();
            img.save(format!("{dir}/{name}.png")).unwrap();
        }
    }

    #[test]
    fn bar_color_keeps_identity_below_warning() {
        assert_eq!(bar_color(0, COLOR_SESSION), COLOR_SESSION);
        assert_eq!(bar_color(79, COLOR_SESSION), COLOR_SESSION);
        assert_eq!(bar_color(79, COLOR_WEEKLY), COLOR_WEEKLY);
    }

    #[test]
    fn bar_color_warns_at_threshold() {
        assert_eq!(bar_color(80, COLOR_SESSION), COLOR_WARNING);
        assert_eq!(bar_color(94, COLOR_WEEKLY), COLOR_WARNING);
    }

    #[test]
    fn a_nonzero_percentage_lights_a_segment() {
        assert_eq!(filled_segments(0.0), 0);
        assert_eq!(filled_segments(0.01), 1);
        assert_eq!(filled_segments(0.07), 1);
        assert_eq!(filled_segments(0.5), 3);
        assert_eq!(filled_segments(1.0), NUM_SEGMENTS);
    }

    #[test]
    fn bar_color_criticals_at_threshold() {
        assert_eq!(bar_color(95, COLOR_SESSION), COLOR_CRITICAL);
        assert_eq!(bar_color(100, COLOR_WEEKLY), COLOR_CRITICAL);
    }
}
