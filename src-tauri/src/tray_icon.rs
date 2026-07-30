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
/// Rows that fit the design height: S/W for a single provider, or one weekly
/// row per provider when several are active.
const MAX_ROWS: usize = 2;

// Bar layout
const NUM_SEGMENTS: u32 = 10;
const SEGMENT_WIDTH: u32 = 4;
const SEGMENT_GAP: u32 = 1;
const SEGMENT_HEIGHT: u32 = 10;
const BAR_GAP_FROM_TEXT: u32 = 1;

// Text metrics
const CHAR_ADVANCE: u32 = 8; // approximate advance per character for SF Pro at this size
const VALUE_X_OFFSET: u32 = 18; // fixed X for value text, accommodates widest label "W:"
const VALUE_CHARS: u32 = 4; // "06%" to "100%"
const MAX_TEXT_WIDTH: u32 = VALUE_X_OFFSET + VALUE_CHARS * CHAR_ADVANCE;
const BAR_X: u32 = 1 + MAX_TEXT_WIDTH + BAR_GAP_FROM_TEXT;
const BAR_TOTAL_WIDTH: u32 = NUM_SEGMENTS * (SEGMENT_WIDTH + SEGMENT_GAP) - SEGMENT_GAP;
const DESIGN_WIDTH: u32 = BAR_X + BAR_TOTAL_WIDTH + 1;

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

/// Generates a dynamic tray icon with segmented progress bars, one row per
/// `(label, percent 0.0..=1.0, identity color)` entry, top to bottom.
/// At most MAX_ROWS fit the 22px design height; callers never pass more.
pub fn generate_icon(rows: Vec<(char, f64, Rgba<u8>)>) -> Image<'static> {
    let font = &*FONT;
    let mut img = RgbaImage::new(ICON_WIDTH, ICON_HEIGHT);
    let text_color = if menubar_is_dark_cached() { COLOR_TEXT_DARK_BG } else { COLOR_TEXT_LIGHT_BG };

    debug_assert!(rows.len() <= MAX_ROWS);
    for (i, &(label, pct, color)) in rows.iter().enumerate() {
        let line_y = LINE1_Y + i as u32 * (LINE_HEIGHT + LINE_GAP);
        draw_line(&mut img, font, line_y, pct, label, color, text_color);
    }

    Image::new_owned(img.into_raw(), ICON_WIDTH, ICON_HEIGHT)
}

fn draw_line(
    img: &mut RgbaImage,
    font: &FontArc,
    line_y: u32,
    pct: f64,
    label: char,
    color: Rgba<u8>,
    text_color: Rgba<u8>,
) {
    let clamped = pct.clamp(0.0, 1.0);
    let pct_int = (clamped * 100.0).round() as u32;
    let label_str = format!("{}:", label);
    let value_str = format!("{:02}%", pct_int);

    // Draw label and value separately so percentages align vertically
    let text_x = 1 * SCALE as i32;
    let value_x = text_x + (VALUE_X_OFFSET * SCALE) as i32;
    let text_y = (line_y * SCALE) as i32 + TEXT_Y_OFFSET;

    // Pseudo-bold: draw twice with 1px horizontal offset
    draw_text_mut(img, text_color, text_x, text_y, FONT_SIZE, font, &label_str);
    draw_text_mut(img, text_color, text_x + 1, text_y, FONT_SIZE, font, &label_str);
    draw_text_mut(img, text_color, value_x, text_y, FONT_SIZE, font, &value_str);
    draw_text_mut(img, text_color, value_x + 1, text_y, FONT_SIZE, font, &value_str);

    // Draw segmented bar
    let bar_y = line_y + (LINE_HEIGHT - SEGMENT_HEIGHT) / 2;
    let filled_segments = ((NUM_SEGMENTS as f64) * clamped).round() as u32;

    let filled_color = bar_color(pct_int, color);

    for i in 0..NUM_SEGMENTS {
        let seg_x = (BAR_X + i * (SEGMENT_WIDTH + SEGMENT_GAP)) * SCALE;
        let seg_y = bar_y * SCALE;
        let seg_color = if i < filled_segments { filled_color } else { COLOR_SEGMENT_OFF };

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

    /// The single-provider layout: identical input to what the pre-Vec
    /// `generate_icon(session, weekly)` drew.
    fn sw_rows(session: f64, weekly: f64) -> Vec<(char, f64, Rgba<u8>)> {
        vec![
            ('S', session, COLOR_SESSION),
            ('W', weekly, COLOR_WEEKLY),
        ]
    }

    #[test]
    fn test_generate_icon_dimensions() {
        let icon = generate_icon(sw_rows(0.5, 0.3));
        assert_eq!(icon.width(), ICON_WIDTH);
        assert_eq!(icon.height(), ICON_HEIGHT);
    }

    #[test]
    fn test_generate_icon_not_empty() {
        let icon = generate_icon(sw_rows(0.5, 0.3));
        let rgba = icon.rgba();
        assert!(rgba.iter().any(|&b| b != 0));
    }

    #[test]
    fn test_generate_icon_zero() {
        let _icon = generate_icon(sw_rows(0.0, 0.0));
    }

    #[test]
    fn test_generate_icon_full() {
        let _icon = generate_icon(sw_rows(1.0, 1.0));
    }

    #[test]
    fn test_generate_icon_over_range() {
        let _icon = generate_icon(sw_rows(1.5, -0.5));
    }

    #[test]
    fn test_rows_fit_design_height() {
        for i in 0..MAX_ROWS {
            let bottom = LINE1_Y + i as u32 * (LINE_HEIGHT + LINE_GAP) + LINE_HEIGHT;
            assert!(bottom <= DESIGN_HEIGHT);
        }
    }

    #[test]
    fn two_rows_paint_both_bands() {
        // Full bars render in COLOR_CRITICAL regardless of font availability,
        // so each 11px band must contain painted pixels — catches a row
        // landing at the wrong Y or not rendering at all.
        let icon = generate_icon(sw_rows(1.0, 1.0));
        let rgba = icon.rgba();
        let band_bytes = (ICON_WIDTH * LINE_HEIGHT * SCALE * 4) as usize;
        let (top, bottom) = rgba.split_at(band_bytes);
        assert!(top.iter().any(|&b| b != 0));
        assert!(bottom.iter().any(|&b| b != 0));
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
    fn bar_color_criticals_at_threshold() {
        assert_eq!(bar_color(95, COLOR_SESSION), COLOR_CRITICAL);
        assert_eq!(bar_color(100, COLOR_WEEKLY), COLOR_CRITICAL);
    }
}
