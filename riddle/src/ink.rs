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

    /// Remove the most recent finished stroke (an absorbed gesture) and return
    /// its bounding box. The ink bbox is rebuilt from what remains.
    pub fn pop_stroke(&mut self) -> Option<BBox> {
        let s = self.strokes.pop()?;
        let mut gone = BBox::empty();
        for &(x, y, r) in &s {
            gone.add(x, y, r + 2);
        }
        self.bbox = BBox::empty();
        for st in &self.strokes {
            for &(x, y, r) in st {
                self.bbox.add(x, y, r + 2);
            }
        }
        for &(x, y, r) in &self.current {
            self.bbox.add(x, y, r + 2);
        }
        Some(gone)
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

    /// Eraser tip: brush white over the page.
    pub fn erase_point(&mut self, surf: &mut Surface, x: i32, y: i32, r: i32) -> BBox {
        let mut dirty = BBox::empty();
        if let Some((px, py)) = self.last_erase {
            surf.brush_line(px, py, x, y, r, WHITE);
            dirty.add(px, py, r + 2);
        } else {
            surf.stamp(x, y, r, WHITE);
        }
        dirty.add(x, y, r + 2);
        self.last_erase = Some((x, y));
        dirty
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
