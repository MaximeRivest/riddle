//! Geometry helpers. Drawing lives in surface.rs.

use std::sync::OnceLock;

// Panel size varies by device (Paper Pro: 1620x2160, Paper Pro Move:
// 960x1696) and isn't known until the display backend opens and reports
// the real aux/qtfb framebuffer geometry. `init_screen` sets it once, early
// in `main`; every reader after that goes through `screen_w`/`screen_h`.
static SCREEN_DIMS: OnceLock<(usize, usize)> = OnceLock::new();

pub fn init_screen(w: usize, h: usize) {
    let _ = SCREEN_DIMS.set((w, h));
}

pub fn screen_w() -> usize {
    SCREEN_DIMS.get().expect("init_screen not called before screen_w").0
}

pub fn screen_h() -> usize {
    SCREEN_DIMS.get().expect("init_screen not called before screen_h").1
}

/// Tests exercise geometry helpers without ever opening a display, so they
/// need `init_screen` called explicitly. Any dimensions work for a test;
/// this uses the Paper Pro's so tests match the values they were written
/// against. Idempotent (and safe under parallel test execution) since it's
/// the same `OnceLock::set` `init_screen` itself uses.
#[cfg(test)]
pub fn test_init_screen() {
    init_screen(1620, 2160);
}

/// Grow-only pixel bounding box, used to build update/dissolve regions.
#[derive(Clone, Copy, Debug)]
pub struct BBox {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl BBox {
    pub fn empty() -> Self {
        Self { x0: i32::MAX, y0: i32::MAX, x1: i32::MIN, y1: i32::MIN }
    }
    pub fn is_empty(&self) -> bool {
        self.x0 > self.x1
    }
    pub fn add(&mut self, x: i32, y: i32, margin: i32) {
        self.x0 = self.x0.min(x - margin).max(0);
        self.y0 = self.y0.min(y - margin).max(0);
        self.x1 = self.x1.max(x + margin).min(screen_w() as i32 - 1);
        self.y1 = self.y1.max(y + margin).min(screen_h() as i32 - 1);
    }
    pub fn rect(&self) -> (i32, i32, i32, i32) {
        (self.x0, self.y0, self.x1 - self.x0 + 1, self.y1 - self.y0 + 1)
    }
}
