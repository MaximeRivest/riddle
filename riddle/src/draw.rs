//! Tom's drawing hand. The oracle may answer with a sketch — a ⟦draw:…⟧
//! block of pen strokes on an abstract 100×100 canvas — and this module turns
//! that block into screen-space strokes the reply animator can ink.
//!
//! Two halves:
//!   * [`parse`] — the block's payload into canvas strokes (forgiving:
//!     malformed points are skipped, coordinates clamped to the canvas).
//!   * [`place`] — canvas strokes onto the page: uniform scale, centered,
//!     below the prose written so far, subdivided into pen-sized steps with a
//!     slow wobble so the lines read as drawn by a hand, not plotted.

use crate::fb::{SCREEN_H, SCREEN_W};

/// Strokes on the model's 0–100 canvas (0,0 = top left).
pub type Sketch = Vec<Vec<(f32, f32)>>;

/// The abstract canvas is 100×100.
const CANVAS: f32 = 100.0;
/// The canvas never maps larger than this many pixels per side.
const MAX_SIDE: f32 = 900.0;
/// Least vertical room worth drawing in; below this the sketch is skipped.
const MIN_ROOM: i32 = 220;
/// Side margins, matching the reply text margins.
const MARGIN_X: i32 = 120;
/// Bottom of the drawable page.
const FLOOR: i32 = SCREEN_H as i32 - 140;
/// Ink is laid down in steps about this long (px) — the animator draws
/// point-to-point, so steps set both smoothness and writing speed.
const STEP: f32 = 2.5;

/// Parse a draw block's inner text ("draw: x,y x,y; x,y …") into canvas
/// strokes. Strokes are ';'-separated, points whitespace-separated. A lone
/// point is a dot. Returns None when nothing drawable survives.
pub fn parse(inner: &str) -> Option<Sketch> {
    let body = inner.trim();
    let body = strip_prefix_ci(body, "draw")?;
    let body = body.trim_start_matches([':', ' ', '\n', '\r', '\t']);
    let mut sketch = Vec::new();
    for stroke_text in body.split(';') {
        let mut stroke = Vec::new();
        for pt in stroke_text.split_whitespace() {
            let Some((x, y)) = pt.split_once(',') else { continue };
            let (Ok(x), Ok(y)) = (x.trim().parse::<f32>(), y.trim().parse::<f32>()) else {
                continue;
            };
            if !x.is_finite() || !y.is_finite() {
                continue;
            }
            stroke.push((x.clamp(0.0, CANVAS), y.clamp(0.0, CANVAS)));
        }
        if !stroke.is_empty() {
            sketch.push(stroke);
        }
    }
    if sketch.is_empty() { None } else { Some(sketch) }
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

/// Place a sketch on the page: its canvas bounding box is scaled uniformly
/// (capped by the page margins and MAX_SIDE), centered horizontally, and laid
/// with its top at `y_top`. Returns the screen strokes plus the bottom edge of
/// the ink (where prose may continue), or None when the page has no room.
pub fn place(sketch: &Sketch, y_top: i32, seed: u32) -> Option<(Vec<Vec<(i32, i32)>>, i32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for s in sketch {
        for &(x, y) in s {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    if x0 > x1 {
        return None;
    }
    let room = FLOOR - y_top;
    if room < MIN_ROOM {
        return None;
    }
    // Uniform scale: the full canvas maps to at most MAX_SIDE / page width;
    // the sketch's own height must also fit the room below the prose.
    let bw = (x1 - x0).max(1.0);
    let bh = (y1 - y0).max(1.0);
    let mut scale = (MAX_SIDE / CANVAS).min((SCREEN_W as i32 - 2 * MARGIN_X) as f32 / CANVAS);
    scale = scale.min(room as f32 / bh);
    let ox = (SCREEN_W as f32 - bw * scale) / 2.0 - x0 * scale;
    let oy = y_top as f32 - y0 * scale;

    let mut out = Vec::with_capacity(sketch.len());
    let mut bottom = y_top;
    for (si, stroke) in sketch.iter().enumerate() {
        let h = hash(seed, si as u32);
        let phase = (h % 628) as f32 / 100.0;
        let placed = wobble_stroke(stroke, scale, ox, oy, phase, h);
        for &(_, y) in &placed {
            bottom = bottom.max(y);
        }
        if !placed.is_empty() {
            out.push(placed);
        }
    }
    if out.is_empty() { None } else { Some((out, bottom)) }
}

/// Map one canvas stroke to the screen, subdividing every segment into
/// STEP-sized pieces displaced sideways by a slow sine — the small unsteadiness
/// of a real hand. Deterministic for a given phase/seed.
fn wobble_stroke(
    stroke: &[(f32, f32)],
    scale: f32,
    ox: f32,
    oy: f32,
    phase: f32,
    seed: u32,
) -> Vec<(i32, i32)> {
    const WAVELENGTH: f32 = 70.0;
    const AMP: f32 = 1.6;
    let jitter = |i: u32| (hash(seed, 0x0DD + i) % 300) as f32 / 100.0 - 1.5;

    let pts: Vec<(f32, f32)> = stroke
        .iter()
        .enumerate()
        .map(|(i, &(x, y))| (x * scale + ox + jitter(i as u32 * 2), y * scale + oy + jitter(i as u32 * 2 + 1)))
        .collect();
    let mut out: Vec<(i32, i32)> = Vec::new();
    let mut push = |x: f32, y: f32| {
        let p = (x.round() as i32, y.round() as i32);
        if out.last() != Some(&p) {
            out.push(p);
        }
    };
    if pts.len() == 1 {
        push(pts[0].0, pts[0].1);
        return out;
    }
    let mut dist = 0.0f32; // arc length so the wobble is continuous across segments
    for w in pts.windows(2) {
        let ((ax, ay), (bx, by)) = (w[0], w[1]);
        let (dx, dy) = (bx - ax, by - ay);
        let len = (dx * dx + dy * dy).sqrt();
        let steps = (len / STEP).ceil().max(1.0) as u32;
        // Unit normal for the sideways wobble.
        let (nx, ny) = if len > 0.0 { (-dy / len, dx / len) } else { (0.0, 0.0) };
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let along = dist + t * len;
            // The wobble dies at the stroke's own vertices so corners land
            // where the model put them.
            let envelope = (t * (1.0 - t) * 4.0).min(1.0);
            let w = (along / WAVELENGTH * std::f32::consts::TAU + phase).sin() * AMP * envelope;
            push(ax + dx * t + nx * w, ay + dy * t + ny * w);
        }
        dist += len;
    }
    out
}

/// A deterministic per-sketch seed: the same drawing always wobbles the same.
pub fn sketch_seed(sketch: &Sketch) -> u32 {
    sketch
        .iter()
        .flatten()
        .fold(0x51D2u32, |h, &(x, y)| hash(h, ((x * 7.0) as u32) ^ (((y * 13.0) as u32) << 8)))
}

fn hash(seed: u32, i: u32) -> u32 {
    let mut h = seed.wrapping_add(i.wrapping_mul(0x9E37_79B1));
    h ^= h >> 15;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^ (h >> 13)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_strokes_and_dots() {
        let s = parse("draw: 0,0 100,100; 50,50").unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0], vec![(0.0, 0.0), (100.0, 100.0)]);
        assert_eq!(s[1], vec![(50.0, 50.0)]);
    }

    #[test]
    fn parse_tolerates_case_newlines_and_junk() {
        let s = parse("DRAW\n 10,20 nonsense 30,40 ;; 5,5").unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0], vec![(10.0, 20.0), (30.0, 40.0)]);
    }

    #[test]
    fn parse_clamps_to_canvas() {
        let s = parse("draw: -50,300").unwrap();
        assert_eq!(s[0], vec![(0.0, 100.0)]);
    }

    #[test]
    fn parse_rejects_empty_and_alien_blocks() {
        assert!(parse("draw:").is_none());
        assert!(parse("draw: x,y ;").is_none());
        assert!(parse("show:3").is_none());
    }

    #[test]
    fn place_scales_centers_and_stays_on_page() {
        let sketch: Sketch = vec![vec![(0.0, 0.0), (100.0, 100.0)]];
        let (strokes, bottom) = place(&sketch, 300, 7).unwrap();
        let all: Vec<(i32, i32)> = strokes.concat();
        let (min_x, max_x) = (
            all.iter().map(|p| p.0).min().unwrap(),
            all.iter().map(|p| p.0).max().unwrap(),
        );
        let min_y = all.iter().map(|p| p.1).min().unwrap();
        assert!(min_x >= MARGIN_X - 4, "left edge {min_x}");
        assert!(max_x <= SCREEN_W as i32 - MARGIN_X + 4, "right edge {max_x}");
        assert!((min_y - 300).abs() <= 4, "top {min_y}");
        assert!(bottom > 300 + 400, "diagonal should be tall, bottom {bottom}");
        assert!(bottom <= FLOOR + 4);
        // Centered: midpoint of the span sits near the page middle.
        assert!(((min_x + max_x) / 2 - SCREEN_W as i32 / 2).abs() < 20);
    }

    #[test]
    fn place_subdivides_into_pen_steps() {
        let sketch: Sketch = vec![vec![(0.0, 50.0), (100.0, 50.0)]];
        let (strokes, _) = place(&sketch, 300, 7).unwrap();
        // A full-width line is ~1380px: at ~2.5px steps that is many points.
        assert!(strokes[0].len() > 300, "only {} points", strokes[0].len());
    }

    #[test]
    fn place_refuses_a_full_page() {
        let sketch: Sketch = vec![vec![(0.0, 0.0), (100.0, 100.0)]];
        assert!(place(&sketch, SCREEN_H as i32 - 150, 7).is_none());
    }

    #[test]
    fn place_fits_height_to_remaining_room() {
        let sketch: Sketch = vec![vec![(0.0, 0.0), (100.0, 100.0)]];
        let y_top = SCREEN_H as i32 - 140 - 300; // exactly 300px of room
        let (_, bottom) = place(&sketch, y_top, 7).unwrap();
        assert!(bottom <= FLOOR + 4, "bottom {bottom} spills past the page");
    }

    #[test]
    fn place_is_deterministic() {
        let sketch: Sketch = vec![vec![(10.0, 10.0), (90.0, 40.0), (20.0, 80.0)]];
        assert_eq!(place(&sketch, 400, 42), place(&sketch, 400, 42));
        assert_ne!(place(&sketch, 400, 42), place(&sketch, 400, 43));
    }
}
