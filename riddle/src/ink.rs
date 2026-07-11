//! User ink: capture pen strokes, render them, dissolve them, rasterize them
//! for the oracle.

use crate::fb::BBox;
use crate::surface::{Surface, BLACK, WHITE};

pub struct Ink {
    /// Finished strokes as point lists (x, y, radius).
    strokes: Vec<Vec<(i32, i32, i32)>>,
    current: Vec<(i32, i32, i32)>,
    last_erase: Option<(i32, i32)>,
    pub bbox: BBox,
}

impl Ink {
    pub fn new() -> Self {
        Self { strokes: Vec::new(), current: Vec::new(), last_erase: None, bbox: BBox::empty() }
    }

    pub fn is_empty(&self) -> bool {
        self.strokes.is_empty() && self.current.is_empty()
    }

    /// Finished strokes (the current in-flight stroke is not included).
    pub fn stroke_list(&self) -> &[Vec<(i32, i32, i32)>] {
        &self.strokes
    }

    pub fn clear(&mut self) {
        self.strokes.clear();
        self.current.clear();
        self.last_erase = None;
        self.bbox = BBox::empty();
    }

    /// Pen touched down or moved while down, with brush radius already
    /// resolved by the caller. Returns the dirty rect of what was drawn.
    pub fn pen_point(&mut self, surf: &mut Surface, x: i32, y: i32, r: i32) -> BBox {
        let mut dirty = BBox::empty();
        if let Some(&(px, py, pr)) = self.current.last() {
            surf.brush_line(px, py, x, y, r.min(pr + 1), BLACK);
            dirty.add(px, py, pr + 2);
        } else {
            surf.stamp(x, y, r, BLACK);
        }
        dirty.add(x, y, r + 2);
        self.current.push((x, y, r));
        self.bbox.add(x, y, r + 2);
        dirty
    }

    /// Eraser tip: brush white over the page AND drop the stored points it
    /// covers, so the stroke model stays true to the visible page. Without
    /// this, erased ink would still be remembered and re-conjured, and an
    /// erased "?" would still summon the guide.
    pub fn erase_point(&mut self, surf: &mut Surface, x: i32, y: i32, r: i32) -> BBox {
        let mut dirty = BBox::empty();
        if let Some((px, py)) = self.last_erase {
            surf.brush_line(px, py, x, y, r, WHITE);
            dirty.add(px, py, r + 2);
        } else {
            surf.stamp(x, y, r, WHITE);
        }
        dirty.add(x, y, r + 2);
        self.forget_near(x, y, r);
        self.last_erase = Some((x, y));
        dirty
    }

    /// Remove committed stroke points within `r` of (x, y); split strokes that
    /// are erased through the middle, and recompute the ink bbox.
    fn forget_near(&mut self, x: i32, y: i32, r: i32) {
        let r2 = (r + 2) * (r + 2);
        let mut kept: Vec<Vec<(i32, i32, i32)>> = Vec::new();
        for stroke in self.strokes.drain(..) {
            let mut seg: Vec<(i32, i32, i32)> = Vec::new();
            for p in stroke {
                let (dx, dy) = (p.0 - x, p.1 - y);
                if dx * dx + dy * dy <= r2 {
                    if !seg.is_empty() {
                        kept.push(std::mem::take(&mut seg));
                    }
                } else {
                    seg.push(p);
                }
            }
            if !seg.is_empty() {
                kept.push(seg);
            }
        }
        self.strokes = kept;
        self.bbox = BBox::empty();
        for stroke in &self.strokes {
            for &(px, py, pr) in stroke {
                self.bbox.add(px, py, pr + 2);
            }
        }
    }

    pub fn pen_up(&mut self) {
        if !self.current.is_empty() {
            self.strokes.push(std::mem::take(&mut self.current));
        }
        self.last_erase = None;
    }

    /// Rasterize the ink region to a grayscale PNG for the oracle.
    /// Crops to the ink bounding box and box-downscales so the long side stays
    /// ≤ 800px (at least 2x): the model reads handwriting fine at that scale,
    /// and image pixels are the dominant vision-token / latency cost.
    pub fn to_png(&self, surf: &Surface, path: &str) -> std::io::Result<()> {
        if self.bbox.is_empty() {
            return Err(std::io::Error::other("no ink"));
        }
        let (bx, by, bw, bh) = self.bbox.rect();
        let x0 = (bx - 20).max(0) as usize;
        let y0 = (by - 20).max(0) as usize;
        let x1 = ((bx + bw + 20) as usize).min(surf.w);
        let y1 = ((by + bh + 20) as usize).min(surf.h);
        let f = ((x1 - x0).max(y1 - y0)).div_ceil(800).max(2);
        let (w, h) = ((x1 - x0) / f, (y1 - y0) / f);

        let mut gray = vec![0u8; w * h];
        for oy in 0..h {
            for ox in 0..w {
                let mut acc = 0u32;
                for sy in 0..f {
                    for sx in 0..f {
                        acc += surf.luma((x0 + ox * f + sx) as i32, (y0 + oy * f + sy) as i32) as u32;
                    }
                }
                gray[oy * w + ox] = (acc / (f * f) as u32) as u8;
            }
        }

        let file = std::fs::File::create(path)?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        // Fast deflate: encode time matters more than a few KB on the tablet.
        enc.set_compression(png::Compression::Fast);
        let mut writer = enc.write_header().map_err(std::io::Error::other)?;
        writer
            .write_image_data(&gray)
            .map_err(std::io::Error::other)?;
        Ok(())
    }
}

/// Deterministic per-pixel hash for the dissolve pattern.
#[inline]
fn px_hash(x: i32, y: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x9E3779B1) ^ (y as u32).wrapping_mul(0x85EBCA6B);
    h ^= h >> 13;
    h = h.wrapping_mul(0xC2B2AE35);
    h ^ (h >> 16)
}

/// One pass of the "diary drinks the ink" effect: erase the pixels whose hash
/// falls in this stage. After `stages` passes the region is clean white.
pub fn dissolve_pass(surf: &mut Surface, region: BBox, stage: u32, stages: u32) {
    if region.is_empty() {
        return;
    }
    for y in region.y0..=region.y1 {
        for x in region.x0..=region.x1 {
            if surf.luma(x, y) < 250 && px_hash(x, y) % stages <= stage {
                surf.put_px(x, y, WHITE);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::PixFmt;

    fn surf() -> (Vec<u8>, Surface) {
        let mut buf = vec![0xFFu8; 400 * 400 * 4];
        let ptr = buf.as_mut_ptr();
        let s = Surface::new(ptr, buf.len(), 400, 400, 400 * 4, PixFmt::Rgb32);
        (buf, s)
    }

    #[test]
    fn erase_forgets_covered_points_and_splits_strokes() {
        crate::fb::test_init_screen();
        let (_buf, mut s) = surf();
        let mut ink = Ink::new();
        // A horizontal stroke across the page.
        for x in (20..=200).step_by(10) {
            ink.pen_point(&mut s, x, 100, 3);
        }
        ink.pen_up();
        assert_eq!(ink.stroke_list().len(), 1);
        let before: usize = ink.stroke_list().iter().map(|s| s.len()).sum();

        // Erase through the middle: the stroke splits, points vanish.
        ink.erase_point(&mut s, 110, 100, 20);
        let after: usize = ink.stroke_list().iter().map(|s| s.len()).sum();
        assert!(after < before, "erase kept every point ({after} of {before})");
        assert_eq!(ink.stroke_list().len(), 2, "middle-erase should split the stroke");
        // No surviving point lies under the eraser.
        for st in ink.stroke_list() {
            for &(x, y, _) in st {
                assert!((x - 110).pow(2) + (y - 100).pow(2) > 22 * 22);
            }
        }
    }

    #[test]
    fn erasing_everything_empties_the_ink() {
        crate::fb::test_init_screen();
        let (_buf, mut s) = surf();
        let mut ink = Ink::new();
        ink.pen_point(&mut s, 100, 100, 3);
        ink.pen_point(&mut s, 104, 100, 3);
        ink.pen_up();
        assert!(!ink.is_empty());
        ink.erase_point(&mut s, 102, 100, 30);
        assert!(ink.stroke_list().is_empty());
        assert!(ink.bbox.is_empty());
    }
}
