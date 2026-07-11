//! The diary's guide: a lone, large "?" drawn on the page summons a panel of
//! the diary's gestures; touching the pen to the page dismisses it. Detection
//! is local geometry — no oracle — so the guide works even with no network.

use crate::fb::{screen_h, screen_w, BBox};
use crate::script;
use crate::surface::{Surface, BLACK, WHITE};
use ab_glyph::FontRef;

/// Does the committed ink look like a single big "?" (with or without its
/// dot)? Deliberately forgiving: a false positive only shows the guide.
pub fn looks_like_question_mark(strokes: &[Vec<(i32, i32, i32)>]) -> bool {
    if strokes.is_empty() || strokes.len() > 3 {
        return false;
    }
    let main_i = (0..strokes.len()).max_by_key(|&i| strokes[i].len()).unwrap();
    let main = &strokes[main_i];
    if main.len() < 12 {
        return false;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for &(x, y, _) in main {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    let (w, h) = (x1 - x0, y1 - y0);
    // Big, and taller than wide: a lone glyph, not a line of writing.
    if h < 280 || w < 70 || h < w {
        return false;
    }
    // Any other stroke must be the dot: small, low, roughly under the glyph.
    for (i, s) in strokes.iter().enumerate() {
        if i == main_i {
            continue;
        }
        let (mut dx0, mut dy0, mut dx1, mut dy1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for &(x, y, _) in s {
            dx0 = dx0.min(x);
            dy0 = dy0.min(y);
            dx1 = dx1.max(x);
            dy1 = dy1.max(y);
        }
        if (dx1 - dx0).max(dy1 - dy0) > 90 {
            return false;
        }
        if (dy0 + dy1) / 2 < y0 + h * 60 / 100 {
            return false;
        }
        if (dx0 + dx1) / 2 < x0 - 80 || (dx0 + dx1) / 2 > x1 + 80 {
            return false;
        }
    }
    // Normalize to top-down drawing order.
    let mut pts: Vec<(i32, i32)> = main.iter().map(|&(x, y, _)| (x, y)).collect();
    if pts[0].1 > pts[pts.len() - 1].1 {
        pts.reverse();
    }
    let start = pts[0];
    let end = pts[pts.len() - 1];
    if start.1 > y0 + h * 40 / 100 || end.1 < y0 + h * 55 / 100 {
        return false;
    }
    // The top arcs across most of the width…
    let (mut top_minx, mut top_maxx, mut top_maxx_y) = (i32::MAX, i32::MIN, 0);
    for &(x, y) in &pts {
        if y <= y0 + h * 45 / 100 {
            if x > top_maxx {
                top_maxx = x;
                top_maxx_y = y;
            }
            top_minx = top_minx.min(x);
        }
    }
    if top_maxx == i32::MIN || top_maxx - top_minx < w * 55 / 100 {
        return false;
    }
    // …and comes back DOWN on the right (rules out the flat bar of a "7").
    if top_maxx_y < y0 + h * 8 / 100 {
        return false;
    }
    // The descender stays narrow.
    let (mut bot_minx, mut bot_maxx) = (i32::MAX, i32::MIN);
    for &(x, y) in &pts {
        if y >= y0 + h * 66 / 100 {
            bot_minx = bot_minx.min(x);
            bot_maxx = bot_maxx.max(x);
        }
    }
    if bot_maxx != i32::MIN && bot_maxx - bot_minx > w * 60 / 100 {
        return false;
    }
    true
}

const TITLE: &str = "The Diary";
/// Takeover mode: riddle owns touch and the power button.
const BODY_TAKEOVER: &[&str] = &[
    "Write, then rest your quill:",
    "the diary drinks your ink and Tom replies.",
    "",
    "The diary remembers. Ask it:",
    "\"show me what I wrote about...\"",
    "and the page will rise again.",
    "",
    "Flip the marker to erase.",
    "Tap five fingers at once to leave.",
    "The power button sleeps the diary.",
    "",
    "A large ? summons this guide.",
];
/// Windowed mode: AppLoad owns the window and xochitl owns the button.
const BODY_WINDOWED: &[&str] = &[
    "Write, then rest your quill:",
    "the diary drinks your ink and Tom replies.",
    "",
    "The diary remembers. Ask it:",
    "\"show me what I wrote about...\"",
    "and the page will rise again.",
    "",
    "Flip the marker to erase.",
    "Close the diary from AppLoad.",
    "",
    "A large ? summons this guide.",
];
const FOOTER: &str = "Touch pen to page to close.";

const TITLE_PX: f32 = 88.0;
const BODY_PX: f32 = 54.0;
const FOOTER_PX: f32 = 40.0;
const PAD: usize = 64;

/// The open guide panel: remembers the pixels it covered.
pub struct Help {
    pub region: BBox,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    saved: Vec<u8>,
}

/// Draw the guide panel centered on the page; returns it for later dismissal.
/// The gesture list depends on the display mode: only takeover owns the
/// touchscreen (5-finger exit) and the power button.
pub fn show(surf: &mut Surface, font: &FontRef, takeover: bool) -> Help {
    let body = if takeover { BODY_TAKEOVER } else { BODY_WINDOWED };
    let title_h = (TITLE_PX * 1.4) as usize;
    let line_h = (BODY_PX * 1.3) as usize;
    let footer_h = (FOOTER_PX * 1.4) as usize;

    let mut wmax = script::measure(font, TITLE, TITLE_PX);
    for l in body {
        wmax = wmax.max(script::measure(font, l, BODY_PX));
    }
    let pw = (wmax as usize + 2 * PAD).min(screen_w() - 40);
    let ph = PAD + title_h + line_h / 2 + body.len() * line_h + footer_h + PAD;
    let px = (screen_w() - pw) / 2;
    let py = (screen_h().saturating_sub(ph)) / 2;

    let saved = surf.copy_rect(px, py, pw, ph);
    surf.fill_rect(px, py, pw, ph, WHITE);
    frame(surf, px, py, pw, ph, 4);
    frame(surf, px + 14, py + 14, pw - 28, ph - 28, 1);

    let mut y = py + PAD;
    blit_centered(surf, font, TITLE, TITLE_PX, px, pw, y);
    y += title_h + line_h / 2;
    for l in body {
        if !l.is_empty() {
            blit_centered(surf, font, l, BODY_PX, px, pw, y);
        }
        y += line_h;
    }
    blit_centered(surf, font, FOOTER, FOOTER_PX, px, pw, y);

    let mut region = BBox::empty();
    region.add(px as i32, py as i32, 2);
    region.add((px + pw) as i32, (py + ph) as i32, 2);
    Help { region, x: px, y: py, w: pw, h: ph, saved }
}

impl Help {
    /// Put back what the panel covered; returns the region to refresh.
    pub fn dismiss(self, surf: &mut Surface) -> BBox {
        surf.paste_rect(self.x, self.y, self.w, self.h, &self.saved);
        self.region
    }
}

/// Replace the page with the full-screen sleep card; returns the saved page
/// pixels so waking can restore them exactly.
pub fn show_sleep(surf: &mut Surface, font: &FontRef) -> Vec<u8> {
    let saved = surf.copy_rect(0, 0, screen_w(), screen_h());
    surf.fill_rect(0, 0, screen_w(), screen_h(), WHITE);
    frame(surf, 48, 48, screen_w() - 96, screen_h() - 96, 4);
    frame(surf, 66, 66, screen_w() - 132, screen_h() - 132, 1);
    let y = screen_h() * 38 / 100;
    blit_centered(surf, font, "The diary sleeps.", 116.0, 0, screen_w(), y);
    blit_centered(surf, font, "Press the button to wake it.", 56.0, 0, screen_w(), y + 230);
    saved
}

pub fn restore_sleep(surf: &mut Surface, saved: &[u8]) {
    surf.paste_rect(0, 0, screen_w(), screen_h(), saved);
}

fn frame(surf: &mut Surface, x: usize, y: usize, w: usize, h: usize, t: usize) {
    surf.fill_rect(x, y, w, t, BLACK);
    surf.fill_rect(x, y + h - t, w, t, BLACK);
    surf.fill_rect(x, y, t, h, BLACK);
    surf.fill_rect(x + w - t, y, t, h, BLACK);
}

fn blit_centered(surf: &mut Surface, font: &FontRef, text: &str, px_size: f32, panel_x: usize, panel_w: usize, y: usize) {
    let line = script::rasterize_line(font, text, px_size);
    let x = panel_x + panel_w.saturating_sub(line.width) / 2;
    for row in 0..line.height {
        for col in 0..line.width {
            if line.mask[row * line.width + col] {
                surf.put_px((x + col) as i32, (y + row) as i32, BLACK);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke(pts: &[(i32, i32)]) -> Vec<(i32, i32, i32)> {
        pts.iter().map(|&(x, y)| (x, y, 3)).collect()
    }

    /// Parametric "?": hook (arc sweeping over the top and curling back) then
    /// a straight descender; optional dot.
    fn question_mark(scale: f32, with_dot: bool, reversed: bool) -> Vec<Vec<(i32, i32, i32)>> {
        let mut pts = Vec::new();
        let (cx, cy, r) = (200.0 * scale, 180.0 * scale, 120.0 * scale);
        let mut deg = 180.0f32;
        while deg <= 450.0 {
            let a = deg.to_radians();
            pts.push(((cx + r * a.cos()) as i32, (cy + r * a.sin()) as i32));
            deg += 6.0;
        }
        let (dx, dy) = (cx as i32, (cy + r) as i32);
        for i in 1..=20 {
            pts.push((dx, dy + (i as f32 * 13.0 * scale) as i32));
        }
        if reversed {
            pts.reverse();
        }
        let mut out = vec![stroke(&pts)];
        if with_dot {
            let ddy = dy + (300.0 * scale) as i32 + 60;
            out.push(stroke(&[(dx - 5, ddy), (dx + 5, ddy + 5), (dx, ddy + 8)]));
        }
        out
    }

    #[test]
    fn detects_question_marks() {
        assert!(looks_like_question_mark(&question_mark(1.5, true, false)));
        assert!(looks_like_question_mark(&question_mark(1.5, false, false)));
        assert!(looks_like_question_mark(&question_mark(1.5, true, true)));
        assert!(looks_like_question_mark(&question_mark(3.0, true, false)));
    }

    #[test]
    fn rejects_non_question_marks() {
        // Too small (normal end-of-sentence "?").
        assert!(!looks_like_question_mark(&question_mark(0.5, true, false)));
        // "!" — vertical bar plus dot.
        let bar: Vec<(i32, i32)> = (0..40).map(|i| (200, 60 + i * 12)).collect();
        assert!(!looks_like_question_mark(&[stroke(&bar), stroke(&[(200, 600), (204, 604)])]));
        // "7" — flat top bar, diagonal descender.
        let mut seven: Vec<(i32, i32)> = (0..20).map(|i| (80 + i * 12, 60)).collect();
        seven.extend((0..40).map(|i| (320 - i * 4, 60 + i * 12)));
        assert!(!looks_like_question_mark(&[stroke(&seven)]));
        // Two long strokes side by side (writing, not a glyph).
        let l1: Vec<(i32, i32)> = (0..40).map(|i| (100, 60 + i * 10)).collect();
        let l2: Vec<(i32, i32)> = (0..40).map(|i| (400, 60 + i * 10)).collect();
        assert!(!looks_like_question_mark(&[stroke(&l1), stroke(&l2)]));
        // Empty / too many strokes.
        assert!(!looks_like_question_mark(&[]));
        let dot = stroke(&[(0, 0), (1, 1)]);
        assert!(!looks_like_question_mark(&[dot.clone(), dot.clone(), dot.clone(), dot]));
    }

    #[test]
    fn modal_renders_and_restores() {
        crate::fb::test_init_screen();
        let (w, h) = (screen_w(), screen_h());
        let mut buf = vec![0xFFu8; w * h * 4];
        let ptr = buf.as_mut_ptr();
        let mut surf = Surface::new(ptr, buf.len(), w, h, w * 4, crate::surface::PixFmt::Rgb32);
        let font = FontRef::try_from_slice(include_bytes!("../fonts/DancingScript.ttf")).unwrap();

        // Scribble something under the panel area so restore is observable.
        surf.fill_rect(700, 1000, 200, 200, BLACK);
        let before = surf.copy_rect(0, 0, w, h);

        let panel = show(&mut surf, &font, true);
        let (px, py, pw, ph) = panel.region.rect();
        assert!(pw > 400 && ph > 400, "panel too small: {pw}x{ph}");
        // Panel must contain ink (text + frame).
        let mut black = 0;
        for y in py..py + ph {
            for x in px..px + pw {
                if surf.luma(x, y) < 128 {
                    black += 1;
                }
            }
        }
        assert!(black > 5000, "panel looks empty: {black} dark px");

        // Dump for visual inspection.
        let out = std::env::temp_dir().join("riddle-help-modal.png");
        let mut gray = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                gray[y * w + x] = surf.luma(x as i32, y as i32);
            }
        }
        let file = std::fs::File::create(&out).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&gray).unwrap();
        eprintln!("modal snapshot: {}", out.display());

        // Dismissing must restore the page byte-for-byte.
        panel.dismiss(&mut surf);
        assert_eq!(before, surf.copy_rect(0, 0, w, h), "restore is not exact");
    }

    #[test]
    fn sleep_page_renders_and_restores() {
        crate::fb::test_init_screen();
        let (w, h) = (screen_w(), screen_h());
        let mut buf = vec![0xFFu8; w * h * 4];
        let ptr = buf.as_mut_ptr();
        let mut surf = Surface::new(ptr, buf.len(), w, h, w * 4, crate::surface::PixFmt::Rgb32);
        let font = FontRef::try_from_slice(include_bytes!("../fonts/DancingScript.ttf")).unwrap();

        surf.fill_rect(300, 300, 400, 400, BLACK);
        let before = surf.copy_rect(0, 0, w, h);

        let saved = show_sleep(&mut surf, &font);
        let mut black = 0usize;
        for y in 0..h {
            for x in 0..w {
                if surf.luma(x as i32, y as i32) < 128 {
                    black += 1;
                }
            }
        }
        assert!(black > 10_000, "sleep page looks empty: {black} dark px");

        let out = std::env::temp_dir().join("riddle-sleep-page.png");
        let mut gray = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                gray[y * w + x] = surf.luma(x as i32, y as i32);
            }
        }
        let file = std::fs::File::create(&out).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&gray).unwrap();
        eprintln!("sleep snapshot: {}", out.display());

        restore_sleep(&mut surf, &saved);
        assert_eq!(before, surf.copy_rect(0, 0, w, h), "sleep restore is not exact");
    }
}
