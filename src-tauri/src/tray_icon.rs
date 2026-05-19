use ab_glyph::FontArc;
use image::{Rgba, RgbaImage};
use imageproc::drawing::{draw_filled_rect_mut, draw_text_mut};
use imageproc::rect::Rect;
use std::sync::LazyLock;
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
const LINE2_Y: u32 = LINE1_Y + LINE_HEIGHT + LINE_GAP;

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
const COLOR_SESSION: Rgba<u8> = Rgba([107, 127, 224, 255]);
const COLOR_WEEKLY: Rgba<u8> = Rgba([192, 96, 208, 255]);
const COLOR_SEGMENT_OFF: Rgba<u8> = Rgba([140, 140, 140, 80]);
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

/// Generates a dynamic tray icon with segmented progress bars
pub fn generate_icon(session: f64, weekly: f64) -> Image<'static> {
    let font = &*FONT;
    let mut img = RgbaImage::new(ICON_WIDTH, ICON_HEIGHT);
    let text_color = if menubar_is_dark() { COLOR_TEXT_DARK_BG } else { COLOR_TEXT_LIGHT_BG };

    draw_line(&mut img, font, LINE1_Y, session, 'S', COLOR_SESSION, text_color);
    draw_line(&mut img, font, LINE2_Y, weekly, 'W', COLOR_WEEKLY, text_color);

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

    for i in 0..NUM_SEGMENTS {
        let seg_x = (BAR_X + i * (SEGMENT_WIDTH + SEGMENT_GAP)) * SCALE;
        let seg_y = bar_y * SCALE;
        let seg_color = if i < filled_segments { color } else { COLOR_SEGMENT_OFF };

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

    #[test]
    fn test_generate_icon_dimensions() {
        let icon = generate_icon(0.5, 0.3);
        assert_eq!(icon.width(), ICON_WIDTH);
        assert_eq!(icon.height(), ICON_HEIGHT);
    }

    #[test]
    fn test_generate_icon_not_empty() {
        let icon = generate_icon(0.5, 0.3);
        let rgba = icon.rgba();
        assert!(rgba.iter().any(|&b| b != 0));
    }

    #[test]
    fn test_generate_icon_zero() {
        let _icon = generate_icon(0.0, 0.0);
    }

    #[test]
    fn test_generate_icon_full() {
        let _icon = generate_icon(1.0, 1.0);
    }

    #[test]
    fn test_generate_icon_over_range() {
        let _icon = generate_icon(1.5, -0.5);
    }

    #[test]
    fn test_design_fits_height() {
        assert!(LINE2_Y + LINE_HEIGHT <= DESIGN_HEIGHT);
    }
}
