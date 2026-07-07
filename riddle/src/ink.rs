//! User ink: capture pen strokes, render them, dissolve them, rasterize them
//! for the oracle.

use crate::fb::BBox;
use crate::surface::{Surface, BLACK, WHITE};

/// One committed page operation, in the order the writer made it. Erases are
/// kept as ops rather than applied destructively, so `to_png` can replay the
/// writer's ink into a clean offscreen buffer — independent of whatever else
/// (a lingering reply) is painted on the screen.
enum Op {
    /// Index into `strokes`.
    Pen(usize),
    /// Eraser pass as a point list (x, y, radius).
    Erase(Vec<(i32, i32, i32)>),
}

pub struct Ink {
    /// Finished pen strokes as point lists (x, y, radius).
    strokes: Vec<Vec<(i32, i32, i32)>>,
    /// Pen and erase ops in page order.
    ops: Vec<Op>,
    current: Vec<(i32, i32, i32)>,
    current_erase: Vec<(i32, i32, i32)>,
    last_erase: Option<(i32, i32)>,
    pub bbox: BBox,
}

impl Ink {
    pub fn new() -> Self {
        Self {
            strokes: Vec::new(),
            ops: Vec::new(),
            current: Vec::new(),
            current_erase: Vec::new(),
            last_erase: None,
            bbox: BBox::empty(),
        }
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
        self.ops.clear();
        self.current.clear();
        self.current_erase.clear();
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
        self.current_erase.push((x, y, r));
        dirty
    }

    pub fn pen_up(&mut self) {
        if !self.current.is_empty() {
            self.strokes.push(std::mem::take(&mut self.current));
            self.ops.push(Op::Pen(self.strokes.len() - 1));
        }
        if !self.current_erase.is_empty() {
            self.ops.push(Op::Erase(std::mem::take(&mut self.current_erase)));
        }
        self.last_erase = None;
    }

    /// Rasterize the ink region to a grayscale PNG for the oracle.
    /// Crops to the ink bounding box and box-downscales so the long side stays
    /// ≤ 800px (at least 2x): the model reads handwriting fine at that scale,
    /// and image pixels are the dominant vision-token / latency cost.
    ///
    /// The image is built by replaying the recorded pen/erase ops into a clean
    /// offscreen buffer — NOT by reading the screen — so anything else on the
    /// page (a lingering reply the writer answers underneath) never leaks into
    /// what the oracle sees. `surf` is only consulted for the page dimensions.
    pub fn to_png(&self, surf: &Surface, path: &str) -> std::io::Result<()> {
        if self.bbox.is_empty() {
            return Err(std::io::Error::other("no ink"));
        }
        let (bx, by, bw, bh) = self.bbox.rect();
        let x0 = (bx - 20).max(0) as usize;
        let y0 = (by - 20).max(0) as usize;
        let x1 = ((bx + bw + 20) as usize).min(surf.w);
        let y1 = ((by + bh + 20) as usize).min(surf.h);
        let (cw, ch) = (x1 - x0, y1 - y0);

        // Full-resolution replay of the writer's ink on a white page crop.
        let mut page = vec![255u8; cw * ch];
        for op in &self.ops {
            match op {
                Op::Pen(i) => replay(&mut page, cw, ch, x0 as i32, y0 as i32, &self.strokes[*i], 0, true),
                Op::Erase(pts) => replay(&mut page, cw, ch, x0 as i32, y0 as i32, pts, 255, false),
            }
        }
        // In-flight strokes (normally flushed by pen-up before a commit).
        replay(&mut page, cw, ch, x0 as i32, y0 as i32, &self.current, 0, true);
        replay(&mut page, cw, ch, x0 as i32, y0 as i32, &self.current_erase, 255, false);

        let f = cw.max(ch).div_ceil(800).max(2);
        let (w, h) = (cw / f, ch / f);

        let mut gray = vec![0u8; w * h];
        for oy in 0..h {
            for ox in 0..w {
                let mut acc = 0u32;
                for sy in 0..f {
                    for sx in 0..f {
                        acc += page[(oy * f + sy) * cw + ox * f + sx] as u32;
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

/// Replay one op's point list into a grayscale crop buffer, mirroring the
/// on-screen geometry: first point stamps a disc; later points brush from the
/// previous one. Pen strokes clamp radius growth exactly like `pen_point`.
fn replay(page: &mut [u8], w: usize, h: usize, ox: i32, oy: i32, pts: &[(i32, i32, i32)], v: u8, pen: bool) {
    let mut last: Option<(i32, i32, i32)> = None;
    for &(x, y, r) in pts {
        let (cx, cy) = (x - ox, y - oy);
        match last {
            Some((px, py, pr)) => {
                let br = if pen { r.min(pr + 1) } else { r };
                brush_g(page, w, h, px, py, cx, cy, br, v);
            }
            None => stamp_g(page, w, h, cx, cy, r, v),
        }
        last = Some((cx, cy, r));
    }
}

/// `Surface::stamp` for a grayscale buffer.
fn stamp_g(page: &mut [u8], w: usize, h: usize, cx: i32, cy: i32, r: i32, v: u8) {
    for dy in -r..=r {
        for dx in -r..=r {
            if dx * dx + dy * dy <= r * r {
                let (x, y) = (cx + dx, cy + dy);
                if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                    page[y as usize * w + x as usize] = v;
                }
            }
        }
    }
}

/// `Surface::brush_line` for a grayscale buffer.
fn brush_g(page: &mut [u8], w: usize, h: usize, x0: i32, y0: i32, x1: i32, y1: i32, r: i32, v: u8) {
    let dx = (x1 - x0).abs();
    let dy = (y1 - y0).abs();
    let steps = dx.max(dy).max(1);
    for i in 0..=steps {
        let x = x0 + (x1 - x0) * i / steps;
        let y = y0 + (y1 - y0) * i / steps;
        stamp_g(page, w, h, x, y, r, v);
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
